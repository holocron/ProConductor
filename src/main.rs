#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui::{self, Align, Color32, FontId, Layout, RichText, Stroke, Vec2};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

#[cfg(windows)]
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    TrayIconBuilder, TrayIconEvent,
};

// Win32 FFI — taskbar removal + message pump for tray events
#[cfg(windows)]
#[repr(C)]
struct MSG { hwnd: isize, message: u32, w: usize, l: isize, time: u32, pt_x: i32, pt_y: i32 }

#[cfg(windows)]
#[repr(C)]
struct RECT { left: i32, top: i32, right: i32, bottom: i32 }

#[cfg(windows)]
extern "system" {
    fn FindWindowW(class: *const u16, title: *const u16) -> isize;
    fn GetWindowLongPtrW(hwnd: isize, index: i32) -> isize;
    fn SetWindowLongPtrW(hwnd: isize, index: i32, new: isize) -> isize;
    fn GetWindowRect(hwnd: isize, rect: *mut RECT) -> i32;
    fn SetWindowPos(hwnd: isize, insert_after: isize, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
    fn PeekMessageW(msg: *mut MSG, hwnd: isize, min: u32, max: u32, remove: u32) -> i32;
    fn TranslateMessage(msg: *const MSG) -> i32;
    fn DispatchMessageW(msg: *const MSG) -> isize;
    fn ShowWindow(hwnd: isize, cmd: i32) -> i32;
    fn SetForegroundWindow(hwnd: isize) -> i32;
}
#[cfg(windows)] const SW_HIDE:    i32 = 0;
#[cfg(windows)] const SW_RESTORE: i32 = 9;
#[cfg(windows)] const PM_REMOVE: u32 = 1;
#[cfg(windows)] const GWL_EXSTYLE:      i32 = -20;
#[cfg(windows)] const WS_EX_APPWINDOW:  isize = 0x00040000;
#[cfg(windows)] const WS_EX_TOOLWINDOW: isize = 0x00000080;
#[cfg(windows)] const SWP_NOMOVE:       u32 = 0x0002;
#[cfg(windows)] const SWP_NOSIZE:       u32 = 0x0001;
#[cfg(windows)] const SWP_NOZORDER:     u32 = 0x0004;
#[cfg(windows)] const SWP_FRAMECHANGED: u32 = 0x0020;
use std::{
    collections::{HashMap, HashSet},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

// Tray icon variants — 32x32 RGBA, loaded from icons/ at compile time
#[cfg(windows)]
const TRAY_ICON_W: u32 = 32;
#[cfg(windows)]
const TRAY_ICON_H: u32 = 32;
#[cfg(windows)]
const TRAY_ICON_GREEN: &[u8] = include_bytes!("../icons/tray_green.rgba");
#[cfg(windows)]
const TRAY_ICON_AMBER: &[u8] = include_bytes!("../icons/tray_amber.rgba");
#[cfg(windows)]
const TRAY_ICON_RED:   &[u8] = include_bytes!("../icons/tray_red.rgba");

// macOS dock icon variants — 64x64 RGBA, loaded from icons/ at compile time
#[cfg(target_os = "macos")]
const DOCK_ICON_W: u32 = 64;
#[cfg(target_os = "macos")]
const DOCK_ICON_H: u32 = 64;
#[cfg(target_os = "macos")]
const DOCK_ICON_GREEN: &[u8] = include_bytes!("../icons/dock_green.rgba");
#[cfg(target_os = "macos")]
const DOCK_ICON_AMBER: &[u8] = include_bytes!("../icons/dock_amber.rgba");
#[cfg(target_os = "macos")]
const DOCK_ICON_RED:   &[u8] = include_bytes!("../icons/dock_red.rgba");

// Shared registry of running child PIDs — read by the signal handler to kill
// all children before the process exits (e.g. on Ctrl+C or SIGTERM).
type PidRegistry = Arc<Mutex<Vec<u32>>>;

// ══════════════════════════════════════════════════════════════════════════════
// Palette
// ══════════════════════════════════════════════════════════════════════════════

const BG_BASE:    Color32 = Color32::from_rgb(  9,  12,  18);
const BG_PANEL:   Color32 = Color32::from_rgb( 14,  20,  32);
const BG_CARD:    Color32 = Color32::from_rgb( 22,  30,  46);
const BG_RAISED:  Color32 = Color32::from_rgb( 18,  25,  38);
const BG_INPUT:   Color32 = Color32::from_rgb( 10,  15,  24);
const BG_HOVER:   Color32 = Color32::from_rgb( 26,  40,  64);
const BG_SEL:     Color32 = Color32::from_rgb( 18,  32,  68);

const BORDER:     Color32 = Color32::from_rgb( 28,  42,  62);
const BORDER_HI:  Color32 = Color32::from_rgb( 46,  64,  96);

const TEXT_PRI:   Color32 = Color32::from_rgb(220, 232, 245);
const TEXT_SEC:   Color32 = Color32::from_rgb(122, 155, 191);
const TEXT_MUTED: Color32 = Color32::from_rgb( 61,  85, 112);
const TEXT_DIM:   Color32 = Color32::from_rgb( 35,  52,  72);

const GREEN:      Color32 = Color32::from_rgb( 33, 212, 126);
const GREEN_DIM:  Color32 = Color32::from_rgb( 14,  74,  44);
const GREEN_BG:   Color32 = Color32::from_rgb(  7,  28,  18);
const RED:        Color32 = Color32::from_rgb(240,  74,  94);
const RED_DIM:    Color32 = Color32::from_rgb( 74,  15,  22);
const RED_BG:     Color32 = Color32::from_rgb( 30,   6,  10);
const AMBER:      Color32 = Color32::from_rgb(240, 160,  48);
const AMBER_DIM:  Color32 = Color32::from_rgb( 74,  48,  10);
const AMBER_BG:   Color32 = Color32::from_rgb( 30,  20,   7);
const BLUE:       Color32 = Color32::from_rgb( 68, 136, 255);
const BLUE_DIM:   Color32 = Color32::from_rgb( 18,  32, 100);

// Button-state shades — see action_button()
const RED_GHOST:      Color32 = Color32::from_rgb(150,  70,  80);
const BLUE_HI:        Color32 = Color32::from_rgb(120, 170, 255);
const BLUE_BORDER:    Color32 = Color32::from_rgb( 30,  50, 120);
const GREEN_BG_HOVER: Color32 = Color32::from_rgb( 10,  42,  27);
const AMBER_BG_HOVER: Color32 = Color32::from_rgb( 45,  30,  10);

// ══════════════════════════════════════════════════════════════════════════════
// Log syntax highlighting — span-based pipeline (inspired by tailspin)
// ══════════════════════════════════════════════════════════════════════════════

// ══════════════════════════════════════════════════════════════════════════════
// Config — lives in the .json file the user passes
// ══════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnvVar {
    pub key:   String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Component {
    #[serde(default)] pub id:   String,
    #[serde(default)] pub name: String,
    #[serde(default)] pub executable:  String,
    #[serde(default)] pub working_dir: String,
    #[serde(default)] pub args:        String,
    #[serde(default)] pub log_path:    String,
    #[serde(default)] pub run_as_user: String,
    #[serde(default)] pub env_vars:    Vec<EnvVar>,
}

impl Component {
    fn new() -> Self {
        Self {
            id:           Uuid::new_v4().to_string(),
            name:         String::new(),
            executable:   String::new(),
            working_dir:  String::new(),
            args:         String::new(),
            log_path:     String::new(),
            run_as_user:  String::new(),
            env_vars:     vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    #[serde(default)] pub id:   String,
    #[serde(default)] pub name: String,
    #[serde(default)] pub components: Vec<Component>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)] pub groups: Vec<Group>,
}

/// Load the config. A missing file is normal (fresh start); an unreadable or
/// malformed file is NOT silently replaced — the original is backed up first
/// and the error is surfaced so a later save can't destroy user data.
fn load_config(path: &Path) -> (AppConfig, Option<String>) {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return (AppConfig::default(), None);
        }
        Err(e) => {
            return (AppConfig::default(), Some(format!(
                "Could not read {}: {}. Starting empty — saving will overwrite it.",
                path.display(), e
            )));
        }
    };
    match serde_json::from_str::<AppConfig>(&raw) {
        Ok(c)  => (c, None),
        Err(e) => {
            let backup = path.with_extension("json.corrupt");
            let note = match std::fs::copy(path, &backup) {
                Ok(_)   => format!("Original backed up to {}.", backup.display()),
                Err(be) => format!("Backup failed: {}.", be),
            };
            (AppConfig::default(), Some(format!(
                "Config {} is not valid JSON ({}). {} Starting empty.",
                path.display(), e, note
            )))
        }
    }
}

/// Repair missing/duplicate ids (e.g. hand-edited config). Returns true if
/// anything changed so the caller can mark the config dirty.
fn sanitize_config(config: &mut AppConfig) -> bool {
    let mut changed = false;
    let mut seen: HashSet<String> = HashSet::new();
    for g in &mut config.groups {
        if g.id.is_empty() || !seen.insert(g.id.clone()) {
            g.id = Uuid::new_v4().to_string();
            seen.insert(g.id.clone());
            changed = true;
        }
        for c in &mut g.components {
            if c.id.is_empty() || !seen.insert(c.id.clone()) {
                c.id = Uuid::new_v4().to_string();
                seen.insert(c.id.clone());
                changed = true;
            }
        }
    }
    changed
}

/// Atomic save: write to a temp file, then rename over the target — a crash
/// mid-write can never leave a truncated config behind. Errors propagate so
/// the caller can keep the dirty flag set and tell the user.
fn save_config(config: &AppConfig, path: &Path) -> std::io::Result<()> {
    if let Some(p) = path.parent() {
        if !p.as_os_str().is_empty() { std::fs::create_dir_all(p)?; }
    }
    let s = serde_json::to_string_pretty(config)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, s)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

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

pub enum AppEvent {
    Log         { id: String, line: LogLine },
    Status      { id: String, running: bool, pid: Option<u32>, exit_code: Option<i32> },
    Stopped     { id: String },
    ShowWindow,
    QuitApp,
}

/// Commands sent from main thread → tray background thread
#[cfg(windows)]
enum TrayCmd {
    SetIcon(u8),    // 0=red 1=amber 2=green
    SetHwnd(isize), // share HWND so tray thread can call ShowWindow directly
}

struct RunningProcess {
    pid:        u32,
    started_at: Instant,
    child:      Arc<Mutex<Child>>,
    cancelled:  Arc<std::sync::atomic::AtomicBool>,
}

fn now_hms() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

fn format_uptime(started: Instant) -> String {
    let e = started.elapsed().as_secs();
    if e < 60   { return format!("{}s", e); }
    if e < 3600 { return format!("{}m {}s", e / 60, e % 60); }
    format!("{}h {}m", e / 3600, (e % 3600) / 60)
}

fn split_args(s: &str) -> Vec<String> {
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
fn resolve_path(raw: &str, base_dir: &Path) -> PathBuf {
    let p = PathBuf::from(raw);
    if p.is_absolute() { p } else { base_dir.join(p) }
}

fn resolve_log_path(raw: &str, comp_name: &str, base_dir: &Path) -> Option<PathBuf> {
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
const GRACE_PERIOD_MS: u64 = 8000;

#[cfg(unix)]
extern "C" { fn kill(pid: i32, sig: i32) -> i32; }
#[cfg(unix)]
fn libc_kill(pid: i32, sig: i32) { unsafe { kill(pid, sig); } }

/// Ask the whole process tree to shut down. Returns false if the request
/// could not be delivered (caller should skip the grace wait and force-kill).
#[cfg(unix)]
fn graceful_kill_tree(pid: u32) -> bool {
    libc_kill(-(pid as i32), 15); // SIGTERM to the process group
    true
}
#[cfg(unix)]
fn force_kill_tree(pid: u32) {
    libc_kill(-(pid as i32), 9); // SIGKILL to the process group
}

#[cfg(windows)]
extern "system" {
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
    fn TerminateProcess(handle: isize, code: u32) -> i32;
    fn CloseHandle(handle: isize) -> i32;
}
#[cfg(windows)] const PROCESS_TERMINATE: u32 = 0x0001;

// taskkill without /F sends WM_CLOSE to GUI processes; it refuses to signal
// console processes and reports failure — we use that to skip the grace wait.
// IMPORTANT: Never call FreeConsole()/AttachConsole() from our process —
// that invalidates our own stdio handles causing ERROR_INVALID_HANDLE (os error 6)
// on the next process spawn.
#[cfg(windows)]
fn graceful_kill_tree(pid: u32) -> bool {
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
fn force_kill_tree(pid: u32) {
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
fn graceful_kill_tree(pid: u32) -> bool { let _ = pid; true }
#[cfg(not(any(unix, windows)))]
fn force_kill_tree(pid: u32) { let _ = pid; }

/// Full stop sequence for one child: graceful signal → poll for exit up to
/// `grace_ms` → force-kill the tree → reap the child so it never zombies.
/// Blocks the calling thread; run on a worker thread for interactive stops.
fn stop_process_tree(pid: u32, child: &Arc<Mutex<Child>>, grace_ms: u64) {
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

// ══════════════════════════════════════════════════════════════════════════════
// macOS dock icon
// ══════════════════════════════════════════════════════════════════════════════

#[cfg(target_os = "macos")]
fn set_dock_icon(rgba: &[u8], width: u32, height: u32) {
    use objc2_app_kit::{NSApplication, NSBitmapImageRep, NSImage};
    use objc2_foundation::{NSString, NSSize, MainThreadMarker};
    use objc2::AnyThread;

    // The copy below trusts the caller's dimensions; a short buffer would read
    // out of bounds. (bytesPerRow is pinned to width*4, so no row padding.)
    if rgba.len() != (width as usize) * (height as usize) * 4 { return; }

    unsafe {
        let color_space = NSString::from_str("NSDeviceRGBColorSpace");

        // Build NSBitmapImageRep from our RGBA buffer
        let rep = NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            width as isize,
            height as isize,
            8,
            4,
            true,
            false,
            &color_space,
            (width * 4) as isize,
            32,
        );
        if let Some(rep) = rep {
            // Copy pixel data
            let dst: *mut u8 = rep.bitmapData();
            if !dst.is_null() {
                std::ptr::copy_nonoverlapping(rgba.as_ptr(), dst, rgba.len());
            }
            // Build NSImage and set as dock icon
            let size = NSSize { width: width as f64, height: height as f64 };
            let img = NSImage::initWithSize(NSImage::alloc(), size);
            img.addRepresentation(&rep);
            // SAFETY: dock icon is set from the egui paint callback, which runs on the main thread.
            let mtm = MainThreadMarker::new_unchecked();
            NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&img));
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// UI state helpers
// ══════════════════════════════════════════════════════════════════════════════

#[derive(PartialEq, Clone)]
enum MainView { Dashboard, Log, Edit }

#[derive(Clone)]
struct EditState {
    component:  Component,
    group_id:   String,
    is_new:     bool,
}

// ══════════════════════════════════════════════════════════════════════════════
// App
// ══════════════════════════════════════════════════════════════════════════════

struct ProConductor {
    // Persistent
    config:      AppConfig,
    config_path: PathBuf,
    dirty:       bool,
    // Held for entire lifetime — OS releases exclusive lock on drop (incl. crash)
    _lock:       std::fs::File,
    // Shared with signal handler — all running child PIDs
    pid_registry: PidRegistry,

    // Runtime
    running:    HashMap<String, RunningProcess>,
    stopping:   HashMap<String, Instant>,   // id → when stop was requested
    last_exit:  HashMap<String, i32>,       // id → last nonzero exit code (crash badge)
    logs:       HashMap<String, Vec<LogLine>>,
    event_tx:   mpsc::Sender<AppEvent>,
    event_rx:   mpsc::Receiver<AppEvent>,
    ctx_handle: egui::Context,

    // Error banners — shown until dismissed
    config_load_error: Option<String>,
    save_error:        Option<String>,

    // UI
    start_minimized: bool,   // send minimize command on first frame
    #[cfg(windows)]
    tray_cmd_tx: Option<mpsc::Sender<TrayCmd>>,  // send icon-change cmds to tray thread
    #[cfg(windows)]
    tray_status: u8,               // 0=red 1=amber 2=green — track to avoid redundant swaps
    show_window_requested: bool,
    quit_requested:        bool,
    #[cfg(target_os = "macos")]
    macos_dock_status: u8,   // 0=red 1=amber 2=green
    #[cfg(windows)]
    taskbar_hidden: bool,  // whether we've removed the window from taskbar yet
    #[cfg(windows)]
    just_hid: bool,        // skip one frame after hiding to avoid instant restore
    selected_comp:  Option<String>,
    selected_group: Option<String>,
    view:           MainView,
    edit:           Option<EditState>,
    open_groups:    HashSet<String>,
    log_filter:     String,
    log_autoscroll: bool,
    confirm_delete: Option<(String, String)>, // (kind, id)
}

impl ProConductor {
    #[allow(clippy::too_many_arguments)]
    fn new(cc: &eframe::CreationContext, config: AppConfig, path: PathBuf, lock: std::fs::File, pid_registry: PidRegistry, autostart: bool, minimized: bool, load_error: Option<String>, initial_dirty: bool) -> Self {
        // Apply dark theme with our custom palette
        let mut visuals = egui::Visuals::dark();
        visuals.window_fill              = BG_BASE;
        visuals.panel_fill               = BG_PANEL;
        visuals.faint_bg_color           = BG_CARD;
        visuals.extreme_bg_color         = BG_INPUT;
        visuals.widgets.noninteractive.bg_fill  = BG_CARD;
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_MUTED);
        visuals.widgets.inactive.bg_fill        = BG_CARD;
        visuals.widgets.inactive.fg_stroke      = Stroke::new(1.0, TEXT_SEC);
        visuals.widgets.hovered.bg_fill         = BG_HOVER;
        visuals.widgets.hovered.fg_stroke       = Stroke::new(1.0, TEXT_PRI);
        visuals.widgets.active.bg_fill          = BG_SEL;
        visuals.widgets.active.fg_stroke        = Stroke::new(1.0, TEXT_PRI);
        visuals.selection.bg_fill               = BLUE_DIM;
        visuals.selection.stroke                = Stroke::new(1.0, BLUE);
        visuals.override_text_color             = Some(TEXT_PRI);
        cc.egui_ctx.set_visuals(visuals);

        // Slightly larger default font
        let mut style = (*cc.egui_ctx.style()).clone();
        style.text_styles.insert(
            egui::TextStyle::Body,
            FontId::new(13.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Small,
            FontId::new(11.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Monospace,
            FontId::new(11.5, egui::FontFamily::Monospace),
        );
        style.spacing.item_spacing    = Vec2::new(6.0, 4.0);
        style.spacing.button_padding  = Vec2::new(8.0, 4.0);
        style.spacing.window_margin   = egui::Margin::same(0.0);
        cc.egui_ctx.set_style(style);

        let (tx, rx) = mpsc::channel();

        // Open all groups initially
        let open_groups = config.groups.iter().map(|g| g.id.clone()).collect();

        // tray-icon uses Rc internally — not Send — so the TrayIcon handle can never
        // leave the thread it was built on. We keep it entirely inside the background
        // thread and communicate via channels:
        //   app → thread : TrayCmd  (icon colour changes)
        //   thread → app : AppEvent (show/quit)
        #[cfg(windows)]
        let tray_cmd_tx: Option<mpsc::Sender<TrayCmd>> = {
            let tx_app    = tx.clone();
            let ctx_tray  = cc.egui_ctx.clone();
            let (cmd_tx, cmd_rx) = mpsc::channel::<TrayCmd>();

            thread::spawn(move || {
                // Build tray on this thread — never send the handle anywhere
                let icon = tray_icon::Icon::from_rgba(TRAY_ICON_RED.to_vec(), TRAY_ICON_W, TRAY_ICON_H)
                    .unwrap_or_else(|_| tray_icon::Icon::from_rgba(vec![240u8,74,94,255], 1, 1).unwrap());
                let menu = Menu::new();
                let _ = menu.append(&MenuItem::with_id("show", "Show", true, None));
                let _ = menu.append(&MenuItem::with_id("quit", "Quit", true, None));
                let tray = match TrayIconBuilder::new()
                    .with_menu(Box::new(menu))
                    .with_tooltip("ProConductor")
                    .with_icon(icon)
                    .build() {
                    Ok(t) => t,
                    Err(_) => return,
                };

                let mut hwnd: isize = 0;
                loop {
                    // Must pump Win32 messages on the thread that built the tray window
                    unsafe {
                        let mut msg = MSG { hwnd:0, message:0, w:0, l:0, time:0, pt_x:0, pt_y:0 };
                        while PeekMessageW(&mut msg, 0, 0, 0, PM_REMOVE) != 0 {
                            TranslateMessage(&msg);
                            DispatchMessageW(&msg);
                        }
                    }
                    // Tray click / menu → act via Win32 directly, no egui needed
                    if let Ok(ev) = TrayIconEvent::receiver().try_recv() {
                        if matches!(ev, TrayIconEvent::Click { .. }) {
                            unsafe { ShowWindow(hwnd, SW_RESTORE); SetForegroundWindow(hwnd); }
                        }
                    }
                    if let Ok(ev) = MenuEvent::receiver().try_recv() {
                        match ev.id.0.as_str() {
                            "show" => unsafe { ShowWindow(hwnd, SW_RESTORE); SetForegroundWindow(hwnd); },
                            _      => { let _ = tx_app.send(AppEvent::QuitApp); ctx_tray.request_repaint(); }
                        }
                    }
                    // Commands from app thread
                    while let Ok(cmd) = cmd_rx.try_recv() {
                        match cmd {
                            TrayCmd::SetHwnd(h) => hwnd = h,
                            TrayCmd::SetIcon(status) => {
                                let rgba = match status { 2 => TRAY_ICON_GREEN, 1 => TRAY_ICON_AMBER, _ => TRAY_ICON_RED };
                                if let Ok(icon) = tray_icon::Icon::from_rgba(rgba.to_vec(), TRAY_ICON_W, TRAY_ICON_H) {
                                    let _ = tray.set_icon(Some(icon));
                                }
                            }
                        }
                    }
                    thread::sleep(Duration::from_millis(50));
                }
            });

            Some(cmd_tx)
        };

        let mut app = Self {
            config,
            config_path: path,
            dirty: initial_dirty,
            _lock: lock,
            pid_registry,
            running: HashMap::new(),
            stopping: HashMap::new(),
            last_exit: HashMap::new(),
            logs: HashMap::new(),
            config_load_error: load_error,
            save_error: None,
            event_tx: tx,
            event_rx: rx,
            ctx_handle: cc.egui_ctx.clone(),
            selected_comp: None,
            selected_group: None,
            view: MainView::Dashboard,
            edit: None,
            open_groups,
            log_filter: String::new(),
            log_autoscroll: true,
            confirm_delete: None,
            start_minimized: minimized,
            #[cfg(windows)]
            tray_cmd_tx: tray_cmd_tx,
            #[cfg(windows)]
            tray_status: 0,
            show_window_requested: false,
            quit_requested:        false,
            #[cfg(target_os = "macos")]
            macos_dock_status: 0,
            #[cfg(windows)]
            taskbar_hidden: false,
            #[cfg(windows)]
            just_hid: false,
        };
        if autostart { app.start_all(); }

        // Set initial dock icon immediately (red = nothing running yet)
        #[cfg(target_os = "macos")]
        set_dock_icon(DOCK_ICON_RED, DOCK_ICON_W, DOCK_ICON_H);

        app
    }

    // ── Process spawning ───────────────────────────────────────────────────

    fn start(&mut self, comp: &Component) {
        if self.running.contains_key(&comp.id) { return; }
        // Clear any stale stopping entry — user clicked Start before grace period ended
        self.stopping.remove(&comp.id);
        self.last_exit.remove(&comp.id);

        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut args = split_args(&comp.args);

        // sudo -n -u <user> -- <exe> on Unix.
        // -n: fail fast instead of hanging on a password prompt (we have no TTY).
        // --: stop option parsing so a user/executable starting with '-' can't
        //     inject sudo options.
        let exe = if !comp.run_as_user.is_empty() {
            #[cfg(unix)] {
                args.insert(0, comp.executable.clone());
                args.insert(0, "--".into());
                args.insert(0, comp.run_as_user.clone());
                args.insert(0, "-u".into());
                args.insert(0, "-n".into());
                "sudo".to_string()
            }
            #[cfg(not(unix))]
            { comp.executable.clone() }
        } else {
            comp.executable.clone()
        };

        // Base dir for resolving relative paths = directory containing the config file
        let base_dir = self.config_path.parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        let mut cmd = Command::new(&exe);
        cmd.args(&args).stdout(Stdio::piped()).stderr(Stdio::piped());
        if !comp.working_dir.is_empty() {
            cmd.current_dir(resolve_path(&comp.working_dir, &base_dir));
        }
        for ev in &comp.env_vars { if !ev.key.is_empty() { cmd.env(&ev.key, &ev.value); } }

        // Unix: new process group for clean tree-kill
        #[cfg(unix)] { use std::os::unix::process::CommandExt; cmd.process_group(0); }

        // Windows: hide child console windows — stdout/stderr captured via pipes
        #[cfg(windows)] {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                self.logs.entry(comp.id.clone()).or_default().push(LogLine {
                    time: now_hms(), source: Source::System,
                    text: format!("Failed to start: {}", e),
                });
                self.ctx_handle.request_repaint();
                return;
            }
        };

        let pid = child.id();
        let child_arc = Arc::new(Mutex::new(child));

        // Lazily-opened, date-rollover-aware on-disk log writer (shared by both
        // reader threads)
        let log_writer: Option<Arc<Mutex<LogWriter>>> = (!comp.log_path.is_empty())
            .then(|| Arc::new(Mutex::new(LogWriter::new(
                comp.log_path.clone(), comp.name.clone(), base_dir.clone(),
            ))));

        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));

        // Reader thread body, shared by stdout/stderr: forward each line to the
        // UI and the on-disk log; surface log-file open failures as SYS lines.
        let spawn_reader = |pipe: Box<dyn std::io::Read + Send>, source: Source, disk_tag: &'static str| {
            let tx     = self.event_tx.clone();
            let id     = comp.id.clone();
            let writer = log_writer.clone();
            let ctx    = self.ctx_handle.clone();
            thread::spawn(move || {
                read_lines_capped(pipe, |text| {
                    if let Some(w) = &writer {
                        if let Ok(mut w) = w.lock() {
                            if let Some(err) = w.write_line(disk_tag, &text) {
                                let _ = tx.send(AppEvent::Log { id: id.clone(), line: LogLine {
                                    time: now_hms(), source: Source::System, text: err,
                                }});
                            }
                        }
                    }
                    let _ = tx.send(AppEvent::Log { id: id.clone(), line: LogLine {
                        time: now_hms(), source: source.clone(), text,
                    }});
                    ctx.request_repaint();
                });
            });
        };

        let stdout = child_arc.lock().unwrap().stdout.take().unwrap();
        spawn_reader(Box::new(stdout), Source::Stdout, "STDOUT");
        let stderr = child_arc.lock().unwrap().stderr.take().unwrap();
        spawn_reader(Box::new(stderr), Source::Stderr, "STDERR");

        // ── Monitor thread: wait for exit
        let child_mon = child_arc.clone();
        let tx_mon    = self.event_tx.clone();
        let id_mon    = comp.id.clone();
        let ctx_mon   = self.ctx_handle.clone();
        let cancelled_mon = cancelled.clone();
        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_millis(200));
                // If stop() was called for this component, the entry in `running`
                // has already been removed. Firing Status here would wrongly remove
                // a NEW process started with the same component id. Bail out.
                if cancelled_mon.load(std::sync::atomic::Ordering::Relaxed) { break; }
                let mut guard = match child_mon.lock() { Ok(g) => g, Err(_) => break };
                match guard.try_wait() {
                    Ok(Some(status)) => {
                        if cancelled_mon.load(std::sync::atomic::Ordering::Relaxed) { break; }
                        #[cfg(unix)] let code = {
                            use std::os::unix::process::ExitStatusExt;
                            status.code().or_else(|| status.signal().map(|s| -s))
                        };
                        #[cfg(not(unix))] let code = status.code();
                        let _ = tx_mon.send(AppEvent::Log { id: id_mon.clone(), line: LogLine {
                            time: now_hms(), source: Source::System,
                            text: format!("Process exited (code {:?})", code),
                        }});
                        let _ = tx_mon.send(AppEvent::Status {
                            id: id_mon, running: false, pid: None, exit_code: code,
                        });
                        ctx_mon.request_repaint();
                        break;
                    }
                    Ok(None) => {} // still running
                    Err(_)   => break,
                }
            }
        });

        self.running.insert(comp.id.clone(), RunningProcess {
            pid, started_at: Instant::now(), child: child_arc,
            cancelled: cancelled.clone(),
        });
        // Register PID so signal handler can kill it on Ctrl+C
        self.pid_registry.lock().unwrap().push(pid);

        self.logs.entry(comp.id.clone()).or_default().push(LogLine {
            time: now_hms(), source: Source::System,
            text: format!("Started PID {}", pid),
        });
    }

    fn stop(&mut self, id: &str) {
        if let Some(handle) = self.running.remove(id) {
            // Cancel the monitor thread so it doesn't fire Status on the next start
            handle.cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
            self.stopping.insert(id.to_string(), Instant::now());
            self.logs.entry(id.to_string()).or_default().push(LogLine {
                time: now_hms(), source: Source::System, text: "Stop requested…".into(),
            });
            self.ctx_handle.request_repaint();
            let pid       = handle.pid;
            let child_arc = handle.child.clone();
            let ctx       = self.ctx_handle.clone();
            let tx        = self.event_tx.clone();
            let id_owned  = id.to_string();
            let registry  = self.pid_registry.clone();
            thread::spawn(move || {
                stop_process_tree(pid, &child_arc, GRACE_PERIOD_MS);
                // Only now is the process confirmed dead — deregistering earlier
                // would hide a TERM-ignoring child from the Ctrl+C handler.
                registry.lock().unwrap().retain(|&p| p != pid);
                let _ = tx.send(AppEvent::Stopped { id: id_owned });
                ctx.request_repaint();
            });
        }
    }

    /// Synchronous shutdown for app exit: signal every tree, give them a short
    /// shared grace window, then force-kill stragglers — all on this thread,
    /// because detached kill threads die with the process.
    fn shutdown_sync(&mut self) {
        const EXIT_GRACE_MS: u64 = 2000;
        let handles: Vec<RunningProcess> = self.running.drain().map(|(_, h)| h).collect();
        if handles.is_empty() { return; }
        for h in &handles {
            h.cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
            graceful_kill_tree(h.pid);
        }
        let deadline = Instant::now() + Duration::from_millis(EXIT_GRACE_MS);
        for h in handles {
            let dead = loop {
                if let Ok(mut c) = h.child.try_lock() {
                    if matches!(c.try_wait(), Ok(Some(_))) { break true; }
                }
                if Instant::now() >= deadline { break false; }
                thread::sleep(Duration::from_millis(50));
            };
            if !dead {
                force_kill_tree(h.pid);
                if let Ok(mut c) = h.child.try_lock() { let _ = c.kill(); let _ = c.wait(); }
            }
        }
        self.pid_registry.lock().unwrap().clear();
    }

    fn start_all(&mut self) {
        let comps: Vec<Component> = self.config.groups.iter()
            .flat_map(|g| g.components.iter().cloned())
            .filter(|c| !self.running.contains_key(&c.id) && !self.stopping.contains_key(&c.id))
            .collect();
        for c in comps { self.start(&c); }
    }

    fn stop_all(&mut self) {
        let ids: Vec<String> = self.running.keys().cloned().collect();
        for id in ids { self.stop(&id); }
    }

    fn start_group(&mut self, group_id: &str) {
        let comps: Vec<Component> = self.config.groups.iter()
            .find(|g| g.id == group_id)
            .map(|g| g.components.iter()
                .filter(|c| !self.running.contains_key(&c.id) && !self.stopping.contains_key(&c.id))
                .cloned().collect())
            .unwrap_or_default();
        for c in comps { self.start(&c); }
    }

    fn stop_group(&mut self, group_id: &str) {
        let ids: Vec<String> = self.config.groups.iter()
            .find(|g| g.id == group_id)
            .map(|g| g.components.iter()
                .filter(|c| self.running.contains_key(&c.id))
                .map(|c| c.id.clone()).collect())
            .unwrap_or_default();
        for id in ids { self.stop(&id); }
    }

    fn group_running_count(&self, group_id: &str) -> (usize, usize) {
        // returns (running, total)
        let comps = self.config.groups.iter()
            .find(|g| g.id == group_id)
            .map(|g| &g.components[..])
            .unwrap_or(&[]);
        let running = comps.iter().filter(|c| self.running.contains_key(&c.id)).count();
        (running, comps.len())
    }

    fn drain_events(&mut self) {
        while let Ok(ev) = self.event_rx.try_recv() {
            match ev {
                AppEvent::Log { id, line } => {
                    let buf = self.logs.entry(id).or_default();
                    buf.push(line);
                    if buf.len() > 5000 { buf.drain(..500); }
                }
                AppEvent::Status { id, running, exit_code, .. } => {
                    if !running {
                        // Prune the PID so the Ctrl+C handler can never signal a
                        // recycled PID belonging to an unrelated process.
                        if let Some(h) = self.running.remove(&id) {
                            self.pid_registry.lock().unwrap().retain(|&p| p != h.pid);
                        }
                        self.stopping.remove(&id);
                        match exit_code {
                            Some(c) if c != 0 => { self.last_exit.insert(id, c); }
                            _                 => { self.last_exit.remove(&id); }
                        }
                    }
                }
                AppEvent::Stopped { id } => { self.stopping.remove(&id); }
                AppEvent::ShowWindow => { self.show_window_requested = true; }
                AppEvent::QuitApp    => { self.quit_requested        = true; }
            }
        }
    }

    /// Save the config; on failure keep the dirty flag set and surface the error.
    fn persist(&mut self) {
        match save_config(&self.config, &self.config_path) {
            Ok(())  => { self.dirty = false; self.save_error = None; }
            Err(e)  => {
                self.dirty = true;
                self.save_error = Some(format!("Failed to save {}: {}", self.config_path.display(), e));
            }
        }
    }

    // ── Counts ────────────────────────────────────────────────────────────
    fn running_count(&self) -> usize { self.running.len() }
    fn total_count(&self)   -> usize {
        self.config.groups.iter().map(|g| g.components.len()).sum()
    }
}

