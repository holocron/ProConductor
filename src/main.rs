#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod control;
mod highlight;
mod platform;
mod process;
mod theme;
mod ui;

use eframe::egui::{self, RichText};
use fs2::FileExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use app::{AppHandle, PidRegistry, ProConductor};
use config::{load_config, sanitize_config};
use process::{force_kill_tree, graceful_kill_tree};
use theme::{AMBER, BG_BASE, TEXT_SEC};

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
    eprintln!("       proconductor [config.json] --start|--stop|--restart <target> [--timeout SECS]");
    eprintln!("       proconductor [config.json] --status");
    eprintln!();
    eprintln!("  config.json   Path to config file (default: proconductor.json)");
    eprintln!("  --autostart   Start all components immediately on launch");
    eprintln!("  --minimized   Start with window minimized");
    eprintln!("  --foreground  Stay attached to the console (default: detach and return the prompt)");
    eprintln!();
    eprintln!("Remote control of an already running instance (same config file):");
    eprintln!("  --start <t>   Start a component/group        --status  Print JSON state");
    eprintln!("  --stop <t>    Stop and wait until it is down  --timeout Wait limit (default 30s)");
    eprintln!("  --restart <t> Stop, wait, start again");
    eprintln!("  <t> = component name or id, group name or id, or `all`");
    eprintln!("  Exit code 0 on success, 1 on failure/timeout. Output is the instance's reply.");
}

#[cfg(windows)]
extern "system" { fn AttachConsole(pid: u32) -> i32; }

/// Re-exec this binary with the same arguments plus `--foreground`, fully
/// detached: no stdio, own session (Unix: setsid, so closing the terminal
/// does not SIGHUP the GUI; Windows: no console attached).
fn spawn_detached() -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let mut cmd = Command::new(exe);
    cmd.args(std::env::args_os().skip(1)).arg("--foreground")
       .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(unix)] {
        use std::os::unix::process::CommandExt;
        extern "C" { fn setsid() -> i32; }
        // SAFETY: setsid is async-signal-safe and takes no arguments.
        unsafe { cmd.pre_exec(|| { setsid(); Ok(()) }); }
    }
    #[cfg(windows)] {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        cmd.creation_flags(DETACHED_PROCESS);
    }
    cmd.spawn().map(|_| ())
}

fn main() -> eframe::Result<()> {
    let mut config_path: Option<PathBuf> = None;
    let mut autostart  = false;
    let mut minimized  = false;
    let mut foreground = false;
    let mut remote: Option<(control::Action, String)> = None;
    let mut timeout_secs: u64 = 30;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--autostart"              => autostart = true,
            "--minimized"              => minimized  = true,
            "--foreground"             => foreground = true,
            "--help" | "-h"            => { print_usage(); std::process::exit(0); }
            "--status"                 => remote = Some((control::Action::Status, String::new())),
            "--start" | "--stop" | "--restart" => {
                let Some(target) = args.next() else {
                    eprintln!("{} requires a target", arg); print_usage(); std::process::exit(1);
                };
                let action = control::Action::parse(&arg[2..]).expect("known action");
                remote = Some((action, target));
            }
            "--timeout" => {
                timeout_secs = args.next().and_then(|s| s.parse().ok()).unwrap_or_else(|| {
                    eprintln!("--timeout requires a number of seconds"); std::process::exit(1);
                });
            }
            a if a.starts_with('-')    => { eprintln!("Unknown argument: {}", a); print_usage(); std::process::exit(1); }
            _                          => config_path = Some(PathBuf::from(&arg)),
        }
    }
    let config_path = config_path.unwrap_or_else(|| PathBuf::from("proconductor.json"));

    let (mut config, load_error) = load_config(&config_path);

    // ── CLI client mode: talk to the running instance and exit ──────────────
    if let Some((action, target)) = remote {
        // Release builds are a GUI-subsystem executable on Windows and start
        // without a console; borrow the parent's so the reply is visible.
        // (No children are spawned in this mode, so the stdio-handle caveat in
        // process.rs does not apply here.)
        #[cfg(windows)]
        unsafe { AttachConsole(u32::MAX); }
        if let Some(e) = &load_error { eprintln!("ProConductor: {}", e); std::process::exit(1); }
        match control::send_command(&config, &config_path, action, &target, Duration::from_secs(timeout_secs)) {
            Ok((true,  msg)) => { println!("{}", msg); std::process::exit(0); }
            Ok((false, msg)) => { eprintln!("{}", msg); std::process::exit(1); }
            Err(e)           => { eprintln!("ProConductor: {}", e); std::process::exit(1); }
        }
    }
    // ── Detach from the launching console ───────────────────────────────────
    // Relaunch ourselves as a background session and return the prompt at once;
    // `--foreground` keeps the classic attached behaviour (debugging, systemd).
    if !foreground {
        match spawn_detached() {
            Ok(())  => std::process::exit(0),
            Err(e)  => eprintln!("ProConductor: could not detach from console ({}); running in foreground.", e),
        }
    }

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
    let port_file_for_signal = control::port_file_for(&config_path);

    ctrlc::set_handler(move || {
        // Kill every registered child tree: graceful signal to all, a short
        // shared grace pause, then force-kill — a single SIGTERM would leave
        // TERM-ignoring children running.
        let pids = registry_for_signal.lock().unwrap().clone();
        for &pid in &pids { graceful_kill_tree(pid); }
        thread::sleep(Duration::from_millis(500));
        for &pid in &pids { force_kill_tree(pid); }
        // process::exit skips destructors — drop the advertised port by hand.
        let _ = std::fs::remove_file(&port_file_for_signal);
        std::process::exit(0);
    }).expect("Failed to set signal handler");

    eframe::run_native(
        &title,
        options,
        Box::new(move |cc| Box::new(AppHandle::new(ProConductor::new(cc, config, config_path, lock_file, pid_registry, autostart, minimized, load_error, ids_repaired)))),
    )
}
