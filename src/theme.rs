//! Palette and semantic button styling.

use eframe::egui::{self, Color32, RichText, Stroke, Vec2};

// ══════════════════════════════════════════════════════════════════════════════
// Palette
// ══════════════════════════════════════════════════════════════════════════════

pub(crate) const BG_BASE:    Color32 = Color32::from_rgb(  9,  12,  18);
pub(crate) const BG_PANEL:   Color32 = Color32::from_rgb( 14,  20,  32);
pub(crate) const BG_CARD:    Color32 = Color32::from_rgb( 22,  30,  46);
pub(crate) const BG_RAISED:  Color32 = Color32::from_rgb( 18,  25,  38);
pub(crate) const BG_INPUT:   Color32 = Color32::from_rgb( 10,  15,  24);
pub(crate) const BG_HOVER:   Color32 = Color32::from_rgb( 26,  40,  64);
pub(crate) const BG_SEL:     Color32 = Color32::from_rgb( 18,  32,  68);

pub(crate) const BORDER:     Color32 = Color32::from_rgb( 28,  42,  62);
pub(crate) const BORDER_HI:  Color32 = Color32::from_rgb( 46,  64,  96);

pub(crate) const TEXT_PRI:   Color32 = Color32::from_rgb(220, 232, 245);
pub(crate) const TEXT_SEC:   Color32 = Color32::from_rgb(122, 155, 191);
pub(crate) const TEXT_MUTED: Color32 = Color32::from_rgb( 61,  85, 112);
pub(crate) const TEXT_DIM:   Color32 = Color32::from_rgb( 35,  52,  72);

pub(crate) const GREEN:      Color32 = Color32::from_rgb( 33, 212, 126);
pub(crate) const GREEN_DIM:  Color32 = Color32::from_rgb( 14,  74,  44);
pub(crate) const GREEN_BG:   Color32 = Color32::from_rgb(  7,  28,  18);
pub(crate) const RED:        Color32 = Color32::from_rgb(240,  74,  94);
pub(crate) const RED_DIM:    Color32 = Color32::from_rgb( 74,  15,  22);
pub(crate) const RED_BG:     Color32 = Color32::from_rgb( 30,   6,  10);
pub(crate) const AMBER:      Color32 = Color32::from_rgb(240, 160,  48);
pub(crate) const AMBER_DIM:  Color32 = Color32::from_rgb( 74,  48,  10);
pub(crate) const AMBER_BG:   Color32 = Color32::from_rgb( 30,  20,   7);
pub(crate) const BLUE:       Color32 = Color32::from_rgb( 68, 136, 255);
pub(crate) const BLUE_DIM:   Color32 = Color32::from_rgb( 18,  32, 100);

// Button-state shades — see action_button()
pub(crate) const RED_GHOST:      Color32 = Color32::from_rgb(150,  70,  80);
pub(crate) const BLUE_HI:        Color32 = Color32::from_rgb(120, 170, 255);
pub(crate) const BLUE_BORDER:    Color32 = Color32::from_rgb( 30,  50, 120);
pub(crate) const GREEN_BG_HOVER: Color32 = Color32::from_rgb( 10,  42,  27);
pub(crate) const AMBER_BG_HOVER: Color32 = Color32::from_rgb( 45,  30,  10);
// ── UI helpers ─────────────────────────────────────────────────────────────

/// Semantic button kinds — one color trio per state, defined in one place so
/// call sites can't drift. Color rules:
///   red is exclusively destructive (Delete/Clear), amber = interrupt (Stop),
///   green = positive (Start/Save), blue = navigation accent (Logs),
///   neutral = TEXT_SEC or brighter; TEXT_DIM + no fill/stroke = disabled only.
#[derive(Clone, Copy)]
pub(crate) enum BtnKind {
    Positive,    // Start / Save — filled green
    Caution,     // Stop — amber, chains into the amber "Stopping…" state
    Destructive, // Delete / Clear — ghost at rest, alarm red only on hover
    Accent,      // Logs — outline accent, must not outshine Start/Stop
    Neutral,     // Config / Log file — clearly enabled, never disabled-gray
}

pub(crate) fn action_button(ui: &mut egui::Ui, kind: BtnKind, label: &str, size: f32, min_w: f32, enabled: bool) -> egui::Response {
    ui.scope(|ui| {
        // (text, fill, border) at rest and on hover/press
        let (rest, hover) = match kind {
            BtnKind::Positive    => ((GREEN,     GREEN_BG,             GREEN_DIM), (GREEN,    GREEN_BG_HOVER, GREEN)),
            BtnKind::Caution     => ((AMBER,     AMBER_BG,             AMBER_DIM), (AMBER,    AMBER_BG_HOVER, AMBER)),
            BtnKind::Destructive => ((RED_GHOST, Color32::TRANSPARENT, BORDER),    (RED,      RED_BG,         RED_DIM)),
            BtnKind::Accent      => ((BLUE,      Color32::TRANSPARENT, BORDER),    (BLUE_HI,  BLUE_DIM,       BLUE_BORDER)),
            BtnKind::Neutral     => ((TEXT_SEC,  BG_PANEL,             BORDER),    (TEXT_PRI, BG_HOVER,       BORDER_HI)),
        };
        let v = ui.visuals_mut();
        // The app sets a global override_text_color; clear it so the label
        // follows the per-state fg_stroke (hover brightens text too).
        v.override_text_color = None;
        let set = |w: &mut egui::style::WidgetVisuals, (fg, bg, bd): (Color32, Color32, Color32)| {
            w.weak_bg_fill = bg;
            w.bg_fill      = bg;
            w.fg_stroke    = Stroke::new(1.0, fg);
            w.bg_stroke    = Stroke::new(1.0, bd);
        };
        set(&mut v.widgets.inactive, rest);
        set(&mut v.widgets.hovered, hover);
        set(&mut v.widgets.active, hover);
        // Disabled: a visible but clearly inert slot — dim text on the neutral
        // panel fill. (Fully transparent disabled buttons made rows look broken:
        // an invisible "Stop All" just left a mystery gap in the layout.)
        set(&mut v.widgets.noninteractive, (TEXT_DIM, BG_PANEL, BORDER));

        let mut btn = egui::Button::new(RichText::new(label).size(size));
        // Fixed footprint where labels morph (Start↔Stop, "Logs (N)") so the
        // row doesn't shift under the cursor.
        if min_w > 0.0 { btn = btn.min_size(Vec2::new(min_w, 0.0)); }
        ui.add_enabled(enabled, btn)
    }).inner
}
