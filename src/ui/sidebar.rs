//! Sidebar: group tree, component list, add-component footer.

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

use uuid::Uuid;

impl ProConductor {

    // ── Sidebar ────────────────────────────────────────────────────────────

    pub(crate) fn render_sidebar(&mut self, ctx: &egui::Context) {
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

    pub(crate) fn render_sidebar_group(&mut self, ui: &mut egui::Ui, gid: &str) {
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

    pub(crate) fn render_sidebar_component(&mut self, ui: &mut egui::Ui, cid: &str) {
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

}