/// On-disk log writer. Re-resolves the {date}/{name} template on every line so
/// the file rolls over at midnight, and emits each line as a single write_all
/// on an O_APPEND handle so concurrent writers can't interleave mid-line.
struct LogWriter {
    template:       String,
    comp_name:      String,
    base_dir:       PathBuf,
    current_path:   Option<PathBuf>,
    file:           Option<std::fs::File>,
    error_reported: bool,
}

impl LogWriter {
    fn new(template: String, comp_name: String, base_dir: PathBuf) -> Self {
        Self { template, comp_name, base_dir, current_path: None, file: None, error_reported: false }
    }

    /// Write one line. Returns an error message exactly once per failing path
    /// so the caller can surface it in the in-app log instead of dropping it.
    fn write_line(&mut self, src: &str, text: &str) -> Option<String> {
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
const MAX_LINE_BYTES: usize = 16 * 1024;

fn read_lines_capped<R: std::io::Read>(inner: R, mut on_line: impl FnMut(String)) {
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


// ══════════════════════════════════════════════════════════════════════════════
// Log highlighting — tailspin-inspired span pipeline
// Finders produce (start, end, Color32, priority) spans on byte offsets.
// Lower priority wins on overlap. Rendered via egui LayoutJob.
// ══════════════════════════════════════════════════════════════════════════════

#[derive(Clone, Copy)]
struct HSpan { start: usize, end: usize, color: Color32, bold: bool, priority: u8 }

// ── Highlight colours ─────────────────────────────────────────────────────────
const HL_NUMBER:  Color32 = Color32::from_rgb( 86, 210, 255);  // cyan
const HL_STRING:  Color32 = Color32::from_rgb(152, 220,  90);  // green
const HL_URL:     Color32 = Color32::from_rgb( 86, 180, 255);  // light blue
const _HL_KEY:    Color32 = Color32::from_rgb(140, 170, 220);  // steel blue
const HL_UUID:    Color32 = Color32::from_rgb(190, 140, 255);  // purple
const HL_IP:      Color32 = Color32::from_rgb(240, 175,  60);  // amber
const HL_PATH:    Color32 = Color32::from_rgb(170, 170, 170);  // grey
const HL_HTTP_OK: Color32 = Color32::from_rgb( 33, 212, 126);  // green
const HL_HTTP_RD: Color32 = Color32::from_rgb( 86, 210, 255);  // cyan
const HL_HTTP_CL: Color32 = Color32::from_rgb(240, 175,  60);  // amber
const HL_HTTP_ER: Color32 = Color32::from_rgb(240,  74,  94);  // red
const HL_METHOD:  Color32 = Color32::from_rgb( 68, 136, 255);  // blue
const HL_ERR_LVL: Color32 = Color32::from_rgb(240,  74,  94);  // red
const HL_WRN_LVL: Color32 = Color32::from_rgb(240, 175,  60);  // amber
const HL_INF_LVL: Color32 = Color32::from_rgb( 68, 136, 255);  // blue
const HL_DBG_LVL: Color32 = Color32::from_rgb(100, 130, 160);  // dim

fn is_word_boundary(b: &[u8], start: usize, end: usize) -> bool {
    let lb = if start == 0 { true } else { !b[start-1].is_ascii_alphanumeric() && b[start-1] != b'_' };
    let rb = end >= b.len() || (!b[end].is_ascii_alphanumeric() && b[end] != b'_');
    lb && rb
}

fn push(spans: &mut Vec<HSpan>, start: usize, end: usize, color: Color32, bold: bool, prio: u8) {
    if start < end { spans.push(HSpan { start, end, color, bold, priority: prio }); }
}

// ── Numbers ───────────────────────────────────────────────────────────────────
fn find_numbers(text: &str, spans: &mut Vec<HSpan>) {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() {
            // Make sure left boundary is not alphanumeric/underscore
            let lb = i == 0 || (!b[i-1].is_ascii_alphanumeric() && b[i-1] != b'_');
            if lb {
                let start = i;
                while i < b.len() && b[i].is_ascii_digit() { i += 1; }
                // Optional decimal
                if i + 1 < b.len() && b[i] == b'.' && b[i+1].is_ascii_digit() {
                    i += 1;
                    while i < b.len() && b[i].is_ascii_digit() { i += 1; }
                }
                // Right boundary
                let rb = i >= b.len() || (!b[i].is_ascii_alphanumeric() && b[i] != b'_');
                if rb { push(spans, start, i, HL_NUMBER, false, 50); }
                continue;
            }
        }
        i += 1;
    }
}

// ── Quoted strings ────────────────────────────────────────────────────────────
fn find_quoted(text: &str, spans: &mut Vec<HSpan>) {
    let b = text.as_bytes();
    for quote in [b'"', b'\''] {
        let mut open: Option<usize> = None;
        for (i, &c) in b.iter().enumerate() {
            if c != quote { continue; }
            match open {
                // An opening quote must not follow an alphanumeric character —
                // this keeps apostrophes in contractions ("don't") from pairing.
                None => {
                    if i == 0 || !b[i - 1].is_ascii_alphanumeric() {
                        open = Some(i);
                    }
                }
                Some(s) => {
                    push(spans, s, i + 1, HL_STRING, false, 40);
                    open = None;
                }
            }
        }
    }
}

// ── Log levels ────────────────────────────────────────────────────────────────
fn find_log_levels(text: &str, spans: &mut Vec<HSpan>) {
    let b = text.as_bytes();
    let levels: &[(&[u8], Color32, bool)] = &[
        (b"CRITICAL", HL_ERR_LVL, true), (b"FATAL",    HL_ERR_LVL, true),
        (b"ERROR",    HL_ERR_LVL, true), (b"ERR",      HL_ERR_LVL, false),
        (b"WARNING",  HL_WRN_LVL, true), (b"WARN",     HL_WRN_LVL, false),
        (b"INFO",     HL_INF_LVL, false),
        (b"DEBUG",    HL_DBG_LVL, false), (b"DBG",     HL_DBG_LVL, false),
        (b"TRACE",    HL_DBG_LVL, false),
    ];
    for &(kw, color, bold) in levels {
        let mut i = 0;
        while i + kw.len() <= b.len() {
            if b[i..].starts_with(kw) && is_word_boundary(b, i, i + kw.len()) {
                push(spans, i, i + kw.len(), color, bold, 5);
            }
            i += 1;
        }
    }
}

// ── HTTP methods ──────────────────────────────────────────────────────────────
fn find_http_methods(text: &str, spans: &mut Vec<HSpan>) {
    let b = text.as_bytes();
    for kw in [b"GET".as_ref(), b"POST", b"PUT", b"DELETE", b"PATCH", b"HEAD", b"OPTIONS"] {
        let mut i = 0;
        while i + kw.len() <= b.len() {
            if b[i..].starts_with(kw) && is_word_boundary(b, i, i + kw.len()) {
                push(spans, i, i + kw.len(), HL_METHOD, false, 10);
            }
            i += 1;
        }
    }
}

// ── HTTP status codes ─────────────────────────────────────────────────────────
fn find_http_status(text: &str, spans: &mut Vec<HSpan>) {
    let b = text.as_bytes();
    let mut i = 0;
    while i + 3 <= b.len() {
        if b[i].is_ascii_digit() && b[i+1].is_ascii_digit() && b[i+2].is_ascii_digit() {
            let lb = i == 0 || b[i-1] == b' ' || b[i-1] == b'"';
            let rb = i+3 >= b.len() || b[i+3] == b' ' || b[i+3] == b'"' || b[i+3] == b'\r';
            if lb && rb {
                let code = (b[i]-b'0') as u16 * 100
                         + (b[i+1]-b'0') as u16 * 10
                         + (b[i+2]-b'0') as u16;
                let color = match code {
                    200..=299 => HL_HTTP_OK,
                    300..=399 => HL_HTTP_RD,
                    400..=499 => HL_HTTP_CL,
                    500..=599 => HL_HTTP_ER,
                    _ => { i += 1; continue; }
                };
                push(spans, i, i+3, color, false, 8);
            }
        }
        i += 1;
    }
}

// ── URLs ──────────────────────────────────────────────────────────────────────
fn find_urls(text: &str, spans: &mut Vec<HSpan>) {
    let b = text.as_bytes();
    let mut i = 0;
    while i + 8 <= b.len() {
        let is_http  = b[i..].starts_with(b"http://");
        let is_https = b[i..].starts_with(b"https://");
        if is_http || is_https {
            let start = i;
            i += if is_https { 8 } else { 7 };
            while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'"' && b[i] != b'\'' { i += 1; }
            push(spans, start, i, HL_URL, false, 20);
            continue;
        }
        i += 1;
    }
}

// ── UUIDs ─────────────────────────────────────────────────────────────────────
fn find_uuids(text: &str, spans: &mut Vec<HSpan>) {
    let b = text.as_bytes();
    fn is_hex(c: u8) -> bool { c.is_ascii_hexdigit() }
    if b.len() < 36 { return; }
    let mut i = 0;
    while i + 36 <= b.len() {
        // 8-4-4-4-12
        let ok = (0..8).all(|j| is_hex(b[i+j]))
            && b[i+8] == b'-'
            && (0..4).all(|j| is_hex(b[i+9+j]))
            && b[i+13] == b'-'
            && (0..4).all(|j| is_hex(b[i+14+j]))
            && b[i+18] == b'-'
            && (0..4).all(|j| is_hex(b[i+19+j]))
            && b[i+23] == b'-'
            && (0..12).all(|j| is_hex(b[i+24+j]));
        if ok && (i == 0 || !is_hex(b[i-1])) && (i+36 >= b.len() || !is_hex(b[i+36])) {
            push(spans, i, i+36, HL_UUID, false, 15);
            i += 36;
            continue;
        }
        i += 1;
    }
}

// ── IP addresses ──────────────────────────────────────────────────────────────
fn find_ips(text: &str, spans: &mut Vec<HSpan>) {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() {
            let lb = i == 0 || !b[i-1].is_ascii_digit();
            if lb {
                // Try to match d{1,3}.d{1,3}.d{1,3}.d{1,3}
                let start = i;
                let mut j = i;
                let mut valid = true;
                for seg in 0..4 {
                    let seg_start = j;
                    while j < b.len() && b[j].is_ascii_digit() { j += 1; }
                    let seg_len = j - seg_start;
                    if seg_len == 0 || seg_len > 3 { valid = false; break; }
                    // Parse octet value
                    let val: u32 = text[seg_start..j].parse().unwrap_or(999);
                    if val > 255 { valid = false; break; }
                    if seg < 3 {
                        if j >= b.len() || b[j] != b'.' { valid = false; break; }
                        j += 1; // skip dot
                    }
                }
                if valid && (j >= b.len() || !b[j].is_ascii_digit()) {
                    push(spans, start, j, HL_IP, false, 12);
                    i = j;
                    continue;
                }
            }
        }
        i += 1;
    }
}

// ── Unix paths ────────────────────────────────────────────────────────────────
fn find_paths(text: &str, spans: &mut Vec<HSpan>) {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'/' && (i == 0 || b[i-1] == b' ' || b[i-1] == b'"' || b[i-1] == b'\'') {
            let start = i;
            while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'"' && b[i] != b'\'' { i += 1; }
            if i - start > 1 { push(spans, start, i, HL_PATH, false, 30); }
            continue;
        }
        i += 1;
    }
}

