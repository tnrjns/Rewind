//! The pieces a settings page is built from: cards, rows, switches, sliders, combos.
//!
//! These mirror `app/widgets.py` — `Card`, `SettingRow`, `SliderControl`,
//! `ToggleSwitch` — so a ported page looks like the Python one rather than like a
//! generic egui form.

use eframe::egui::{self, Align, Color32, Layout, RichText, Sense, Stroke, Vec2};

use crate::ui::theme;

/// A bordered card with an optional uppercase title.
pub fn card(ui: &mut egui::Ui, title: Option<&str>, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(theme::CARD)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(egui::CornerRadius::same(14))
        .inner_margin(egui::Margin {
            left: 22,
            right: 22,
            top: 16,
            bottom: 10,
        })
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.set_width(ui.available_width());
            if let Some(title) = title {
                ui.label(
                    RichText::new(title.to_uppercase())
                        .size(11.0)
                        .strong()
                        .color(theme::SUBTLE),
                );
                ui.add_space(4.0);
            }
            add(ui);
        });
}

/// One label and description on the left, one control on the right.
pub fn row(ui: &mut egui::Ui, title: &str, sub: Option<&str>, control: impl FnOnce(&mut egui::Ui)) {
    row_with(ui, title, sub, theme::MUTED, control);
}

/// A row whose description is coloured, for a line that reports a problem.
pub fn row_with(
    ui: &mut egui::Ui,
    title: &str,
    sub: Option<&str>,
    sub_color: Color32,
    control: impl FnOnce(&mut egui::Ui),
) {
    let text_width = (ui.available_width() - 300.0).max(180.0);
    egui::Frame::new()
        .inner_margin(egui::Margin {
            left: 0,
            right: 0,
            top: 12,
            bottom: 12,
        })
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 18.0;
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.set_max_width(text_width);
                    ui.label(RichText::new(title).size(13.5).strong());
                    if let Some(sub) = sub.filter(|text| !text.is_empty()) {
                        ui.add_space(3.0);
                        ui.label(RichText::new(sub).size(12.0).color(sub_color));
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), control);
            });
        });
}

/// The 1 px line the Python app draws between rows in a card.
pub fn separator(ui: &mut egui::Ui) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::BORDER);
}

/// A switch, as the Python app draws it: a pill that flips rather than a tick box.
pub fn switch(ui: &mut egui::Ui, on: &mut bool) -> bool {
    switch_sized(ui, on, Vec2::new(44.0, 24.0))
}

/// The same switch at the hero's size.
pub fn switch_large(ui: &mut egui::Ui, on: &mut bool) -> bool {
    switch_sized(ui, on, Vec2::new(62.0, 34.0))
}

fn switch_sized(ui: &mut egui::Ui, on: &mut bool, size: Vec2) -> bool {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let t = ui.ctx().animate_bool(response.id, *on);
    let painter = ui.painter();
    let radius = rect.height() / 2.0;
    // The switch's off track is #4A4A4A, not the card fill, and the knob is white.
    let off = Color32::from_rgb(0x4A, 0x4A, 0x4A);
    painter.rect_filled(
        rect,
        egui::CornerRadius::same(radius as u8),
        mix(off, theme::ACCENT, t),
    );
    let knob = radius - 3.0;
    let knob_x = rect.left() + radius + (rect.width() - rect.height()) * t;
    painter.circle_filled(
        egui::pos2(knob_x, rect.center().y + 1.0),
        knob,
        Color32::from_black_alpha(40),
    );
    painter.circle_filled(egui::pos2(knob_x, rect.center().y), knob, Color32::WHITE);
    response.clicked()
}

/// A thin progress bar: `#383838` track, accent fill, as the Home page draws it.
pub fn bar(ui: &mut egui::Ui, fraction: f32, width: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 6.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        egui::CornerRadius::same(3),
        Color32::from_rgb(0x38, 0x38, 0x38),
    );
    let filled = fraction.clamp(0.0, 1.0) * rect.width();
    if filled > 0.5 {
        painter.rect_filled(
            egui::Rect::from_min_size(rect.min, Vec2::new(filled, rect.height())),
            egui::CornerRadius::same(3),
            theme::ACCENT,
        );
    }
}

