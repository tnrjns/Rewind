//! The app's colours, in one place.
//!
//! The palette is the Python app's: a near-black window, slightly lighter cards and a
//! monochrome accent, so a screenshot of either build looks like the same product.

use eframe::egui::{self, Color32, CornerRadius, Stroke, StrokeKind};

pub const BG: Color32 = Color32::from_rgb(0x11, 0x11, 0x11);
pub const SIDEBAR: Color32 = Color32::from_rgb(0x17, 0x17, 0x17);
pub const CARD: Color32 = Color32::from_rgb(0x1D, 0x1D, 0x1D);
pub const CARD_2: Color32 = Color32::from_rgb(0x24, 0x24, 0x24);
pub const FIELD: Color32 = Color32::from_rgb(0x29, 0x29, 0x29);
pub const BORDER: Color32 = Color32::from_rgb(0x35, 0x35, 0x35);
pub const TEXT: Color32 = Color32::from_rgb(0xF2, 0xF2, 0xF2);
pub const MUTED: Color32 = Color32::from_rgb(0xA3, 0xA3, 0xA3);
pub const SUBTLE: Color32 = Color32::from_rgb(0x8C, 0x8C, 0x8C);
pub const GREEN: Color32 = Color32::from_rgb(0x65, 0xC9, 0x8A);
pub const WARN: Color32 = Color32::from_rgb(0xE5, 0xB9, 0x68);
pub const ACCENT: Color32 = Color32::from_rgb(0xD8, 0xD8, 0xD8);

/// Corner radius used by cards and thumbnails.
pub const RADIUS: CornerRadius = CornerRadius::same(10);

/// Install the palette as the egui style. Call once, at startup.
pub fn apply(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        let visuals = &mut style.visuals;
        *visuals = egui::Visuals::dark();
        visuals.panel_fill = BG;
        visuals.window_fill = CARD;
        visuals.window_stroke = Stroke::new(1.0, BORDER);
        visuals.extreme_bg_color = FIELD;
        visuals.faint_bg_color = CARD;
        visuals.selection.bg_fill = CARD_2;
        visuals.selection.stroke = Stroke::new(1.0, TEXT);
        visuals.widgets.noninteractive.bg_fill = SIDEBAR;
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        visuals.widgets.inactive.bg_fill = CARD_2;
        visuals.widgets.inactive.weak_bg_fill = CARD_2;
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(0x2E, 0x2E, 0x2E);
        visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x2E, 0x2E, 0x2E);
        visuals.widgets.active.bg_fill = FIELD;
        visuals.widgets.active.weak_bg_fill = FIELD;
        for state in [
            &mut visuals.widgets.noninteractive,
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            state.fg_stroke.color = TEXT;
            state.corner_radius = CornerRadius::same(8);
        }
        visuals.widgets.noninteractive.fg_stroke.color = MUTED;

        style.spacing.item_spacing = egui::vec2(10.0, 10.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.spacing.scroll.bar_width = 8.0;
        style
            .text_styles
            .insert(egui::TextStyle::Heading, egui::FontId::proportional(24.0));
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(13.0));
    });
}

/// The affirmative button in a dialog: monochrome accent, black text, like the
/// Python app's primary buttons.
pub fn primary(label: &'static str) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(label).color(Color32::BLACK))
        .fill(ACCENT)
        .corner_radius(CornerRadius::same(8))
}

/// The app mark: two left-pointing triangles, the shape the Python logo draws.
pub fn rewind_glyph(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(size), egui::Sense::hover());
    let painter = ui.painter();
    let height = size * 0.58;
    let width = height * 0.62;
    let gap = width * 0.92;
    let left = rect.center().x - (gap + width) / 2.0;
    for step in 0..2 {
        let x = left + gap * step as f32;
        painter.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(x, rect.center().y),
                egui::pos2(x + width, rect.center().y - height / 2.0),
                egui::pos2(x + width, rect.center().y + height / 2.0),
            ],
            TEXT,
            Stroke::NONE,
        ));
    }
}

/// [`primary`] with an owned label, for buttons whose text changes.
pub fn primary_text(label: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(label.into()).color(Color32::BLACK))
        .fill(ACCENT)
        .corner_radius(CornerRadius::same(8))
}

/// Paint a card: fill, rounded border, nothing else.
pub fn card(painter: &egui::Painter, rect: egui::Rect, fill: Color32, border: Option<Color32>) {
    painter.rect_filled(rect, RADIUS, fill);
    match border {
        Some(color) => {
            painter.rect_stroke(rect, RADIUS, Stroke::new(1.0, color), StrokeKind::Inside)
        }
        None => painter.rect_stroke(rect, RADIUS, Stroke::new(1.0, BORDER), StrokeKind::Inside),
    };
}