// ── Merge: lower priority wins on overlap ────────────────────────────────────
fn merge_spans_hl(text_len: usize, spans: Vec<HSpan>) -> Vec<HSpan> {
    if spans.is_empty() || text_len == 0 { return vec![]; }
    // Per-byte index into spans vec (using index+1 so 0 = unset)
    let mut map: Vec<Option<usize>> = vec![None; text_len];
    for (idx, s) in spans.iter().enumerate() {
        let end = s.end.min(text_len);
        for slot in &mut map[s.start..end] {
            match slot {
                None => *slot = Some(idx),
                Some(existing) if s.priority < spans[*existing].priority => *slot = Some(idx),
                _ => {}
            }
        }
    }
    // Run-length encode into output spans
    let mut out: Vec<HSpan> = Vec::new();
    let mut i = 0;
    while i < text_len {
        if let Some(idx) = map[i] {
            let s = &spans[idx];
            let start = i;
            while i < text_len && map[i] == Some(idx) { i += 1; }
            out.push(HSpan { start, end: i, color: s.color, bold: s.bold, priority: s.priority });
        } else { i += 1; }
    }
    out
}

// ── Build a LayoutJob for one log line ───────────────────────────────────────
fn highlight_log_line(
    text:     &str,
    base_color: Color32,
    font_id:  &egui::FontId,
) -> egui::text::LayoutJob {
    let mut spans: Vec<HSpan> = Vec::new();
    find_log_levels(text, &mut spans);
    find_http_status(text, &mut spans);
    find_http_methods(text, &mut spans);
    find_uuids(text, &mut spans);
    find_ips(text, &mut spans);
    find_urls(text, &mut spans);
    find_paths(text, &mut spans);
    find_quoted(text, &mut spans);
    find_numbers(text, &mut spans);

    let resolved = merge_spans_hl(text.len(), spans);

    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = f32::INFINITY; // no word wrap — horizontal scroll handles it

    let plain_fmt = |color: Color32, _bold: bool| egui::text::TextFormat {
        font_id: font_id.clone(),
        color,
        background: Color32::TRANSPARENT,
        italics: false,
        underline: egui::Stroke::NONE,
        strikethrough: egui::Stroke::NONE,
        valign: egui::Align::BOTTOM,
        ..Default::default()
    };

    let mut pos = 0usize;
    for s in &resolved {
        if s.start > pos {
            job.append(&text[pos..s.start], 0.0, plain_fmt(base_color, false));
        }
        job.append(&text[s.start..s.end], 0.0, plain_fmt(s.color, s.bold));
        pos = s.end;
    }
    if pos < text.len() {
        job.append(&text[pos..], 0.0, plain_fmt(base_color, false));
    }
    if job.sections.is_empty() {
        job.append(text, 0.0, plain_fmt(base_color, false));
    }
    job
}

