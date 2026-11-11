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
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_max_width(text_width);
                    ui.label(RichText::new(title).size(13.5).strong());
                    if let Some(sub) = sub.filter(|text| !text.is_empty()) {
                        ui.add_space(2.0);
                        ui.label(RichText::new(sub).size(11.5).color(sub_color));
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
    let (rect, response) = ui.allocate_exact_size(Vec2::new(40.0, 22.0), Sense::click());
    if response.clicked() {
        *on = !*on;
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let t = ui.ctx().animate_bool(response.id, *on);
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        egui::CornerRadius::same(11),
        if *on { theme::ACCENT } else { theme::FIELD },
    );
    let knob_x = rect.left() + 11.0 + (rect.width() - 22.0) * t;
    painter.circle_filled(
        egui::pos2(knob_x, rect.center().y),
        8.0,
        if *on {
            Color32::from_rgb(0x1B, 0x1B, 0x1B)
        } else {
            theme::MUTED
        },
    );
    response.clicked()
}

/// A dropdown over fixed labels; returns the index the user picked, if any.
pub fn combo(ui: &mut egui::Ui, id: &str, labels: &[String], current: usize) -> Option<usize> {
    let mut chosen = None;
    egui::ComboBox::from_id_salt(id)
        .selected_text(labels.get(current).cloned().unwrap_or_default())
        .width(230.0)
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
pub fn slider(
    ui: &mut egui::Ui,
    value: &mut i64,
    range: (i64, i64),
    step: i64,
    suffix: &str,
) -> bool {
    ui.spacing_mut().slider_width = 190.0;
    // Row controls sit in a right-to-left layout, so the readout is added before the
    // track: that renders as [track][value], the way the Python app's SliderControl does.
    ui.add_sized(
        [86.0, 18.0],
        egui::Label::new(egui::RichText::new(format!("{value}{suffix}")).size(12.5)),
    );
    let mut scaled = *value as f64;
    let changed = ui
        .add(
            egui::Slider::new(&mut scaled, range.0 as f64..=range.1 as f64)
                .step_by(step as f64)
                .show_value(false),
        )
        .changed();
    if changed {
        *value = scaled.round() as i64;
    }
    changed
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
