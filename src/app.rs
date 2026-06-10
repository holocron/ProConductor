//! Application state and runtime: the ProConductor struct, event loop,
//! process start/stop orchestration, and the eframe::App impl.

use eframe::egui::{self, FontId, Stroke, Vec2};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use crate::config::{AppConfig, Component, save_config};
use crate::process::*;
use crate::theme::*;

#[cfg(windows)]
use crate::platform::windows::{TrayCmd, CREATE_NO_WINDOW};
#[cfg(target_os = "macos")]
use crate::platform::macos::{set_dock_icon, DOCK_ICON_AMBER, DOCK_ICON_GREEN, DOCK_ICON_RED, DOCK_ICON_H, DOCK_ICON_W};

// Shared registry of running child PIDs — read by the signal handler to kill
// all children before the process exits (e.g. on Ctrl+C or SIGTERM).
pub(crate) type PidRegistry = Arc<Mutex<Vec<u32>>>;

pub enum AppEvent {
    Log         { id: String, line: LogLine },
    Status      { id: String, running: bool, exit_code: Option<i32> },
    Stopped     { id: String },
    // Never constructed: the tray thread restores the window via Win32
    // ShowWindow directly. Kept so the event API covers both tray actions.
    #[allow(dead_code)]
    ShowWindow,
    // Constructed only by the Windows tray thread
    #[cfg_attr(not(windows), allow(dead_code))]
    QuitApp,
}


pub(crate) struct RunningProcess {
    pub(crate) pid:        u32,
    pub(crate) started_at: Instant,
    pub(crate) child:      Arc<Mutex<Child>>,
    pub(crate) cancelled:  Arc<std::sync::atomic::AtomicBool>,
}

// ══════════════════════════════════════════════════════════════════════════════
// UI state helpers
// ══════════════════════════════════════════════════════════════════════════════

#[derive(PartialEq, Clone)]
pub(crate) enum MainView { Dashboard, Log, Edit }

#[derive(Clone)]
pub(crate) struct EditState {
    pub(crate) component:  Component,
    pub(crate) group_id:   String,
    pub(crate) is_new:     bool,
}

// ══════════════════════════════════════════════════════════════════════════════
// App
// ══════════════════════════════════════════════════════════════════════════════

pub(crate) struct ProConductor {
    // Persistent
    pub(crate) config:      AppConfig,
    pub(crate) config_path: PathBuf,
    pub(crate) dirty:       bool,
    // Held for entire lifetime — OS releases exclusive lock on drop (incl. crash)
    pub(crate) _lock:       std::fs::File,
    // Shared with signal handler — all running child PIDs
    pub(crate) pid_registry: PidRegistry,

    // Runtime
    pub(crate) running:    HashMap<String, RunningProcess>,
    pub(crate) stopping:   HashMap<String, Instant>,   // id → when stop was requested
    pub(crate) last_exit:  HashMap<String, i32>,       // id → last nonzero exit code (crash badge)
    pub(crate) logs:       HashMap<String, Vec<LogLine>>,
    pub(crate) event_tx:   mpsc::Sender<AppEvent>,
    pub(crate) event_rx:   mpsc::Receiver<AppEvent>,
    pub(crate) ctx_handle: egui::Context,

    // Error banners — shown until dismissed
    pub(crate) config_load_error: Option<String>,
    pub(crate) save_error:        Option<String>,