/// Lines that actually carry error vocabulary — independent of which stream
/// they arrived on. Many healthy servers write access logs to stderr.
fn looks_like_error(text: &str) -> bool {
    let l = text.to_lowercase();
    l.contains("error") || l.contains("exception") || l.contains("traceback")
        || l.contains("fatal") || l.contains("panic") || l.contains("critical")
        || l.starts_with("  file \"")
}

/// Build the full row for one log line: timestamp + stream tag + highlighted
/// text, as ONE LayoutJob. Using ui.horizontal() would split the row into
/// multiple widgets which egui clips to available_width, breaking h-scroll.
///
/// Stream ≠ severity: plain stderr gets a muted lowercase "err" tag; the loud
/// red "ERR" is reserved for lines that actually look like errors.
fn log_line_job(line: &LogLine, font_id: &egui::FontId) -> egui::text::LayoutJob {
    let is_real_error = looks_like_error(&line.text);
    let base_color = if is_real_error { Color32::from_rgb(240, 120, 130) } else { TEXT_PRI };

    let (src_text, src_color) = match line.source {
        Source::Stdout => ("OUT", BLUE),
        Source::Stderr => if is_real_error { ("ERR", RED) } else { ("err", TEXT_MUTED) },
        Source::System => ("SYS", AMBER),
    };

    let dim_fmt = egui::text::TextFormat { font_id: font_id.clone(), color: TEXT_DIM, ..Default::default() };
    let src_fmt = egui::text::TextFormat { font_id: font_id.clone(), color: src_color, ..Default::default() };

    let mut job = highlight_log_line(&line.text, base_color, font_id);
    let text_part = std::mem::take(&mut job.text);
    let sections  = std::mem::take(&mut job.sections);
    let mut full_job = egui::text::LayoutJob::default();
    full_job.wrap.max_width = f32::INFINITY;
    full_job.append(&line.time, 0.0, dim_fmt.clone());
    full_job.append("  ", 0.0, dim_fmt.clone());
    full_job.append(src_text, 0.0, src_fmt);
    full_job.append(" ", 0.0, dim_fmt);
    // Re-add the highlighted text sections at the shifted offset
    let offset = full_job.text.len();
    full_job.text.push_str(&text_part);
    for mut s in sections {
        s.byte_range.start += offset;
        s.byte_range.end   += offset;
        full_job.sections.push(s);
    }
    full_job
}

// ══════════════════════════════════════════════════════════════════════════════
// Rendering
// ══════════════════════════════════════════════════════════════════════════════

impl eframe::App for ProConductor {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.start_minimized {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            self.start_minimized = false;
        }

        self.drain_events();

        // macOS: update dock icon color when status changes
        #[cfg(target_os = "macos")]
        {
            let running = self.running_count();
            let total   = self.total_count();
            let new_status: u8 = if total == 0 || running == 0 { 0 }
                                 else if running < total         { 1 }
                                 else                            { 2 };
            if new_status != self.macos_dock_status {
                self.macos_dock_status = new_status;
                let (rgba, w, h) = match new_status {
                    2 => (DOCK_ICON_GREEN, DOCK_ICON_W, DOCK_ICON_H),
                    1 => (DOCK_ICON_AMBER, DOCK_ICON_W, DOCK_ICON_H),
                    _ => (DOCK_ICON_RED,   DOCK_ICON_W, DOCK_ICON_H),
                };
                set_dock_icon(rgba, w, h);
            }
        }

        // ShowWindow is now done directly via Win32 in the tray thread
        self.show_window_requested = false;
        if self.quit_requested {
            // No stop_all() here — on_exit() does a synchronous shutdown; spawning
            // detached kill threads now would let the process exit out from under them.
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // Windows tray: update icon color, hide window on minimize
        #[cfg(windows)]
        self.handle_tray(ctx);
        // Faster repaint while any process is stopping (progress bar needs it)
        if !self.stopping.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(100));
        } else {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        self.render_topbar(ctx);
        self.render_topbar_controls(ctx);
        #[cfg(any(windows, target_os = "macos"))] self.render_window_controls(ctx);
        self.render_sidebar(ctx);
        self.render_main(ctx);
        self.render_confirm_dialog(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Unsaved config changes must not be silently lost on quit.
        if self.dirty {
            let _ = save_config(&self.config, &self.config_path);
        }
        // Kill all child processes synchronously before the process exits —
        // detached kill threads would die with us and orphan the children.
        // The exclusive file lock (_lock) is released automatically on drop.
        self.shutdown_sync();
    }
}

impl ProConductor {

    // ── Tray (Windows only) ───────────────────────────────────────────────────

    #[cfg(windows)]
    fn handle_tray(&mut self, ctx: &egui::Context) {
        // Update tray icon color
        let running = self.running_count();
        let total   = self.total_count();
        let new_status: u8 = if total == 0 || running == 0 { 0 }
                             else if running < total         { 1 }
                             else                            { 2 };
        if new_status != self.tray_status {
            self.tray_status = new_status;
            if let Some(ref tx) = self.tray_cmd_tx {
                let _ = tx.send(TrayCmd::SetIcon(new_status));
            }
        }

        // First frame: find HWND, remove from taskbar, share with tray thread
        if !self.taskbar_hidden {
            self.taskbar_hidden = true;
            let title: Vec<u16> = ctx.input(|i| i.viewport().title.clone())
                .unwrap_or_default()
                .encode_utf16().chain(std::iter::once(0)).collect();
            unsafe {
                let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
                if hwnd != 0 {
                    // Snapshot rect before touching styles — SWP_FRAMECHANGED triggers
                    // WM_NCCALCSIZE which can collapse a borderless (undecorated) window
                    // to zero size on some Windows x64 configurations.
                    let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                    GetWindowRect(hwnd, &mut rect);
                    let w = rect.right - rect.left;
                    let h = rect.bottom - rect.top;

                    // Remove from taskbar
                    let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                    let ex = (ex & !WS_EX_APPWINDOW) | WS_EX_TOOLWINDOW;
                    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex);
                    SetWindowPos(hwnd, 0, 0, 0, 0, 0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED);

                    // Re-apply the snapshotted size: SWP_FRAMECHANGED can collapse
                    // borderless windows during WM_NCCALCSIZE processing.
                    if w > 0 && h > 0 {
                        SetWindowPos(hwnd, 0, rect.left, rect.top, w, h, SWP_NOZORDER);
                    }

                    // Share HWND with tray thread — it will call ShowWindow directly
                    if let Some(ref tx) = self.tray_cmd_tx {
                        let _ = tx.send(TrayCmd::SetHwnd(hwnd));
                    }
                }
            }
        }

