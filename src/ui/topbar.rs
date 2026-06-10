//! Topbar: title/drag strip, right-side controls, window controls.

use eframe::egui::{self, Align, Color32, Layout, RichText, Stroke, Vec2};
#[allow(unused_imports)]
use std::path::PathBuf;
#[allow(unused_imports)]
use std::time::Duration;
use crate::app::*;
use crate::theme::*;
#[allow(unused_imports)]
use crate::config::*;
#[allow(unused_imports)]
use crate::process::*;

impl ProConductor {
    // ── Topbar ─────────────────────────────────────────────────────────────

    pub(crate) fn render_topbar(&mut self, ctx: &egui::Context) {
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

    pub(crate) fn render_topbar_controls(&mut self, ctx: &egui::Context) {
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
    pub(crate) fn render_window_controls(&self, ctx: &egui::Context) {
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
}
