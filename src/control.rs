//! External control bus: lets scripts, agents and other processes start, stop,
//! restart and query components of a running ProConductor instance.
//!
//! Mechanism (dependency-free, works on Linux/macOS/Windows):
//!   * Next to `foo.json` the app owns a directory `foo.json.ctl/`.
//!   * A client drops `<anything>.cmd` into it (write to `.tmp`, then rename so
//!     the app never reads a half-written file). Content is either JSON
//!     `{"action":"restart","target":"MCP Server"}` or one plain-text line
//!     `restart MCP Server`.
//!   * The app watches the directory, executes the command, deletes the `.cmd`
//!     and writes `<anything>.result` containing `{"ok":bool,"message":...}`.
//!     For `stop`/`restart` the result is written only when the process is
//!     really down / really back up, so a client can block on it.
//!   * `proconductor foo.json --restart "MCP Server"` does exactly that from
//!     the CLI and exits 0/1 — the intended entry point for automation.
//!
//! Target resolution: component id, component name (case-insensitive), group
//! id, group name, or `all`.

use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::app::{lock_app, ProConductor};
use crate::config::AppConfig;
use crate::process::resolve_path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action { Start, Stop, Restart, Status }

impl Action {
    pub fn parse(s: &str) -> Option<Action> {
        match s.trim().to_ascii_lowercase().as_str() {
            "start"   => Some(Action::Start),
            "stop"    => Some(Action::Stop),
            "restart" => Some(Action::Restart),
            "status"  => Some(Action::Status),
            _         => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Action::Start => "start", Action::Stop => "stop",
            Action::Restart => "restart", Action::Status => "status",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ControlCommand {
    /// Path of the `.cmd` file; the `.result` is written next to it.
    pub path:   PathBuf,
    pub action: Action,
    pub target: String,
}

/// Control directory for a config: `control.dir` if set (relative to the
/// config's folder), otherwise `foo.json` → `foo.json.ctl`. None if disabled.
pub fn control_dir_for(config: &AppConfig, config_path: &Path) -> Option<PathBuf> {
    if !config.control.enabled { return None; }
    if !config.control.dir.trim().is_empty() {
        let base = config_path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
        return Some(resolve_path(config.control.dir.trim(), &base));
    }
    let mut p = config_path.to_path_buf();
    let name = p.file_name().unwrap_or_default().to_string_lossy().to_string() + ".ctl";
    p.set_file_name(name);
    Some(p)
}

fn parse_body(body: &str) -> Option<(Action, String)> {
    let body = body.trim();
    if body.starts_with('{') {
        let v: serde_json::Value = serde_json::from_str(body).ok()?;
        let action = Action::parse(v.get("action")?.as_str()?)?;
        let target = v.get("target").and_then(|t| t.as_str()).unwrap_or("").to_string();
        return Some((action, target));
    }
    let first_line = body.lines().next().unwrap_or("");
    let mut it = first_line.splitn(2, char::is_whitespace);
    let action = Action::parse(it.next()?)?;
    let target = it.next().unwrap_or("").trim().to_string();
    Some((action, target))
}

/// Write `{ok, message}` next to the command file. Best-effort: a client that
/// already gave up (or never waited) simply leaves a file we sweep later.
pub fn write_result(cmd_path: &Path, ok: bool, message: &str) {
    let out = serde_json::json!({ "ok": ok, "message": message }).to_string();
    let result = cmd_path.with_extension("result");
    let tmp    = cmd_path.with_extension("result.tmp");
    if std::fs::write(&tmp, out).is_ok() {
        let _ = std::fs::rename(&tmp, &result);
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// App side — watcher thread
// ══════════════════════════════════════════════════════════════════════════════

/// Watches the control directory on its own thread and executes commands
/// directly against the shared app state. A minimized/occluded window gets no
/// frames on macOS, so anything routed through the frame loop would stall
/// exactly when automation needs it most. The same tick also drains process
/// events, so exits and deferred replies are handled while minimized too.
pub struct ControlBus {
    dir:  PathBuf,
    stop: Arc<AtomicBool>,
}

const POLL_INTERVAL:  Duration = Duration::from_millis(300);
const SWEEP_INTERVAL: Duration = Duration::from_secs(60);
/// Result files nobody collected are deleted after this long.
const RESULT_TTL:     Duration = Duration::from_secs(600);

impl ControlBus {
    /// Creates the directory, clears commands left over from a previous run
    /// (executing yesterday's stale "stop" on a fresh instance would be a nasty
    /// surprise) and starts the watcher thread.
    pub fn new(dir: PathBuf, app: Weak<Mutex<ProConductor>>, ctx: egui::Context) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let p = e.path();
                let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
                if matches!(ext, "cmd" | "result" | "tmp") { let _ = std::fs::remove_file(&p); }
            }
        }
        let stop = Arc::new(AtomicBool::new(false));
        let (d, s) = (dir.clone(), stop.clone());
        thread::spawn(move || {
            let mut last_sweep = Instant::now();
            while !s.load(Ordering::Relaxed) {
                thread::sleep(POLL_INTERVAL);
                if last_sweep.elapsed() >= SWEEP_INTERVAL { last_sweep = Instant::now(); sweep(&d); }
                let cmds = collect(&d);           // filesystem work outside the lock
                let Some(app) = app.upgrade() else { return };
                let mut a = lock_app(&app);
                a.drain_events();
                if cmds.is_empty() { continue; }
                for c in cmds { a.handle_control_command(c); }
                drop(a);
                ctx.request_repaint();
            }
        });
        Self { dir, stop }
    }

    pub fn dir(&self) -> &Path { &self.dir }
}

impl Drop for ControlBus {
    fn drop(&mut self) { self.stop.store(true, Ordering::Relaxed); }
}

/// Newly arrived commands, oldest first. Unparseable files get an immediate
/// error result. Files are removed before they are handed out, so a crash
/// mid-execution can never replay a command on the next tick.
fn collect(dir: &Path) -> Vec<ControlCommand> {
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut files: Vec<(SystemTime, PathBuf)> = rd.flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("cmd"))
        .map(|p| (p.metadata().and_then(|m| m.modified()).unwrap_or(UNIX_EPOCH), p))
        .collect();
    files.sort();
    let mut out = vec![];
    for (_, path) in files {
        let body = std::fs::read_to_string(&path).unwrap_or_default();
        let _ = std::fs::remove_file(&path);
        match parse_body(&body) {
            Some((action, target)) => out.push(ControlCommand { path, action, target }),
            None => write_result(&path, false,
                "Unrecognised command. Use `start|stop|restart <target>` or `status`."),
        }
    }
    out
}

fn sweep(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for p in rd.flatten().map(|e| e.path()) {
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !matches!(ext, "result" | "tmp") { continue; }
        let age = p.metadata().and_then(|m| m.modified()).ok()
            .and_then(|m| SystemTime::now().duration_since(m).ok());
        if age.is_none_or(|a| a > RESULT_TTL) { let _ = std::fs::remove_file(&p); }
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Client side — used by the CLI mode of the binary
// ══════════════════════════════════════════════════════════════════════════════

/// Send one command to the instance owning `config_path` and wait for its
/// result. Returns Ok((ok, message)); Err if there is no instance / timeout.
pub fn send_command(config: &AppConfig, config_path: &Path, action: Action, target: &str, timeout: Duration) -> Result<(bool, String), String> {
    let Some(dir) = control_dir_for(config, config_path) else {
        return Err(format!("Remote control is disabled in {} (\"control\": {{\"enabled\": false}}).", config_path.display()));
    };
    if !dir.is_dir() {
        return Err(format!(
            "No running ProConductor instance for {} (control dir {} missing).",
            config_path.display(), dir.display()));
    }
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let base  = dir.join(format!("{}-{}", stamp, std::process::id()));
    let tmp   = base.with_extension("tmp");
    let cmd   = base.with_extension("cmd");
    let res   = base.with_extension("result");
    let body  = serde_json::json!({ "action": action.as_str(), "target": target }).to_string();
    std::fs::write(&tmp, body).map_err(|e| format!("Cannot write command: {}", e))?;
    std::fs::rename(&tmp, &cmd).map_err(|e| format!("Cannot write command: {}", e))?;

    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(s) = std::fs::read_to_string(&res) {
            let _ = std::fs::remove_file(&res);
            let v: serde_json::Value = serde_json::from_str(&s).unwrap_or_default();
            let ok  = v.get("ok").and_then(|b| b.as_bool()).unwrap_or(false);
            let msg = v.get("message").and_then(|m| m.as_str()).unwrap_or("").to_string();
            return Ok((ok, msg));
        }
        if Instant::now() >= deadline {
            // Withdraw the command if nobody picked it up — otherwise a later
            // instance would execute it out of the blue.
            let _ = std::fs::remove_file(&cmd);
            return Err(format!(
                "Timed out after {}s waiting for ProConductor ({}) — is it running?",
                timeout.as_secs(), config_path.display()));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_text_and_json() {
        assert_eq!(parse_body("restart MCP Server\n"), Some((Action::Restart, "MCP Server".into())));
        assert_eq!(parse_body("status"), Some((Action::Status, String::new())));
        assert_eq!(parse_body(r#"{"action":"stop","target":"api"}"#), Some((Action::Stop, "api".into())));
        assert_eq!(parse_body("dance"), None);
    }
}