        // Minimize → hide via Win32
        // We skip one frame after hiding (just_hid) to avoid the restore that
        // would otherwise fire when egui processes its own minimize event.
        if self.just_hid {
            self.just_hid = false;
            return;
        }
        let is_minimized = ctx.input(|i| i.viewport().minimized == Some(true));
        if is_minimized {
            let title: Vec<u16> = ctx.input(|i| i.viewport().title.clone())
                .unwrap_or_default()
                .encode_utf16().chain(std::iter::once(0)).collect();
            unsafe {
                let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
                if hwnd != 0 { ShowWindow(hwnd, SW_HIDE); }
            }
            self.just_hid = true;
            // Do NOT send Minimized(false) — that restores the window immediately
        }
    }

    // ── Topbar ─────────────────────────────────────────────────────────────

    fn render_topbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("topbar")
            .exact_height(48.0)
            .frame(egui::Frame::none().fill(BG_PANEL).inner_margin(egui::Margin::symmetric(14.0, 0.0)))
            .show(ctx, |ui| {
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.visuals_mut().override_text_color = Some(TEXT_PRI);

                    // Logo
                    egui::Frame::none()
                        .fill(BLUE)
                        .rounding(4.0)
                        .inner_margin(egui::Margin::symmetric(7.0, 4.0))
                        .show(ui, |ui| {
                            ui.label(RichText::new("▶").size(13.0).color(Color32::WHITE).strong());
                        });
                    ui.label(RichText::new("ProConductor").size(13.0).strong().color(TEXT_PRI));

                    // Config file name
                    let fname = self.config_path.file_name()
                        .unwrap_or_default().to_string_lossy();
                    ui.label(RichText::new(format!("/ {}", fname)).size(11.0).color(TEXT_MUTED));

                    if self.dirty {
                        ui.label(RichText::new("●").color(AMBER).size(10.0));
                    }

                    // Right-side controls are rendered as a floating Area in render_topbar_controls()
                    // to avoid right-to-left layout distorting hit rects.
                });

                // Drag by dragging empty topbar space.
                // Area::Foreground for window controls takes priority so no conflict.
                #[cfg(any(windows, target_os = "macos"))]
                {
                    let topbar_resp = ui.interact(
                        ui.min_rect(),
                        ui.id().with("topbar_drag"),
                        egui::Sense::drag(),
                    );
                    if topbar_resp.drag_started() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                }
            });
    }

    // ── Topbar right-side controls ────────────────────────────────────────

    fn render_topbar_controls(&mut self, ctx: &egui::Context) {
        // Reserve room for window controls on Windows/macOS
        let right_margin: f32 = if cfg!(windows) || cfg!(target_os = "macos") { 92.0 } else { 8.0 };

        // NOTE: Area::anchor() can't be used here — it aligns within
        // ctx.available_rect(), which already excludes the topbar panel, so
        // RIGHT_TOP would land BELOW the topbar, overlapping the dashboard.
        // Position absolutely instead, right-aligned via the area's own width
        // as measured last frame (no hardcoded width estimate).
        let area_id = egui::Id::new("topbar_controls");
        let last_w = ctx.memory(|m| m.area_rect(area_id).map(|r| r.width())).unwrap_or(420.0);
        let x = ctx.screen_rect().width() - right_margin - last_w;

        egui::Area::new(area_id)
            .fixed_pos(egui::pos2(x, 0.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_height(48.0);
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;

                    // Save button — amber: "unsaved changes need attention"
                    if self.dirty {
                        let save_btn = egui::Button::new(
                            RichText::new("💾 Save").size(11.0).color(AMBER)
                        ).fill(AMBER_BG).stroke(Stroke::new(1.0, AMBER_DIM));
                        if ui.add(save_btn).clicked() {
                            self.persist();
                        }
                    }

                    // Status pill
                    let run = self.running_count();
                    let tot = self.total_count();
                    egui::Frame::none()
                        .fill(BG_CARD).rounding(12.0)
                        .stroke(Stroke::new(1.0, BORDER))
                        .inner_margin(egui::Margin::symmetric(10.0, 4.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("●").color(GREEN).size(9.0));
                                ui.label(RichText::new(format!("{}", run)).size(11.0).color(TEXT_PRI));
                                ui.label(RichText::new("/").size(11.0).color(TEXT_DIM));
                                ui.label(RichText::new(format!("{}", tot)).size(11.0).color(TEXT_MUTED));
                            });
                        });

                    // Start All
                    let all_running = run == tot && tot > 0;
                    if action_button(ui, BtnKind::Positive, "▶  Start All", 12.0, 0.0, !all_running && tot > 0)
                        .clicked() {
                        self.start_all();
                    }

                    // Stop All — amber: interrupting is routine, red stays
                    // reserved for destructive actions
                    let all_stopped = run == 0;
                    if action_button(ui, BtnKind::Caution, "■  Stop All", 12.0, 0.0, !all_stopped)
                        .clicked() {
                        self.stop_all();
                    }
                });
            });
    }

    // ── Window controls overlay (Windows only) ────────────────────────────

    #[cfg(any(windows, target_os = "macos"))]
    fn render_window_controls(&self, ctx: &egui::Context) {
        // Render close and minimize as a floating Area pinned to top-right.
        // This avoids right-to-left layout distorting the hit rects.
        let screen_w = ctx.screen_rect().width();
        egui::Area::new(egui::Id::new("win_controls"))
            .fixed_pos(egui::pos2(screen_w - 84.0, 4.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;

                    // Minimize
                    let min = ui.add(
                        egui::Button::new(RichText::new("  _  ").size(12.0).color(TEXT_SEC))
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::NONE)
                            .min_size(Vec2::new(40.0, 40.0))
                    );
                    if min.hovered() {
                        ui.painter().rect_filled(min.rect, 2.0, BG_HOVER);
                        // Redraw glyph on top of highlight so it stays visible
                        ui.painter().text(min.rect.center(), egui::Align2::CENTER_CENTER,
                            "_", egui::FontId::proportional(12.0), TEXT_PRI);
                    }
                    if min.clicked() { ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)); }

                    // Close
                    let close = ui.add(
                        egui::Button::new(RichText::new("  X  ").size(12.0).color(TEXT_SEC))
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::NONE)
                            .min_size(Vec2::new(40.0, 40.0))
                    );
                    if close.hovered() {
                        ui.painter().rect_filled(close.rect, 2.0, RED_DIM);
                        // Redraw text on top of highlight so it stays visible
                        ui.painter().text(close.rect.center(), egui::Align2::CENTER_CENTER,
                            "X", egui::FontId::proportional(12.0), TEXT_PRI);
                    }
                    if close.clicked() { ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
                });
            });
    }

    // ── Sidebar ────────────────────────────────────────────────────────────

    fn render_sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("sidebar")
            .default_width(240.0)
            .width_range(180.0..=340.0)
            .frame(egui::Frame::none().fill(BG_PANEL)
                .stroke(Stroke::new(1.0, BORDER))
                .inner_margin(egui::Margin::same(0.0)))
            .show(ctx, |ui| {
                // Header
                egui::Frame::none()
                    .fill(BG_PANEL)
                    .stroke(Stroke::new(1.0, BORDER))
                    .inner_margin(egui::Margin { left: 14.0, right: 8.0, top: 9.0, bottom: 9.0 })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("APPLICATIONS").size(10.0).color(TEXT_MUTED).strong());
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.small_button(RichText::new("+").size(14.0).color(TEXT_SEC)).on_hover_text("New application group").clicked() {
                                    let gid = Uuid::new_v4().to_string();
                                    self.config.groups.push(Group {
                                        id: gid.clone(), name: "New Application".into(), components: vec![],
                                    });
                                    self.open_groups.insert(gid.clone());
                                    self.selected_group = Some(gid);
                                    self.dirty = true;
                                }
                            });
                        });
                    });

                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.set_width(ui.available_width());

                    if self.config.groups.is_empty() {
                        ui.add_space(20.0);
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new("No groups yet.").color(TEXT_MUTED).size(11.0));
                            ui.label(RichText::new("Click + to create one.").color(TEXT_DIM).size(10.0));
                        });
                    }

                    let group_ids: Vec<String> = self.config.groups.iter().map(|g| g.id.clone()).collect();
                    for gid in group_ids {
                        self.render_sidebar_group(ui, &gid.clone());
                    }

                    ui.add_space(8.0);
                });

                // Footer
                ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                    egui::Frame::none()
                        .fill(BG_PANEL)
                        .stroke(Stroke::new(1.0, BORDER))
                        .inner_margin(egui::Margin::same(10.0))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            let has_group = self.selected_group.is_some();
                            let btn = egui::Button::new(
                                RichText::new("+ Add Component").size(11.0)
                                    .color(if has_group { TEXT_SEC } else { TEXT_DIM })
                            ).fill(if has_group { BG_CARD } else { BG_PANEL })
                             .stroke(Stroke::new(1.0, if has_group { BORDER_HI } else { BORDER }))
                             .min_size(Vec2::new(ui.available_width() - 4.0, 0.0));
                            if ui.add_enabled(has_group, btn).clicked() {
                                if let Some(gid) = self.selected_group.clone() {
                                    let new_comp = Component::new();
                                    self.edit = Some(EditState {
                                        component: new_comp, group_id: gid, is_new: true,
                                    });
                                    self.view = MainView::Edit;
                                }
                            }
                        });
                });
            });
    }

    fn render_sidebar_group(&mut self, ui: &mut egui::Ui, gid: &str) {
        let group_idx = match self.config.groups.iter().position(|g| g.id == gid) {
            Some(i) => i, None => return,
        };
        let group_name = self.config.groups[group_idx].name.clone();
        let comp_count = self.config.groups[group_idx].components.len();
        let is_open    = self.open_groups.contains(gid);
        let is_sel_grp = self.selected_group.as_deref() == Some(gid);

        // Group header row
        let header_fill = if is_sel_grp { BG_SEL } else { Color32::TRANSPARENT };
        egui::Frame::none().fill(header_fill).inner_margin(egui::Margin { left: 8.0, right: 6.0, top: 4.0, bottom: 4.0 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let arrow = if is_open { "▾" } else { "▸" };
                if ui.add(egui::Label::new(
                    RichText::new(arrow).size(11.0).color(TEXT_MUTED)
                ).sense(egui::Sense::click())).clicked() {
                    if is_open { self.open_groups.remove(gid); }
                    else       { self.open_groups.insert(gid.to_string()); }
                }

                // Editable group name
                let mut name_buf = group_name.clone();
                let te = egui::TextEdit::singleline(&mut name_buf)
                    .font(egui::TextStyle::Body)
                    .desired_width(120.0)
                    .frame(false)
                    .text_color(if is_sel_grp { TEXT_PRI } else { TEXT_SEC });
                if ui.add(te).changed() {
                    self.config.groups[group_idx].name = name_buf;
                    self.dirty = true;
                }
                if ui.add(egui::Label::new(RichText::new(format!("{}", comp_count)).size(10.0).color(TEXT_DIM)).sense(egui::Sense::click())).clicked() {
                    self.selected_group = Some(gid.to_string());
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button(RichText::new("✕").size(9.0).color(TEXT_DIM)).on_hover_text("Delete group").clicked() {
                        self.confirm_delete = Some(("group".into(), gid.to_string()));
                    }
                });
            });
        });

        if !is_open { return; }

        let comp_ids: Vec<String> = self.config.groups[group_idx].components.iter().map(|c| c.id.clone()).collect();
        for cid in comp_ids {
            self.render_sidebar_component(ui, &cid.clone());
        }
    }

    fn render_sidebar_component(&mut self, ui: &mut egui::Ui, cid: &str) {
        let comp = self.config.groups.iter()
            .flat_map(|g| g.components.iter())
            .find(|c| c.id == cid)
            .cloned();
        let comp = match comp { Some(c) => c, None => return };

        let is_running = self.running.contains_key(cid);
        let is_sel     = self.selected_comp.as_deref() == Some(cid);
        let dot_color  = if is_running { GREEN } else { TEXT_DIM };

        let bg = if is_sel { BG_SEL } else { Color32::TRANSPARENT };
        let response = egui::Frame::none()
            .fill(bg)
            .stroke(Stroke::new(if is_sel { 1.0 } else { 0.0 }, if is_sel { BLUE } else { Color32::TRANSPARENT }))
            .inner_margin(egui::Margin { left: 28.0, right: 8.0, top: 4.0, bottom: 4.0 })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    // Status dot
                    let (resp, painter) = ui.allocate_painter(Vec2::splat(10.0), egui::Sense::hover());
                    let c = resp.rect.center();
                    painter.circle_filled(c, 4.0, dot_color);
                    if is_running { painter.circle_stroke(c, 5.5, Stroke::new(1.0, Color32::from_rgba_premultiplied(33, 212, 126, 60))); }

                    ui.label(RichText::new(&comp.name).size(12.0)
                        .color(if is_sel { TEXT_PRI } else { TEXT_SEC }));
                });
            }).response;

        if ui.interact(response.rect, ui.id().with(cid), egui::Sense::click()).clicked() {
            self.selected_comp  = Some(cid.to_string());
            let gid = self.config.groups.iter()
                .find(|g| g.components.iter().any(|c| c.id == cid))
                .map(|g| g.id.clone());
            self.selected_group = gid;
            // Always go back to Dashboard when switching components
            self.view = MainView::Dashboard;
            self.edit = None;
        }
    }

    // ── Main panel ─────────────────────────────────────────────────────────

    /// Dismissible error banners (config load / save failures) — shown above
    /// every view so a failed save can't go unnoticed.
    fn render_banners(&mut self, ui: &mut egui::Ui) {
        let mut dismiss_load = false;
        let mut dismiss_save = false;
        for (msg, is_load) in [(&self.config_load_error, true), (&self.save_error, false)] {
            let Some(text) = msg else { continue };
            egui::Frame::none()
                .fill(RED_BG)
                .stroke(Stroke::new(1.0, RED_DIM))
                .inner_margin(egui::Margin::symmetric(12.0, 6.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("⚠").size(12.0).color(RED));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.small_button(RichText::new("✕").size(10.0).color(TEXT_SEC))
                                .on_hover_text("Dismiss").clicked() {
                                if is_load { dismiss_load = true; } else { dismiss_save = true; }
                            }
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                ui.label(RichText::new(text).size(11.0).color(TEXT_PRI));
                            });
                        });
                    });
                });
        }
        if dismiss_load { self.config_load_error = None; }
        if dismiss_save { self.save_error = None; }
    }

    fn render_main(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG_BASE))
            .show(ctx, |ui| {
                self.render_banners(ui);
                // Dashboard is the root view.
                // Log and Configure are full-screen overlays with their own ← Back header.
                if let Some(edit) = self.edit.clone() {
                    self.render_edit_view(ui, edit);
                } else if self.view == MainView::Log {
                    self.render_log_view(ui);
                } else {
                    self.view = MainView::Dashboard;
                    self.render_dashboard(ui);
                }
            });
    }

    // ── Dashboard ──────────────────────────────────────────────────────────

    fn render_dashboard(&mut self, ui: &mut egui::Ui) {
        if self.config.groups.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(80.0);
                ui.label(RichText::new("⚙").size(40.0).color(TEXT_DIM));
                ui.add_space(12.0);
                ui.label(RichText::new("Welcome to ProConductor").size(18.0).color(TEXT_SEC));
                ui.add_space(6.0);
                ui.label(RichText::new("Create a group in the sidebar, then add\ncomponents to orchestrate your application.").size(12.0).color(TEXT_MUTED));
                ui.add_space(16.0);
                if ui.button(RichText::new("+ Create First Group").size(12.0).color(BLUE)).clicked() {
                    let gid = Uuid::new_v4().to_string();
                    self.config.groups.push(Group { id: gid.clone(), name: "My App".into(), components: vec![] });
                    self.open_groups.insert(gid.clone());
                    self.selected_group = Some(gid);
                    self.dirty = true;
                }
            });
            return;
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(16.0);
            let groups: Vec<(String, String, Vec<String>)> = self.config.groups.iter()
                .map(|g| (g.id.clone(), g.name.clone(), g.components.iter().map(|c| c.id.clone()).collect()))
                .collect();

            for (gid, gname, comp_ids) in groups {
                ui.add_space(4.0);
                egui::Frame::none()
                    .inner_margin(egui::Margin { left: 20.0, right: 20.0, top: 0.0, bottom: 0.0 })
                    .show(ui, |ui| {
                        // Group header with per-group start/stop
                        let (g_running, g_total) = self.group_running_count(&gid);
                        let mut do_start_group = false;
                        let mut do_stop_group  = false;

                        egui::Frame::none()
                            .fill(BG_RAISED)
                            .rounding(6.0)
                            .stroke(Stroke::new(1.0, BORDER))
                            .inner_margin(egui::Margin { left: 12.0, right: 8.0, top: 7.0, bottom: 7.0 })
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    // Status dot for group
                                    let (r, painter) = ui.allocate_painter(Vec2::splat(10.0), egui::Sense::hover());
                                    let c = r.rect.center();
                                    let gc = if g_running == g_total && g_total > 0 { GREEN }
                                             else if g_running > 0 { AMBER }
                                             else { TEXT_DIM };
                                    painter.circle_filled(c, 4.0, gc);

                                    ui.label(RichText::new(&gname).size(12.0).color(TEXT_PRI).strong());

                                    // Running count badge
                                    egui::Frame::none()
                                        .fill(BG_BASE).rounding(10.0)
                                        .stroke(Stroke::new(1.0, BORDER))
                                        .inner_margin(egui::Margin::symmetric(7.0, 2.0))
                                        .show(ui, |ui| {
                                            ui.label(RichText::new(format!("{}/{}", g_running, g_total))
                                                .size(10.0).color(if g_running > 0 { GREEN } else { TEXT_MUTED })
                                                .monospace());
                                        });

                                    // Group start/stop buttons — right side, added after fixed-size left content
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.spacing_mut().button_padding = Vec2::new(8.0, 4.0);
                                        let all_stopped = g_running == 0;
                                        let all_running = g_running == g_total && g_total > 0;

                                        if action_button(ui, BtnKind::Caution, "Stop group", 11.0, 0.0, !all_stopped)
                                            .clicked() { do_stop_group = true; }

                                        if action_button(ui, BtnKind::Positive, "Start group", 11.0, 0.0, !all_running)
                                            .clicked() { do_start_group = true; }
                                    });
                                });
                            });

                        if do_start_group { self.start_group(&gid); }
                        if do_stop_group  { self.stop_group(&gid); }

                        ui.add_space(6.0);

                        if comp_ids.is_empty() {
                            ui.label(RichText::new("No components. Add one using the sidebar.").size(11.0).color(TEXT_DIM));
                        }

                        for cid in comp_ids {
                            self.render_component_card(ui, &cid.clone());
                            ui.add_space(4.0);
                        }
                    });
                ui.add_space(12.0);
            }
        });
    }

    fn render_component_card(&mut self, ui: &mut egui::Ui, cid: &str) {
        let comp = match self.config.groups.iter().flat_map(|g| g.components.iter()).find(|c| c.id == cid).cloned() {
            Some(c) => c, None => return,
        };
        let is_running   = self.running.contains_key(cid);
        let is_stopping  = self.stopping.contains_key(cid);
        let stop_started = self.stopping.get(cid).copied();
        let handle_info  = self.running.get(cid).map(|h| (h.pid, h.started_at));
        let is_sel       = self.selected_comp.as_deref() == Some(cid);
        let crash_code   = (!is_running && !is_stopping)
            .then(|| self.last_exit.get(cid).copied()).flatten();
        let border_color = if is_stopping { AMBER_DIM }
                           else if is_running { GREEN_DIM }
                           else if crash_code.is_some() { RED_DIM }
                           else { BORDER };
        let bg           = if is_sel { BG_SEL } else { BG_CARD };

        let mut do_start     = false;
        let mut do_stop      = false;
        let mut do_log       = false;
        let mut do_configure = false;
        let mut do_delete    = false;
        let mut do_open_log  = false;

        egui::Frame::none()
            .fill(bg)
            .rounding(6.0)
            .stroke(Stroke::new(1.0, border_color))
            .inner_margin(egui::Margin::symmetric(14.0, 10.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());

                // ── In egui, right-to-left widgets must be added BEFORE left-to-right
                // ones in the same horizontal strip, otherwise the left widget expands
                // to fill all space and the button rects end up with zero effective area.
                // Strategy: reserve button space first, then fill name/meta on the left.

                // Measure available width and reserve the right portion for buttons
                let total_w = ui.available_width();

                ui.horizontal(|ui| {
                    // LEFT: status dot + name/meta — does NOT use remaining width greedy
                    let (r, painter) = ui.allocate_painter(Vec2::splat(12.0), egui::Sense::hover());
                    let center = r.rect.center();
                    let dot_color = if is_stopping { AMBER }
                                    else if is_running { GREEN }
                                    else if crash_code.is_some() { RED }
                                    else { TEXT_DIM };
                    painter.circle_filled(center, 5.0, dot_color);
                    if is_running || is_stopping || crash_code.is_some() {
                        let glow = if is_stopping { Color32::from_rgba_premultiplied(240,160,48,60) }
                                   else if is_running { Color32::from_rgba_premultiplied(33,212,126,60) }
                                   else { Color32::from_rgba_premultiplied(240,74,94,60) };
                        painter.circle_stroke(center, 7.0, Stroke::new(1.0, glow));
                    }

                    // Name/meta column with explicit max width so buttons get space
                    let left_w = (total_w * 0.55).max(160.0);
                    ui.allocate_ui(Vec2::new(left_w, 0.0), |ui| {
                        ui.vertical(|ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(&comp.name).size(13.0).strong().color(TEXT_PRI));
                                if is_stopping {
                                    let grace_secs = GRACE_PERIOD_MS as f32 / 1000.0;
                                    let elapsed = stop_started.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0);
                                    let progress = (elapsed / grace_secs).min(1.0);
                                    egui::Frame::none().fill(AMBER_BG).rounding(4.0)
                                        .stroke(Stroke::new(1.0, AMBER_DIM))
                                        .inner_margin(egui::Margin::symmetric(6.0, 2.0))
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.label(RichText::new("Stopping").size(9.0).color(AMBER).strong());
                                                // Progress bar
                                                let (bar_rect, _) = ui.allocate_exact_size(
                                                    Vec2::new(48.0, 5.0), egui::Sense::hover());
                                                let painter = ui.painter();
                                                painter.rect_filled(bar_rect, 2.0, AMBER_DIM);
                                                let mut fill = bar_rect;
                                                fill.set_right(bar_rect.left() + bar_rect.width() * progress);
                                                painter.rect_filled(fill, 2.0, AMBER);
                                                let secs_left = (grace_secs - elapsed).max(0.0).ceil() as u32;
                                                ui.label(RichText::new(format!("{}s", secs_left)).size(9.0).color(AMBER_DIM));
                                            });
                                        });
                                } else if is_running {
                                    egui::Frame::none().fill(GREEN_BG).rounding(4.0)
                                        .stroke(Stroke::new(1.0, GREEN_DIM))
                                        .inner_margin(egui::Margin::symmetric(6.0, 2.0))
                                        .show(ui, |ui| {
                                            ui.label(RichText::new("RUNNING").size(9.0).color(GREEN).strong());
                                        });
                                    if let Some((pid, started)) = handle_info {
                                        ui.label(RichText::new(format!("PID {}", pid)).size(9.0).color(TEXT_MUTED).monospace());
                                        ui.label(RichText::new(format_uptime(started)).size(10.0).color(GREEN));
                                    }
                                } else if let Some(code) = crash_code {
                                    // Nonzero exit — visibly different from a clean stop
                                    egui::Frame::none().fill(RED_BG).rounding(4.0)
                                        .stroke(Stroke::new(1.0, RED_DIM))
                                        .inner_margin(egui::Margin::symmetric(6.0, 2.0))
                                        .show(ui, |ui| {
                                            let what = if code < 0 { format!("CRASHED (signal {})", -code) }
                                                       else        { format!("CRASHED (code {})", code) };
                                            ui.label(RichText::new(what).size(9.0).color(RED).strong());
                                        });
                                } else {
                                    egui::Frame::none().fill(BG_BASE).rounding(4.0)
                                        .stroke(Stroke::new(1.0, BORDER))
                                        .inner_margin(egui::Margin::symmetric(6.0, 2.0))
                                        .show(ui, |ui| {
                                            ui.label(RichText::new("STOPPED").size(9.0).color(TEXT_DIM));
                                        });
                                }
                            });
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 12.0;
                                if !comp.executable.is_empty() {
                                    let exe = comp.executable.rsplit(&['/', '\\'][..]).next().unwrap_or(&comp.executable);
                                    meta_item(ui, "exe", exe);
                                }
                                if !comp.working_dir.is_empty() {
                                    let dir = comp.working_dir.rsplit(&['/', '\\'][..]).next().unwrap_or(&comp.working_dir);
                                    meta_item(ui, "dir", dir);
                                }
                                if !comp.args.is_empty() {
                                    // Truncate on char boundaries — a byte slice
                                    // panics on multibyte args
                                    let short = if comp.args.chars().count() > 36 {
                                        format!("{}…", comp.args.chars().take(36).collect::<String>())
                                    } else { comp.args.clone() };
                                    meta_item(ui, "args", &short);
                                }
                            });
                        });
                    });

                    // RIGHT: buttons — added into remaining space with right-to-left layout
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().button_padding = Vec2::new(8.0, 5.0);

                        // Delete — ghost-red until hover (rare destructive action,
                        // not a permanent alarm), disabled while running, confirms.
                        if action_button(ui, BtnKind::Destructive, "Delete", 11.0, 0.0, !is_running)
                            .on_hover_text("Delete component")
                            .on_disabled_hover_text("Stop the process first")
                            .clicked() { do_delete = true; }

                        // Dead zone so a slip on Config can't land on Delete
                        ui.add_space(10.0);

                        // Configure — enabled neutral, not the disabled-looking gray
                        if action_button(ui, BtnKind::Neutral, "Config", 11.0, 0.0, true)
                            .on_hover_text("Configure component").clicked() { do_configure = true; }

                        // Open log file — resolved path
                        if !comp.log_path.is_empty() {
                            let base_dir = self.config_path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
                            let resolved = resolve_log_path(&comp.log_path, &comp.name, &base_dir)
                                .map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
                            if action_button(ui, BtnKind::Neutral, "Log file", 11.0, 0.0, true)
                                .on_hover_text(&resolved).clicked() { do_open_log = true; }
                        }

                        // View logs in-app — outline accent (navigation, not the
                        // primary action); fixed width so the live count can't
                        // shift the row under the cursor.
                        let log_count = self.logs.get(cid).map(|l| l.len()).unwrap_or(0);
                        let log_lbl = if log_count > 0 { format!("Logs ({})", log_count) } else { "Logs".into() };
                        if action_button(ui, BtnKind::Accent, &log_lbl, 11.0, 86.0, true)
                            .on_hover_text("View live log output").clicked() { do_log = true; }

                        // Start / Stop / Stopping… — one fixed-width slot.
                        // Stop is amber (interrupt), never Delete's red.
                        if is_stopping {
                            ui.add_enabled(false, egui::Button::new(
                                RichText::new("Stopping…").size(11.0).color(AMBER))
                                .fill(AMBER_BG).stroke(Stroke::new(1.0, AMBER_DIM))
                                .min_size(Vec2::new(72.0, 0.0)))
                                .on_disabled_hover_text("Waiting for the process to exit");
                        } else if is_running {
                            // Brief disable right after start so a double-click on
                            // Start can't land on the freshly-rendered Stop
                            let just_started = handle_info
                                .map(|(_, s)| s.elapsed() < Duration::from_millis(600))
                                .unwrap_or(false);
                            if action_button(ui, BtnKind::Caution, "Stop", 11.0, 72.0, !just_started)
                                .on_disabled_hover_text("Just started…")
                                .clicked() { do_stop = true; }
                        } else if action_button(ui, BtnKind::Positive, "Start", 11.0, 72.0, true)
                            .clicked() { do_start = true; }
                    });
                });
            });

        if do_start     { self.start(&comp); }
        if do_stop      { self.stop(cid); }
        if do_log       { self.selected_comp = Some(cid.to_string()); self.view = MainView::Log; }
        if do_open_log  {
            let base_dir = self.config_path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
            if let Some(resolved) = resolve_log_path(&comp.log_path, &comp.name, &base_dir) {
                open_path(&resolved.to_string_lossy());
            }
        }
        if do_configure {
            let gid = self.config.groups.iter()
                .find(|g| g.components.iter().any(|c| c.id == cid))
                .map(|g| g.id.clone()).unwrap_or_default();
            self.edit = Some(EditState { component: comp.clone(), group_id: gid, is_new: false });
            self.view = MainView::Edit;
        }
        if do_delete    { self.confirm_delete = Some(("component".into(), cid.to_string())); }
    }
    // ── Log view ───────────────────────────────────────────────────────────

    fn render_log_view(&mut self, ui: &mut egui::Ui) {
        let comp = match self.selected_comp.as_ref().and_then(|id| {
            self.config.groups.iter().flat_map(|g| g.components.iter()).find(|c| c.id == *id).cloned()
        }) { Some(c) => c, None => { self.view = MainView::Dashboard; self.render_dashboard(ui); return; } };

        let log_count = self.logs.get(&comp.id).map(|l| l.len()).unwrap_or(0);

        let base_dir = self.config_path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
        let resolved_log = resolve_log_path(&comp.log_path, &comp.name, &base_dir)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        let mut do_back   = false;
        let mut do_clear  = false;
        let mut do_open   = false;
        let mut do_reveal = false;

        // Breadcrumb header — matches style of Configure header
        egui::Frame::none()
            .fill(BG_PANEL)
            .stroke(Stroke::new(1.0, BORDER))
            .inner_margin(egui::Margin::symmetric(14.0, 9.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    if ui.small_button(RichText::new("← Dashboard").size(11.0).color(TEXT_SEC)).clicked() {
                        do_back = true;
                    }
                    // Right: log actions — added before expanding content
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !resolved_log.is_empty() {
                            if ui.small_button(RichText::new("📁 Reveal").size(11.0)).on_hover_text(&resolved_log).clicked() {
                                do_reveal = true;
                            }
                            if ui.small_button(RichText::new("📄 Open").size(11.0)).on_hover_text(&resolved_log).clicked() {
                                do_open = true;
                            }
                        }
                        // Clear is destructive (drops the in-app buffer) — ghost-red,
                        // never the disabled-looking gray it had before.
                        if action_button(ui, BtnKind::Destructive, "Clear", 11.0, 0.0, log_count > 0)
                            .on_hover_text("Clear the in-app log buffer (the log file on disk is kept)")
                            .clicked() {
                            do_clear = true;
                        }
                        ui.checkbox(&mut self.log_autoscroll, RichText::new("Auto-scroll").size(11.0).color(TEXT_SEC));
                        ui.add(egui::TextEdit::singleline(&mut self.log_filter)
                            .hint_text("Filter…")
                            .desired_width(150.0)
                            .font(egui::TextStyle::Monospace));
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.label(RichText::new(&comp.name).size(12.0).color(TEXT_PRI).strong());
                            ui.label(RichText::new("— Logs").size(11.0).color(TEXT_MUTED));
                            ui.label(RichText::new(format!("({} lines)", log_count)).size(10.0).color(TEXT_DIM));
                        });
                    });
                });
            });

        if do_back   { self.view = MainView::Dashboard; return; }
        if do_clear  { self.logs.remove(&comp.id); }
        if do_open   { open_path(&resolved_log); }
        if do_reveal { reveal_path(&resolved_log); }

        // Log output — both axes scrollable, syntax-highlighted via span pipeline
        let font_id = egui::FontId::new(11.5, egui::FontFamily::Monospace);

        let filter = self.log_filter.to_lowercase();
        let empty: Vec<LogLine> = Vec::new();
        let logs = self.logs.get(&comp.id).unwrap_or(&empty);
        let filtered: Vec<&LogLine> = logs.iter()
            .filter(|l| filter.is_empty() || l.text.to_lowercase().contains(&filter))
            .collect();

        if filtered.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                let msg = if logs.is_empty() { "No output yet — start the component to see logs." }
                          else                { "No lines match the filter." };
                ui.label(RichText::new(msg).size(11.0).color(TEXT_DIM).monospace());
            });
            return;
        }

        // Content width: widest line in the buffer (monospace), not a hardcoded
        // 4096px — no permanent horizontal scrollbar on short logs.
        let char_w = ui.fonts(|f| f.glyph_width(&font_id, 'M'));
        let prefix_chars = 8 + 2 + 3 + 1; // "HH:MM:SS" + gap + tag + gap
        let max_chars = filtered.iter().map(|l| l.text.chars().count()).max().unwrap_or(0) + prefix_chars;
        let content_w = (max_chars as f32) * char_w + 24.0;
        let row_h = ui.fonts(|f| f.row_height(&font_id));

        // Virtualized: only visible rows are highlighted and laid out, instead
        // of re-rendering the whole 5000-line buffer every frame.
        egui::ScrollArea::both()
            .stick_to_bottom(self.log_autoscroll)
            .auto_shrink([false; 2])
            .show_rows(ui, row_h, filtered.len(), |ui, range| {
                ui.set_min_width(ui.available_width().max(content_w));
                ui.style_mut().wrap = Some(false);
                for line in &filtered[range] {
                    ui.label(log_line_job(line, &font_id));
                }
            });
    }

    // ── Edit view ──────────────────────────────────────────────────────────

    fn render_edit_view(&mut self, ui: &mut egui::Ui, mut edit: EditState) {
        let mut do_back   = false;
        let mut do_save   = false;
        let mut do_delete = false;

        // Header — buttons use fixed width so Back isn't squeezed to zero
        egui::Frame::none()
            .fill(BG_PANEL)
            .stroke(Stroke::new(1.0, BORDER))
            .inner_margin(egui::Margin::symmetric(14.0, 9.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    // Back — added first so it gets a real rect before right-side layout claims space
                    if ui.small_button(RichText::new("← Dashboard").size(11.0).color(TEXT_SEC)).clicked() {
                        do_back = true;
                    }
                    // Right-side buttons — must be added before the expanding label
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !edit.is_new
                            && action_button(ui, BtnKind::Destructive, "Delete", 12.0, 0.0, true).clicked() {
                            do_delete = true;
                        }
                        if action_button(ui, BtnKind::Positive, "Save", 12.0, 0.0, true).clicked() {
                            do_save = true;
                        }
                        // Title fills leftover space between Back and the right buttons
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.label(RichText::new(if edit.is_new { "New Component" } else { "Configure Component" })
                                .size(13.0).strong().color(TEXT_PRI));
                        });
                    });
                });
            });

        // Apply header actions before rendering the scroll area
        if do_back {
            self.edit = None;
            self.view = MainView::Dashboard;
            return;
        }
        if do_delete {
            self.confirm_delete = Some(("component".into(), edit.component.id.clone()));
            self.edit = None;
            return;
        }
        if do_save {
            let c = edit.component.clone();
            if edit.is_new {
                if let Some(g) = self.config.groups.iter_mut().find(|g| g.id == edit.group_id) {
                    g.components.push(c.clone());
                }
                self.selected_comp = Some(c.id.clone());
            } else {
                for g in &mut self.config.groups {
                    if let Some(slot) = g.components.iter_mut().find(|x| x.id == c.id) {
                        *slot = c.clone();
                    }
                }
            }
            self.persist();
            self.edit  = None;
            self.view  = MainView::Dashboard;
            return;
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(12.0);
            // NOTE: edits here go into the edit BUFFER (a clone), not the live
            // config — so they must NOT set self.dirty. The config only changes
            // on Save, which persists immediately.
            let c = &mut edit.component;

            egui::Frame::none()
                .inner_margin(egui::Margin::symmetric(24.0, 0.0))
                .show(ui, |ui| {
                    // Identity
                    section_title(ui, "IDENTITY");
                    ui.add_space(4.0);
                    field_row(ui, "Display Name", |ui| {
                        ui.add(egui::TextEdit::singleline(&mut c.name).hint_text("e.g. API Server"));
                    });

                    ui.add_space(14.0);
                    section_title(ui, "EXECUTION");
                    ui.add_space(4.0);

                    field_row(ui, "Executable", |ui| {
                        if ui.small_button("Browse").clicked() {
                            if let Some(p) = rfd::FileDialog::new().set_title("Select Executable").pick_file() {
                                c.executable = p.to_string_lossy().to_string();
                            }
                        }
                        ui.add(egui::TextEdit::singleline(&mut c.executable)
                            .hint_text(if cfg!(windows) { r"C:\path\to\app.exe" } else { "/usr/bin/node" })
                            .desired_width(ui.available_width()));
                    });
                    field_row(ui, "Arguments", |ui| {
                        ui.add(egui::TextEdit::singleline(&mut c.args).hint_text(r#"--port 8080 --config "my config.json""#).desired_width(f32::INFINITY));
                    });
                    field_row(ui, "Working Dir", |ui| {
                        if ui.small_button("Browse").clicked() {
                            if let Some(p) = rfd::FileDialog::new().set_title("Select Working Directory").pick_folder() {
                                c.working_dir = p.to_string_lossy().to_string();
                            }
                        }
                        ui.add(egui::TextEdit::singleline(&mut c.working_dir)
                            .hint_text(if cfg!(windows) { r"C:\projects\myapp" } else { "/opt/myapp" })
                            .desired_width(ui.available_width()));
                    });

                    ui.add_space(14.0);
                    section_title(ui, "LOGGING");
                    ui.add_space(4.0);
                    field_row(ui, "Log File Path", |ui| {
                        ui.label(RichText::new("{name} {date}").size(9.0).color(TEXT_DIM).monospace());
                        if ui.small_button("Browse").clicked() {
                            if let Some(p) = rfd::FileDialog::new()
                                .add_filter("Log", &["log","txt"]).save_file() {
                                c.log_path = p.to_string_lossy().to_string();
                            }
                        }
                        ui.add(egui::TextEdit::singleline(&mut c.log_path)
                            .hint_text(if cfg!(windows) { r"C:\logs\{name}-{date}.log" } else { "/var/log/{name}-{date}.log" })
                            .desired_width(ui.available_width()));
                    });

                    ui.add_space(14.0);
                    section_title(ui, "SECURITY");
                    ui.add_space(4.0);
                    field_row(ui, "Run as User", |ui| {
                        ui.label(RichText::new("Unix: sudo -u").size(9.0).color(TEXT_DIM));
                        ui.add(egui::TextEdit::singleline(&mut c.run_as_user).hint_text("e.g. www-data").desired_width(200.0));
                    });

                    ui.add_space(14.0);
                    section_title(ui, "ENVIRONMENT VARIABLES");
                    ui.add_space(4.0);

                    // Env var table header
                    ui.horizontal(|ui| {
                        ui.add_space(2.0);
                        ui.label(RichText::new("KEY").size(9.0).color(TEXT_MUTED).strong());
                        ui.add_space(ui.available_width() / 2.0 - 30.0);
                        ui.label(RichText::new("VALUE").size(9.0).color(TEXT_MUTED).strong());
                    });
                    ui.add(egui::Separator::default().spacing(2.0));

                    let mut to_delete: Option<usize> = None;
                    for (i, ev) in c.env_vars.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            if ui.small_button(RichText::new("✕").size(10.0).color(RED_GHOST)).clicked() {
                                to_delete = Some(i);
                            }
                            let w = (ui.available_width() - 8.0) / 2.0;
                            ui.add(egui::TextEdit::singleline(&mut ev.key).hint_text("KEY").desired_width(w));
                            ui.add(egui::TextEdit::singleline(&mut ev.value).hint_text("value").desired_width(w));
                        });
                    }
                    if let Some(idx) = to_delete { c.env_vars.remove(idx); }

                    ui.add_space(4.0);
                    if ui.small_button(RichText::new("+ Add Variable").size(11.0).color(TEXT_SEC)).clicked() {
                        c.env_vars.push(EnvVar::default());
                    }

                    ui.add_space(24.0);
                });
        });

        self.edit = Some(edit);
    }

    // ── Confirm dialog ─────────────────────────────────────────────────────

    fn render_confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some((kind, id)) = self.confirm_delete.clone() else { return };

        // Esc cancels
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.confirm_delete = None;
            return;
        }

        // Resolve display name + which processes the delete would affect
        let (display_name, affected_ids) = if kind == "group" {
            let g = self.config.groups.iter().find(|g| g.id == id);
            (
                g.map(|g| g.name.clone()).unwrap_or_default(),
                g.map(|g| g.components.iter().map(|c| c.id.clone()).collect::<Vec<_>>()).unwrap_or_default(),
            )
        } else {
            (
                self.config.groups.iter().flat_map(|g| g.components.iter())
                    .find(|c| c.id == id).map(|c| c.name.clone()).unwrap_or_default(),
                vec![id.clone()],
            )
        };
        let running_affected = affected_ids.iter().filter(|i| self.running.contains_key(*i)).count();

        // Modal shield: a full-screen click-eating dim layer so the background
        // UI can't be interacted with (or swap the pending target) mid-dialog.
        // Clicking the shield cancels, like Esc.
        let mut shield_clicked = false;
        egui::Area::new(egui::Id::new("modal_shield"))
            .order(egui::Order::Middle)
            .fixed_pos(egui::pos2(0.0, 0.0))
            .show(ctx, |ui| {
                let screen = ui.ctx().screen_rect();
                let resp = ui.allocate_response(screen.size(), egui::Sense::click());
                ui.painter().rect_filled(screen, 0.0, Color32::from_black_alpha(120));
                if resp.clicked() { shield_clicked = true; }
            });

        let win = egui::Window::new(format!("Delete {}?", kind))
            .collapsible(false).resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(egui::Frame::none().fill(BG_CARD).rounding(8.0).stroke(Stroke::new(1.0, BORDER_HI)).inner_margin(egui::Margin::same(20.0)))
            .show(ctx, |ui| {
                ui.label(RichText::new(format!("\"{}\"", display_name)).size(13.0).color(TEXT_PRI).strong());
                ui.add_space(6.0);
                ui.label(RichText::new("This will permanently remove it from the config.").size(11.0).color(TEXT_SEC));
                if running_affected > 0 {
                    ui.add_space(4.0);
                    ui.label(RichText::new(format!(
                        "{} running process{} will be stopped.",
                        running_affected, if running_affected == 1 { "" } else { "es" }
                    )).size(11.0).color(AMBER));
                }
                ui.add_space(14.0);
                // Cancel first so it gets a real rect (right-to-left would squeeze it)
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(RichText::new("Cancel").size(12.0))
                        .min_size(Vec2::new(80.0, 0.0))).clicked() {
                        self.confirm_delete = None;
                    }
                    // Filled red here is intentional — the user is already in a
                    // confirmation context.
                    if ui.add(egui::Button::new(RichText::new("Delete").size(12.0).color(RED))
                        .fill(RED_BG).stroke(Stroke::new(1.0, RED_DIM))
                        .min_size(Vec2::new(80.0, 0.0))).clicked() {
                        // Stop any live processes first so they aren't orphaned
                        // the moment their config entry disappears.
                        for cid in &affected_ids {
                            if self.running.contains_key(cid) { self.stop(cid); }
                        }
                        if kind == "group" {
                            self.config.groups.retain(|g| g.id != id);
                            if self.selected_group.as_deref() == Some(&id) { self.selected_group = None; }
                        } else {
                            for g in &mut self.config.groups {
                                g.components.retain(|c| c.id != id);
                            }
                            if self.selected_comp.as_deref() == Some(&id) { self.selected_comp = None; }
                        }
                        self.persist();
                        self.confirm_delete = None;
                        self.edit = None;
                    }
                });
            });

        // Keep the dialog above the shield
        if let Some(w) = win { ctx.move_to_top(w.response.layer_id); }
        if shield_clicked { self.confirm_delete = None; }
    }
}