    // UI
    pub(crate) start_minimized: bool,   // send minimize command on first frame
    #[cfg(windows)]
    pub(crate) tray_cmd_tx: Option<mpsc::Sender<TrayCmd>>,  // send icon-change cmds to tray thread
    #[cfg(windows)]
    pub(crate) tray_status: u8,               // 0=red 1=amber 2=green — track to avoid redundant swaps
    pub(crate) show_window_requested: bool,
    pub(crate) quit_requested:        bool,
    #[cfg(target_os = "macos")]
    pub(crate) macos_dock_status: u8,   // 0=red 1=amber 2=green
    #[cfg(windows)]
    pub(crate) taskbar_hidden: bool,  // whether we've removed the window from taskbar yet
    #[cfg(windows)]
    pub(crate) just_hid: bool,        // skip one frame after hiding to avoid instant restore
    pub(crate) selected_comp:  Option<String>,
    pub(crate) selected_group: Option<String>,
    pub(crate) view:           MainView,
    pub(crate) edit:           Option<EditState>,
    pub(crate) open_groups:    HashSet<String>,
    pub(crate) log_filter:     String,
    pub(crate) log_autoscroll: bool,
    pub(crate) confirm_delete: Option<(String, String)>, // (kind, id)
}

impl ProConductor {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(cc: &eframe::CreationContext, config: AppConfig, path: PathBuf, lock: std::fs::File, pid_registry: PidRegistry, autostart: bool, minimized: bool, load_error: Option<String>, initial_dirty: bool) -> Self {
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

        // Tray thread lives in platform::windows — see spawn_tray_thread.
        #[cfg(windows)]
        let tray_cmd_tx: Option<mpsc::Sender<TrayCmd>> =
            crate::platform::windows::spawn_tray_thread(tx.clone(), cc.egui_ctx.clone());

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
            tray_cmd_tx,
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

    pub(crate) fn start(&mut self, comp: &Component) {
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
                            id: id_mon, running: false, exit_code: code,
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

    pub(crate) fn stop(&mut self, id: &str) {
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
    pub(crate) fn shutdown_sync(&mut self) {
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

    pub(crate) fn start_all(&mut self) {
        let comps: Vec<Component> = self.config.groups.iter()
            .flat_map(|g| g.components.iter().cloned())
            .filter(|c| !self.running.contains_key(&c.id) && !self.stopping.contains_key(&c.id))
            .collect();
        for c in comps { self.start(&c); }
    }

    pub(crate) fn stop_all(&mut self) {
        let ids: Vec<String> = self.running.keys().cloned().collect();
        for id in ids { self.stop(&id); }
    }

    pub(crate) fn start_group(&mut self, group_id: &str) {
        let comps: Vec<Component> = self.config.groups.iter()
            .find(|g| g.id == group_id)
            .map(|g| g.components.iter()
                .filter(|c| !self.running.contains_key(&c.id) && !self.stopping.contains_key(&c.id))
                .cloned().collect())
            .unwrap_or_default();
        for c in comps { self.start(&c); }
    }

    pub(crate) fn stop_group(&mut self, group_id: &str) {
        let ids: Vec<String> = self.config.groups.iter()
            .find(|g| g.id == group_id)
            .map(|g| g.components.iter()
                .filter(|c| self.running.contains_key(&c.id))
                .map(|c| c.id.clone()).collect())
            .unwrap_or_default();
        for id in ids { self.stop(&id); }
    }

    pub(crate) fn group_running_count(&self, group_id: &str) -> (usize, usize) {
        // returns (running, total)
        let comps = self.config.groups.iter()
            .find(|g| g.id == group_id)
            .map(|g| &g.components[..])
            .unwrap_or(&[]);
        let running = comps.iter().filter(|c| self.running.contains_key(&c.id)).count();
        (running, comps.len())
    }

    pub(crate) fn drain_events(&mut self) {
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
    pub(crate) fn persist(&mut self) {
        match save_config(&self.config, &self.config_path) {
            Ok(())  => { self.dirty = false; self.save_error = None; }
            Err(e)  => {
                self.dirty = true;
                self.save_error = Some(format!("Failed to save {}: {}", self.config_path.display(), e));
            }
        }
    }

    // ── Counts ────────────────────────────────────────────────────────────
    pub(crate) fn running_count(&self) -> usize { self.running.len() }
    pub(crate) fn total_count(&self)   -> usize {
        self.config.groups.iter().map(|g| g.components.len()).sum()
    }
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
