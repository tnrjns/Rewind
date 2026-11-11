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
    install_ui_font(ctx);
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
            // The stylesheet's buttons, fields and menus all use a 6 px radius.
            state.corner_radius = CornerRadius::same(6);
        }
        visuals.widgets.noninteractive.fg_stroke.color = MUTED;

        style.spacing.item_spacing = egui::vec2(10.0, 10.0);
        style.spacing.button_padding = egui::vec2(16.0, 8.0);
        style.spacing.scroll.bar_width = 10.0;
        style
            .text_styles
            .insert(egui::TextStyle::Heading, egui::FontId::proportional(24.0));
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(13.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(13.0));
    });
}

/// Use the platform's own UI font, so the port renders like the app it replaces.
///
/// egui ships a light Ubuntu face; the Python app uses the system font (SF Pro on
/// macOS, Segoe UI on Windows). A missing or unreadable file is not fatal — the
/// bundled font stays in place.
fn install_ui_font(ctx: &egui::Context) {
    let candidates: &[&str] = if cfg!(target_os = "macos") {
        &["/System/Library/Fonts/SFNS.ttf"]
    } else if cfg!(target_os = "windows") {
        &["C:\\Windows\\Fonts\\segoeui.ttf"]
    } else {
        &[]
    };
    let Some(bytes) = candidates.iter().find_map(|path| std::fs::read(path).ok()) else {
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "ui".to_string(),
        std::sync::Arc::new(egui::FontData::from_owned(bytes)),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "ui".to_string());
    ctx.set_fonts(fonts);
}

/// The affirmative button in a dialog: monochrome accent, black text, like the
/// Python app's primary buttons.
pub fn primary(label: &'static str) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(label).color(Color32::BLACK))
        .fill(ACCENT)
        .corner_radius(CornerRadius::same(8))
}

/// The two left-pointing triangles, at `size` tall, centred on `centre`.
///
/// This is the Python app's `draw_rewind_glyph` to the point: its triangles sit a
/// little left of centre, because the wide ends make them look off-centre otherwise.
pub fn glyph(painter: &egui::Painter, centre: egui::Pos2, size: f32, color: Color32) {
    let height = size;
    let width = size * 0.62;
    let left = centre.x - width + width * 0.08 - width * 0.14;
    for step in 0..2 {
        let x = left + width * 0.92 * step as f32;
        painter.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(x, centre.y),
                egui::pos2(x + width, centre.y - height / 2.0),
                egui::pos2(x + width, centre.y + height / 2.0),
            ],
            color,
            Stroke::NONE,
        ));
    }
}

/// The app mark, matching `theme.draw_logo`: a rounded tile, a thin border, a circular
/// arrow that ends in a solid head, and the rewind glyph in the middle.
pub fn logo(painter: &egui::Painter, rect: egui::Rect) {
    let size = rect.width().min(rect.height());
    let square = egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(size));
    let corner = CornerRadius::same((size * 0.24).round() as u8);
    painter.rect_filled(square, corner, SIDEBAR);
    painter.rect_stroke(
        square.shrink(0.5),
        corner,
        Stroke::new((size * 0.025).max(1.0), Color32::from_rgb(0x77, 0x77, 0x77)),
        StrokeKind::Inside,
    );

    let centre = square.center();
    let radius = size * 0.31;
    let width = (size * 0.085).max(1.5);
    let start = 125.0_f32.to_radians();
    let span = 290.0_f32.to_radians();
    let point = |angle: f32| {
        egui::pos2(
            centre.x + radius * angle.cos(),
            centre.y - radius * angle.sin(),
        )
    };
    let points: Vec<egui::Pos2> = (0..=48)
        .map(|i| point(start + span * (i as f32 / 48.0)))
        .collect();
    painter.add(egui::Shape::line(points.clone(), Stroke::new(width, TEXT)));
    // Round caps at both ends of the arc.
    painter.circle_filled(points[0], width / 2.0, TEXT);
    painter.circle_filled(*points.last().unwrap_or(&centre), width / 2.0, TEXT);

    // Arrow head at the end of the arc, pointing along the direction of travel.
    let end = start + span;
    let tip = point(end);
    let tangent = egui::vec2(-end.sin(), -end.cos());
    let normal = egui::vec2(-tangent.y, tangent.x);
    painter.add(egui::Shape::convex_polygon(
        vec![
            tip + tangent * (size * 0.13),
            tip + normal * (size * 0.11),
            tip - normal * (size * 0.11),
        ],
        TEXT,
        Stroke::NONE,
    ));

    glyph(painter, centre, size * 0.30, TEXT);
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
