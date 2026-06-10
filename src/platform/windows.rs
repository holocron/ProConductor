//! Windows-only glue: Win32 FFI for taskbar/tray window management and the
//! tray-icon background thread.

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use eframe::egui;
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    TrayIconBuilder, TrayIconEvent,
};

use crate::app::{AppEvent, ProConductor};

// Win32 FFI — taskbar removal + message pump for tray events

// Win32 type names mirrored verbatim for FFI clarity
#[allow(clippy::upper_case_acronyms)]
#[repr(C)]
struct MSG { hwnd: isize, message: u32, w: usize, l: isize, time: u32, pt_x: i32, pt_y: i32 }


#[allow(clippy::upper_case_acronyms)]
#[repr(C)]
struct RECT { left: i32, top: i32, right: i32, bottom: i32 }


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



pub(crate) const CREATE_NO_WINDOW: u32 = 0x08000000;

// Tray icon variants — 32x32 RGBA, loaded from icons/ at compile time

pub(crate) const TRAY_ICON_W: u32 = 32;

pub(crate) const TRAY_ICON_H: u32 = 32;

pub(crate) const TRAY_ICON_GREEN: &[u8] = include_bytes!("../../icons/tray_green.rgba");

pub(crate) const TRAY_ICON_AMBER: &[u8] = include_bytes!("../../icons/tray_amber.rgba");

pub(crate) const TRAY_ICON_RED:   &[u8] = include_bytes!("../../icons/tray_red.rgba");

/// Commands sent from main thread → tray background thread
pub(crate) enum TrayCmd {
    SetIcon(u8),    // 0=red 1=amber 2=green
    SetHwnd(isize), // share HWND so tray thread can call ShowWindow directly
}

/// Spawn the tray background thread and return the command sender.
/// tray-icon uses Rc internally — not Send — so the TrayIcon handle can never
/// leave the thread it was built on. We keep it entirely inside the background
/// thread and communicate via channels:
///   app → thread : TrayCmd  (icon colour changes)
///   thread → app : AppEvent (show/quit)
pub(crate) fn spawn_tray_thread(tx_app: mpsc::Sender<AppEvent>, ctx_tray: egui::Context) -> Option<mpsc::Sender<TrayCmd>> {
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
}

impl ProConductor {
    // ── Tray (Windows only) ───────────────────────────────────────────────────

    #[cfg(windows)]
    pub(crate) fn handle_tray(&mut self, ctx: &egui::Context) {
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
}
