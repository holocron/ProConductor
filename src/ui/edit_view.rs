//! Component configure/new view.

use eframe::egui::{self, Align, Layout, RichText, Stroke};
#[allow(unused_imports)]
use std::path::PathBuf;
#[allow(unused_imports)]
use std::time::Duration;
use crate::app::*;
use crate::theme::*;
use crate::ui::{section_title, field_row};
#[allow(unused_imports)]
use crate::config::*;
#[allow(unused_imports)]
use crate::process::*;

impl ProConductor {
    // ── Edit view ──────────────────────────────────────────────────────────

    pub(crate) fn render_edit_view(&mut self, ui: &mut egui::Ui, mut edit: EditState) {
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
                    field_row(ui, "Remote Control", |ui| {
                        ui.checkbox(&mut c.remote_control, "");
                        ui.label(RichText::new("allow start/stop/restart via CLI & command files")
                            .size(9.0).color(TEXT_DIM));
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

}
