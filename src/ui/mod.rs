//! UI views — each file holds an `impl ProConductor` block.

pub(crate) mod topbar;
pub(crate) mod sidebar;
pub(crate) mod dashboard;
pub(crate) mod log_view;
pub(crate) mod edit_view;
pub(crate) mod dialogs;

use eframe::egui::{self, Align, Layout, RichText, Stroke};
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
    // ── Main panel ─────────────────────────────────────────────────────────

    /// Dismissible error banners (config load / save failures) — shown above
    /// every view so a failed save can't go unnoticed.
    pub(crate) fn render_banners(&mut self, ui: &mut egui::Ui) {
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

    pub(crate) fn render_main(&mut self, ctx: &egui::Context) {
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

}

pub(crate) fn section_title(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(9.5).color(TEXT_MUTED).strong());
    ui.add(egui::Separator::default().spacing(6.0));
}

pub(crate) fn field_row(ui: &mut egui::Ui, label: &str, content: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.add_sized([130.0, 20.0], egui::Label::new(RichText::new(label).size(11.0).color(TEXT_SEC)));
        content(ui);
    });
    ui.add_space(6.0);
}

pub(crate) fn meta_item(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(10.0).color(TEXT_DIM).monospace());
        ui.label(RichText::new(value).size(10.0).color(TEXT_MUTED).monospace());
    });
}