/// One of the four Home tiles: a small label over a value.
pub fn tile(ui: &mut egui::Ui, label: &str, value: &str) {
    egui::Frame::new()
        .fill(theme::CARD_2)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin {
            left: 14,
            right: 14,
            top: 11,
            bottom: 12,
        })
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.set_width(ui.available_width());
            ui.label(RichText::new(label).size(11.0).strong().color(theme::MUTED));
            ui.add_space(3.0);
            ui.label(RichText::new(value).size(15.0).strong());
        });
}

/// A filter/quick-setting pill.
pub fn chip(ui: &mut egui::Ui, label: &str, selected: bool) -> bool {
    let (fill, text, border) = if selected {
        (
            Color32::from_rgb(0x3A, 0x3A, 0x3A),
            theme::TEXT,
            Color32::from_rgb(0x8A, 0x8A, 0x8A),
        )
    } else {
        (theme::FIELD, theme::MUTED, theme::BORDER)
    };
    ui.add(
        egui::Button::new(RichText::new(label).size(13.0).color(text))
            .fill(fill)
            .stroke(Stroke::new(1.0, border))
            .corner_radius(egui::CornerRadius::same(15))
            .min_size(Vec2::new(0.0, 30.0)),
    )
    .clicked()
}

/// Blend two colours, for the switch's track.
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let channel = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(
        channel(a.r(), b.r()),
        channel(a.g(), b.g()),
        channel(a.b(), b.b()),
    )
}

/// Shrink text with an ellipsis until it fits `max_width`.
pub fn elide(ui: &egui::Ui, text: &str, font: egui::FontId, max_width: f32) -> String {
    fn fits(ui: &egui::Ui, text: &str, font: &egui::FontId, max_width: f32) -> bool {
        ui.painter()
            .layout_no_wrap(text.to_string(), font.clone(), theme::TEXT)
            .size()
            .x
            <= max_width
    }
    if fits(ui, text, &font, max_width) {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut low = 0;
    let mut high = chars.len();
    while low + 1 < high {
        let mid = (low + high) / 2;
        let candidate: String = chars[..mid].iter().collect::<String>() + "…";
        if fits(ui, &candidate, &font, max_width) {
            low = mid;
        } else {
            high = mid;
        }
    }
    chars[..low].iter().collect::<String>() + "…"
}

/// A dropdown over fixed labels; returns the index the user picked, if any.
pub fn combo(ui: &mut egui::Ui, id: &str, labels: &[String], current: usize) -> Option<usize> {
    let mut chosen = None;
    egui::ComboBox::from_id_salt(id)
        .selected_text(labels.get(current).cloned().unwrap_or_default())
        .width(216.0)
        .show_ui(ui, |ui| {
            for (index, label) in labels.iter().enumerate() {
                if ui.selectable_label(index == current, label).clicked() {
                    chosen = Some(index);
                }
            }
        });
    chosen
}

/// An integer slider that snaps to `step`, with a right-hand value readout.
/// What a slider did this frame.
#[derive(Default)]
pub struct SliderEdit {
    pub changed: bool,
    /// The handle was let go: the moment to play a sample of what was chosen.
    pub released: bool,
}

pub fn slider(
    ui: &mut egui::Ui,
    value: &mut i64,
    range: (i64, i64),
    step: i64,
    suffix: &str,
) -> SliderEdit {
    ui.spacing_mut().slider_width = 230.0;
    // Row controls sit in a right-to-left layout, so the readout is added before the
    // track: that renders as [track][value], the way the Python app's SliderControl does.
    ui.spacing_mut().item_spacing.x = 14.0;
    ui.add_sized(
        [78.0, 18.0],
        egui::Label::new(
            egui::RichText::new(format!("{value}{suffix}"))
                .size(13.0)
                .strong(),
        ),
    );
    let mut scaled = *value as f64;
    let response = ui.add(
        egui::Slider::new(&mut scaled, range.0 as f64..=range.1 as f64)
            .step_by(step as f64)
            .show_value(false),
    );
    let changed = response.changed();
    if changed {
        *value = scaled.round() as i64;
    }
    SliderEdit {
        changed,
        released: response.drag_stopped(),
    }
}

/// A number field with a step, for values where a slider would be too coarse.
pub fn number(
    ui: &mut egui::Ui,
    value: &mut i64,
    range: (i64, i64),
    step: f64,
    suffix: &str,
) -> bool {
    ui.add(
        egui::DragValue::new(value)
            .range(range.0..=range.1)
            .speed(step)
            .suffix(suffix),
    )
    .changed()
}