// ── UI helpers ─────────────────────────────────────────────────────────────

/// Semantic button kinds — one color trio per state, defined in one place so
/// call sites can't drift. Color rules:
///   red is exclusively destructive (Delete/Clear), amber = interrupt (Stop),
///   green = positive (Start/Save), blue = navigation accent (Logs),
///   neutral = TEXT_SEC or brighter; TEXT_DIM + no fill/stroke = disabled only.
#[derive(Clone, Copy)]
enum BtnKind {
    Positive,    // Start / Save — filled green
    Caution,     // Stop — amber, chains into the amber "Stopping…" state
    Destructive, // Delete / Clear — ghost at rest, alarm red only on hover
    Accent,      // Logs — outline accent, must not outshine Start/Stop
    Neutral,     // Config / Log file — clearly enabled, never disabled-gray
}

fn action_button(ui: &mut egui::Ui, kind: BtnKind, label: &str, size: f32, min_w: f32, enabled: bool) -> egui::Response {
    ui.scope(|ui| {
        // (text, fill, border) at rest and on hover/press
        let (rest, hover) = match kind {
            BtnKind::Positive    => ((GREEN,     GREEN_BG,             GREEN_DIM), (GREEN,    GREEN_BG_HOVER, GREEN)),
            BtnKind::Caution     => ((AMBER,     AMBER_BG,             AMBER_DIM), (AMBER,    AMBER_BG_HOVER, AMBER)),
            BtnKind::Destructive => ((RED_GHOST, Color32::TRANSPARENT, BORDER),    (RED,      RED_BG,         RED_DIM)),
            BtnKind::Accent      => ((BLUE,      Color32::TRANSPARENT, BORDER),    (BLUE_HI,  BLUE_DIM,       BLUE_BORDER)),
            BtnKind::Neutral     => ((TEXT_SEC,  BG_PANEL,             BORDER),    (TEXT_PRI, BG_HOVER,       BORDER_HI)),
        };
        let v = ui.visuals_mut();
        // The app sets a global override_text_color; clear it so the label
        // follows the per-state fg_stroke (hover brightens text too).
        v.override_text_color = None;
        let set = |w: &mut egui::style::WidgetVisuals, (fg, bg, bd): (Color32, Color32, Color32)| {
            w.weak_bg_fill = bg;
            w.bg_fill      = bg;
            w.fg_stroke    = Stroke::new(1.0, fg);
            w.bg_stroke    = Stroke::new(1.0, bd);
        };
        set(&mut v.widgets.inactive, rest);
        set(&mut v.widgets.hovered, hover);
        set(&mut v.widgets.active, hover);
        // Disabled: a visible but clearly inert slot — dim text on the neutral
        // panel fill. (Fully transparent disabled buttons made rows look broken:
        // an invisible "Stop All" just left a mystery gap in the layout.)
        set(&mut v.widgets.noninteractive, (TEXT_DIM, BG_PANEL, BORDER));

        let mut btn = egui::Button::new(RichText::new(label).size(size));
        // Fixed footprint where labels morph (Start↔Stop, "Logs (N)") so the
        // row doesn't shift under the cursor.
        if min_w > 0.0 { btn = btn.min_size(Vec2::new(min_w, 0.0)); }
        ui.add_enabled(enabled, btn)
    }).inner
}

