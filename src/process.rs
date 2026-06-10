//! Process lifecycle (kill-tree with escalation) and log plumbing
//! (line reader, on-disk writer, path resolution).

use std::io::BufRead;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::Child;
#[cfg(windows)]
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(windows)]
use crate::platform::windows::CREATE_NO_WINDOW;

// ══════════════════════════════════════════════════════════════════════════════
// Runtime types
// ══════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq)]
pub enum Source { Stdout, Stderr, System }

#[derive(Debug, Clone)]
pub struct LogLine {
    pub time:   String,
    pub source: Source,
    pub text:   String,
}
pub(crate) fn now_hms() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

pub(crate) fn format_uptime(started: Instant) -> String {
    let e = started.elapsed().as_secs();
    if e < 60   { return format!("{}s", e); }
    if e < 3600 { return format!("{}m {}s", e / 60, e % 60); }
    format!("{}h {}m", e / 3600, (e % 3600) / 60)
}

pub(crate) fn split_args(s: &str) -> Vec<String> {
    let mut args = vec![];
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut was_quoted = false; // so an explicitly empty "" argument is kept
    for ch in s.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => cur.push(ch),
            None => match ch {
                '"' | '\'' => { quote = Some(ch); was_quoted = true; }
                ' ' | '\t' => {
                    if !cur.is_empty() || was_quoted {
                        args.push(std::mem::take(&mut cur));
                        was_quoted = false;
                    }
                }
                _ => cur.push(ch),
            },
        }
    }
    if !cur.is_empty() || was_quoted { args.push(cur); }
    args
}

/// Resolve a path from the config: if relative, anchor it to the config file's
/// directory so the whole project folder is portable.
pub(crate) fn resolve_path(raw: &str, base_dir: &Path) -> PathBuf {
    let p = PathBuf::from(raw);
    if p.is_absolute() { p } else { base_dir.join(p) }
}

pub(crate) fn resolve_log_path(raw: &str, comp_name: &str, base_dir: &Path) -> Option<PathBuf> {
    if raw.is_empty() { return None; }
    // {name} is user input — strip path separators and reserved characters so a
    // component name can't redirect the log file elsewhere.
    let safe_name: String = comp_name.chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .collect();
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let expanded = raw.replace("{name}", &safe_name).replace("{date}", &date);
    Some(resolve_path(&expanded, base_dir))
}

/// How long a process tree gets to shut down gracefully before force-kill.
/// The UI "Stopping" progress bar is driven by this same constant.
pub(crate) const GRACE_PERIOD_MS: u64 = 8000;

#[cfg(unix)]
extern "C" { fn kill(pid: i32, sig: i32) -> i32; }
#[cfg(unix)]
pub(crate) fn libc_kill(pid: i32, sig: i32) { unsafe { kill(pid, sig); } }

/// Ask the whole process tree to shut down. Returns false if the request
/// could not be delivered (caller should skip the grace wait and force-kill).
#[cfg(unix)]
pub(crate) fn graceful_kill_tree(pid: u32) -> bool {
    libc_kill(-(pid as i32), 15); // SIGTERM to the process group
    true
}
#[cfg(unix)]
pub(crate) fn force_kill_tree(pid: u32) {
    libc_kill(-(pid as i32), 9); // SIGKILL to the process group
}

#[cfg(windows)]
extern "system" {
    pub(crate) fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
    pub(crate) fn TerminateProcess(handle: isize, code: u32) -> i32;
    pub(crate) fn CloseHandle(handle: isize) -> i32;
}
#[cfg(windows)] const PROCESS_TERMINATE: u32 = 0x0001;

