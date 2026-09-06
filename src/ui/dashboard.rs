//! Dashboard: group headers and component cards.

use eframe::egui::{self, Align, Color32, Layout, RichText, Stroke, Vec2};
#[allow(unused_imports)]
use std::path::PathBuf;
#[allow(unused_imports)]
use std::time::Duration;
use crate::app::*;
use crate::theme::*;
use crate::ui::meta_item;
use crate::platform::open_path;
use uuid::Uuid;
#[allow(unused_imports)]
use crate::config::*;
#[allow(unused_imports)]
use crate::process::*;

impl ProConductor {
    // ── Dashboard ──────────────────────────────────────────────────────────

    pub(crate) fn render_dashboard(&mut self, ui: &mut egui::Ui) {
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

    pub(crate) fn render_component_card(&mut self, ui: &mut egui::Ui, cid: &str) {
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
        let mut do_restart   = false;
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
                            if action_button(ui, BtnKind::Accent, "Restart", 11.0, 72.0, !just_started)
                                .on_hover_text("Stop, wait for exit, start again")
                                .clicked() { do_restart = true; }
                        } else if action_button(ui, BtnKind::Positive, "Start", 11.0, 72.0, true)
                            .clicked() { do_start = true; }
                    });
                });
            });

        if do_start     { self.start(&comp); }
        if do_stop      { self.stop(cid); }
        if do_restart   { self.restart(cid); }
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
}