fn section_title(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(9.5).color(TEXT_MUTED).strong());
    ui.add(egui::Separator::default().spacing(6.0));
}

fn field_row(ui: &mut egui::Ui, label: &str, content: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.add_sized([130.0, 20.0], egui::Label::new(RichText::new(label).size(11.0).color(TEXT_SEC)));
        content(ui);
    });
    ui.add_space(6.0);
}

fn meta_item(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(10.0).color(TEXT_DIM).monospace());
        ui.label(RichText::new(value).size(10.0).color(TEXT_MUTED).monospace());
    });
}

fn open_path(path: &str) {
    #[cfg(target_os = "windows")] { let _ = Command::new("explorer").arg(path).spawn(); }
    #[cfg(target_os = "macos")]   { let _ = Command::new("open").arg(path).spawn(); }
    #[cfg(target_os = "linux")]   { let _ = Command::new("xdg-open").arg(path).spawn(); }
}

fn reveal_path(path: &str) {
    let p = PathBuf::from(path);
    let dir = if p.is_file() { p.parent().unwrap_or(&p).to_string_lossy().to_string() } else { path.to_string() };
    open_path(&dir);
}

// ══════════════════════════════════════════════════════════════════════════════
// PID lock — one instance per config file, survives crashes cleanly
// ══════════════════════════════════════════════════════════════════════════════