// taskkill without /F sends WM_CLOSE to GUI processes; it refuses to signal
// console processes and reports failure — we use that to skip the grace wait.
// IMPORTANT: Never call FreeConsole()/AttachConsole() from our process —
// that invalidates our own stdio handles causing ERROR_INVALID_HANDLE (os error 6)
// on the next process spawn.
#[cfg(windows)]
pub(crate) fn graceful_kill_tree(pid: u32) -> bool {
    use std::os::windows::process::CommandExt;
    Command::new("taskkill")
        .args(["/T", "/PID", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// Force-kill: taskkill /F /T must run FIRST — it enumerates the tree from the
// root pid, so terminating the root before it would orphan every grandchild.
// TerminateProcess on the root is the fallback if taskkill itself fails.
#[cfg(windows)]
pub(crate) fn force_kill_tree(pid: u32) {
    use std::os::windows::process::CommandExt;
    let _ = Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if h != 0 { TerminateProcess(h, 1); CloseHandle(h); }
    }
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn graceful_kill_tree(pid: u32) -> bool { let _ = pid; true }
#[cfg(not(any(unix, windows)))]
pub(crate) fn force_kill_tree(pid: u32) { let _ = pid; }

/// Full stop sequence for one child: graceful signal → poll for exit up to
/// `grace_ms` → force-kill the tree → reap the child so it never zombies.
/// Blocks the calling thread; run on a worker thread for interactive stops.
pub(crate) fn stop_process_tree(pid: u32, child: &Arc<Mutex<Child>>, grace_ms: u64) {
    let graceful = graceful_kill_tree(pid);
    // If the graceful request couldn't be delivered there is nothing to wait for.
    let wait_ms = if graceful { grace_ms } else { 300 };
    let deadline = Instant::now() + Duration::from_millis(wait_ms);
    loop {
        if let Ok(mut c) = child.try_lock() {
            if matches!(c.try_wait(), Ok(Some(_))) { return; } // exited + reaped
        }
        if Instant::now() >= deadline { break; }
        thread::sleep(Duration::from_millis(100));
    }
    force_kill_tree(pid);
    if let Ok(mut c) = child.lock() {
        let _ = c.kill();
        let _ = c.wait(); // reap — no zombie left behind
    }
}
/// On-disk log writer. Re-resolves the {date}/{name} template on every line so
/// the file rolls over at midnight, and emits each line as a single write_all
/// on an O_APPEND handle so concurrent writers can't interleave mid-line.
pub(crate) struct LogWriter {
    template:       String,
    comp_name:      String,
    base_dir:       PathBuf,
    current_path:   Option<PathBuf>,
    file:           Option<std::fs::File>,
    error_reported: bool,
}

impl LogWriter {
    pub(crate) fn new(template: String, comp_name: String, base_dir: PathBuf) -> Self {
        Self { template, comp_name, base_dir, current_path: None, file: None, error_reported: false }
    }

    /// Write one line. Returns an error message exactly once per failing path
    /// so the caller can surface it in the in-app log instead of dropping it.
    pub(crate) fn write_line(&mut self, src: &str, text: &str) -> Option<String> {
        use std::io::Write;
        let want = resolve_log_path(&self.template, &self.comp_name, &self.base_dir)?;
        if self.file.is_none() || self.current_path.as_ref() != Some(&want) {
            if let Some(par) = want.parent() { let _ = std::fs::create_dir_all(par); }
            match std::fs::OpenOptions::new().create(true).append(true).open(&want) {
                Ok(f) => {
                    self.file = Some(f);
                    self.current_path = Some(want);
                    self.error_reported = false;
                }
                Err(e) => {
                    self.file = None;
                    if !self.error_reported {
                        self.error_reported = true;
                        return Some(format!("Cannot open log file {}: {}", want.display(), e));
                    }
                    return None;
                }
            }
        }
        if let Some(f) = &mut self.file {
            let _ = f.write_all(format!("[{}] {}\n", src, text).as_bytes());
        }
        None
    }
}

/// Stream a pipe into lines without the failure modes of `.lines()`:
/// non-UTF8 bytes are replaced (not dropped), a newline-free stream is flushed
/// every MAX_LINE_BYTES instead of buffering unboundedly, and a persistent
/// read error ends the loop instead of spinning forever.
pub(crate) const MAX_LINE_BYTES: usize = 16 * 1024;

pub(crate) fn read_lines_capped<R: std::io::Read>(inner: R, mut on_line: impl FnMut(String)) {
    let mut r = BufReader::new(inner);
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let available = match r.fill_buf() {
            Ok(a) => a,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        if available.is_empty() { break; } // EOF
        let avail_len = available.len();
        let mut start = 0;
        while start < avail_len {
            match available[start..].iter().position(|&b| b == b'\n') {
                Some(rel) => {
                    buf.extend_from_slice(&available[start..start + rel]);
                    if buf.last() == Some(&b'\r') { buf.pop(); }
                    on_line(String::from_utf8_lossy(&buf).into_owned());
                    buf.clear();
                    start += rel + 1;
                }
                None => {
                    buf.extend_from_slice(&available[start..]);
                    start = avail_len;
                }
            }
            if buf.len() >= MAX_LINE_BYTES {
                on_line(String::from_utf8_lossy(&buf).into_owned());
                buf.clear();
            }
        }
        r.consume(avail_len);
    }
    if !buf.is_empty() {
        on_line(String::from_utf8_lossy(&buf).into_owned());
    }
}

