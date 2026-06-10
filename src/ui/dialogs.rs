//! Modal confirm dialog.

use eframe::egui::{self, Color32, RichText, Stroke, Vec2};
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
    // ── Confirm dialog ─────────────────────────────────────────────────────

    pub(crate) fn render_confirm_dialog(&mut self, ctx: &egui::Context) {
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
