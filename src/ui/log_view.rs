//! In-app log viewer (virtualized, filtered, highlighted).

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

use crate::highlight::log_line_job;
use crate::platform::{open_path, reveal_path};

impl ProConductor {
    // ── Log view ───────────────────────────────────────────────────────────

    pub(crate) fn render_log_view(&mut self, ui: &mut egui::Ui) {
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

}