// ══════════════════════════════════════════════════════════════════════════════
// Entry point
// ══════════════════════════════════════════════════════════════════════════════

struct AlreadyRunningApp { msg: String }
impl eframe::App for AlreadyRunningApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG_BASE).inner_margin(egui::Margin::same(24.0)))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(10.0);
                    ui.label(RichText::new("⚠  Already Running").size(15.0).color(AMBER).strong());
                    ui.add_space(12.0);
                    for line in self.msg.lines() {
                        ui.label(RichText::new(line).size(12.0).color(TEXT_SEC));
                    }
                    ui.add_space(16.0);
                    if ui.button(RichText::new("  OK  ").size(12.0)).clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
    }
}

fn print_usage() {
    eprintln!("Usage: proconductor [config.json] [--autostart] [--minimized]");
    eprintln!();
    eprintln!("  config.json   Path to config file (default: proconductor.json)");
    eprintln!("  --autostart   Start all components immediately on launch");
    eprintln!("  --minimized   Start with window minimized");
}

fn main() -> eframe::Result<()> {
    let mut config_path: Option<PathBuf> = None;
    let mut autostart  = false;
    let mut minimized  = false;

    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--autostart"              => autostart = true,
            "--minimized"              => minimized  = true,
            "--help" | "-h"            => { print_usage(); std::process::exit(0); }
            a if a.starts_with('-')    => { eprintln!("Unknown argument: {}", a); print_usage(); std::process::exit(1); }
            _                          => config_path = Some(PathBuf::from(&arg)),
        }
    }
    let config_path = config_path.unwrap_or_else(|| PathBuf::from("proconductor.json"));

    let (mut config, load_error) = load_config(&config_path);
    // Repair missing/duplicate ids from hand-edited configs; mark dirty so the
    // repair can be persisted by the user.
    let ids_repaired = sanitize_config(&mut config);

    // Single-instance check per config file using an exclusive OS file lock.
    // The lock is held for the entire process lifetime and released automatically
    // by the OS on clean exit, crash, or any other termination — no stale state.
    let lock_path = {
        let mut p = config_path.clone();
        let name = p.file_name().unwrap_or_default().to_string_lossy().to_string() + ".lock";
        p.set_file_name(name);
        p
    };
    let lock_file = std::fs::OpenOptions::new()
        .create(true).write(true).truncate(false).open(&lock_path)
        .expect("Could not open lock file");
    match lock_file.try_lock_exclusive() {
        Ok(()) => {} // acquired — this is the only instance for this config
        Err(_) => {
            let name = config_path.file_name().unwrap_or_default().to_string_lossy();
            let msg = format!("'{}' is already open in another ProConductor window.

Close that window first.", name);
            eprintln!("ProConductor: {}", msg);
            let options = eframe::NativeOptions {
                viewport: egui::ViewportBuilder::default()
                    .with_title("Already Running")
                    .with_inner_size([400.0, 130.0])
                    .with_resizable(false),
                ..Default::default()
            };
            let _ = eframe::run_native(
                "Already Running",
                options,
                Box::new(move |_cc| Box::new(AlreadyRunningApp { msg })),
            );
            std::process::exit(1);
        }
    }

    #[allow(unused_mut)]
    let mut title = format!(
        "ProConductor — {}",
        config_path.file_name().unwrap_or_default().to_string_lossy()
    );
    // Windows: the title is used to locate our HWND via FindWindowW. Two
    // instances with same-named configs in different directories would collide,
    // so make it unique. The native titlebar is hidden, nobody sees the suffix.
    #[cfg(windows)]
    { title = format!("{} [{}]", title, std::process::id()); }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(&title)
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([900.0, 580.0])
            // Windows/macOS: remove native title bar — we draw our own in
            // render_topbar. Linux keeps native decorations: the custom window
            // controls are not rendered there, so an undecorated window would
            // have no close/minimize/drag at all.
            .with_decorations(cfg!(target_os = "linux"))
            .with_icon(eframe::icon_data::from_png_bytes(&[]).unwrap_or_default()),
        ..Default::default()
    };

    // Shared PID registry — signal handler kills all children on Ctrl+C / SIGTERM
    let pid_registry: PidRegistry = Arc::new(Mutex::new(Vec::new()));
    let registry_for_signal = pid_registry.clone();

    ctrlc::set_handler(move || {
        // Kill every registered child tree: graceful signal to all, a short
        // shared grace pause, then force-kill — a single SIGTERM would leave
        // TERM-ignoring children running.
        let pids = registry_for_signal.lock().unwrap().clone();
        for &pid in &pids { graceful_kill_tree(pid); }
        thread::sleep(Duration::from_millis(500));
        for &pid in &pids { force_kill_tree(pid); }
        std::process::exit(0);
    }).expect("Failed to set signal handler");

    eframe::run_native(
        &title,
        options,
        Box::new(move |cc| Box::new(ProConductor::new(cc, config, config_path, lock_file, pid_registry, autostart, minimized, load_error, ids_repaired))),
    )
}
