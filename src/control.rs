//! External control: lets scripts, agents and other processes start, stop,
//! restart and query components of a running ProConductor instance.
//!
//! Mechanism (dependency-free, event-driven, identical on Linux/macOS/Windows):
//!   * The instance listens on a loopback TCP socket (127.0.0.1, port from
//!     `control.port`, 0 = OS-assigned). The bound port is published in
//!     `<config>.port` next to the config file so clients find it.
//!   * Protocol: one request line, one reply line. Request is either JSON
//!     `{"action":"restart","target":"MCP Server"}` or plain text
//!     `restart MCP Server`. Reply is `{"ok":bool,"message":...}`.
//!     `printf 'restart MCP Server\n' | nc 127.0.0.1 $(cat app.json.port)` works.
//!   * For `stop`/`restart` the reply is sent only when the process is really
//!     down / really back up, so a client can block on it.
//!   * `proconductor foo.json --restart "MCP Server"` does exactly that from
//!     the CLI and exits 0/1 — the intended entry point for automation.
//!
//! Target resolution: component id, component name (case-insensitive), group
//! id, group name, or `all`.

use eframe::egui;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::Duration;

use crate::app::{lock_app, ProConductor};
use crate::config::AppConfig;

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

/// One connected client waiting for its reply line.
pub struct Replier(Option<TcpStream>);

impl Replier {
    /// Send `{ok, message}` and close. Best-effort: a client that already hung
    /// up simply doesn't get it.
    pub fn send(mut self, ok: bool, message: &str) {
        if let Some(mut s) = self.0.take() {
            let out = serde_json::json!({ "ok": ok, "message": message }).to_string();
            let _ = s.write_all(out.as_bytes());
            let _ = s.write_all(b"\n");
            let _ = s.flush();
        }
    }
}

pub struct ControlCommand {
    pub reply:  Replier,
    pub action: Action,
    pub target: String,
}

/// `foo.json` → `foo.json.port` — where the bound port is published.
pub fn port_file_for(config_path: &Path) -> PathBuf {
    let mut p = config_path.to_path_buf();
    let name = p.file_name().unwrap_or_default().to_string_lossy().to_string() + ".port";
    p.set_file_name(name);
    p
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

// ══════════════════════════════════════════════════════════════════════════════
// App side — listener thread
// ══════════════════════════════════════════════════════════════════════════════

/// Accepts control connections on its own thread and executes commands
/// directly against the shared app state — a minimized/occluded window gets no
/// frames on macOS, so anything routed through the frame loop would stall
/// exactly when automation needs it most.
pub struct ControlBus {
    port:      u16,
    port_file: PathBuf,
    stop:      Arc<AtomicBool>,
}

/// A client gets this long to send its request line.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

impl ControlBus {
    pub fn new(want_port: u16, config_path: &Path, app: Weak<Mutex<ProConductor>>, ctx: egui::Context) -> Result<Self, String> {
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, want_port)))
            .map_err(|e| format!("Remote control: cannot listen on 127.0.0.1:{}: {}", want_port, e))?;
        let port = listener.local_addr().map(|a| a.port()).unwrap_or(want_port);
        let port_file = port_file_for(config_path);
        std::fs::write(&port_file, port.to_string())
            .map_err(|e| format!("Remote control: cannot write {}: {}", port_file.display(), e))?;

        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                if s.load(Ordering::Relaxed) { break; }
                let Ok(stream) = stream else { continue };
                let Some(app) = app.upgrade() else { break };
                let ctx = ctx.clone();
                // One thread per connection so a slow client can't block others.
                thread::spawn(move || {
                    let Some(cmd) = read_request(stream) else { return };
                    let mut a = lock_app(&app);
                    a.drain_events();
                    a.handle_control_command(cmd);
                    drop(a);
                    ctx.request_repaint();
                });
            }
        });
        Ok(Self { port, port_file, stop })
    }

    pub fn port(&self) -> u16 { self.port }
}

impl Drop for ControlBus {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Unblock accept() so the thread notices the flag.
        let _ = TcpStream::connect_timeout(
            &SocketAddr::from((Ipv4Addr::LOCALHOST, self.port)), Duration::from_millis(200));
        let _ = std::fs::remove_file(&self.port_file);
    }
}

fn read_request(stream: TcpStream) -> Option<ControlCommand> {
    let _ = stream.set_read_timeout(Some(REQUEST_TIMEOUT));
    let _ = stream.set_write_timeout(Some(REQUEST_TIMEOUT));
    let mut line = String::new();
    {
        let mut r = BufReader::new(&stream);
        // Cap the request so a misbehaving client can't make us buffer forever.
        if r.by_ref().take(64 * 1024).read_line(&mut line).is_err() { return None; }
    }
    if line.trim().is_empty() { return None; } // our own wake-up connect, or noise
    match parse_body(&line) {
        Some((action, target)) => Some(ControlCommand { reply: Replier(Some(stream)), action, target }),
        None => {
            Replier(Some(stream)).send(false,
                "Unrecognised command. Use `start|stop|restart <target>` or `status`.");
            None
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Client side — used by the CLI mode of the binary
// ══════════════════════════════════════════════════════════════════════════════

/// Send one command to the instance owning `config_path` and wait for its
/// reply. Returns Ok((ok, message)); Err if there is no instance / timeout.
pub fn send_command(config: &AppConfig, config_path: &Path, action: Action, target: &str, timeout: Duration) -> Result<(bool, String), String> {
    if !config.control.enabled {
        return Err(format!("Remote control is disabled in {} (\"control\": {{\"enabled\": false}}).", config_path.display()));
    }
    let port_file = port_file_for(config_path);
    let port: u16 = std::fs::read_to_string(&port_file).ok()
        .and_then(|s| s.trim().parse().ok())
        .ok_or_else(|| format!(
            "No running ProConductor instance for {} ({} not found).",
            config_path.display(), port_file.display()))?;
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .map_err(|e| format!("No running ProConductor instance for {} (port {}: {}).", config_path.display(), port, e))?;
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_read_timeout(Some(timeout));
    let body = serde_json::json!({ "action": action.as_str(), "target": target }).to_string();
    stream.write_all(body.as_bytes()).and_then(|_| stream.write_all(b"\n"))
        .map_err(|e| format!("Cannot send command: {}", e))?;
    let mut line = String::new();
    match BufReader::new(&stream).read_line(&mut line) {
        Ok(0) => return Err("ProConductor closed the connection without a reply.".into()),
        Ok(_) => {}
        Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) =>
            return Err(format!("Timed out after {}s waiting for ProConductor ({}).", timeout.as_secs(), config_path.display())),
        Err(e) => return Err(format!("Error reading reply: {}", e)),
    }
    let v: serde_json::Value = serde_json::from_str(&line).unwrap_or_default();
    let ok  = v.get("ok").and_then(|b| b.as_bool()).unwrap_or(false);
    let msg = v.get("message").and_then(|m| m.as_str()).unwrap_or("").to_string();
    Ok((ok, msg))
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
