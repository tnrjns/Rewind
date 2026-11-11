//! The status pill: a tiny always-on-top window that never lands in a clip.
//!
//! The window is created through egui's viewport API with no decorations, a transparent
//! background and mouse passthrough, so only the rounded card is visible and clicks go to
//! whatever is underneath. `App::clear_color` returns transparent, which lets this
//! viewport show the desktop while the main window stays covered by its panels.
//!
//! Capture exclusion is the one thing egui cannot express: `SetWindowDisplayAffinity` /
//! `NSWindowSharingNone` need the native handle, so the window is found by its title and
//! handed to `overlay_window`, which validates it before touching the OS.
//!
//! Because exclusion also hides the pill from screenshots, `REWIND_PILL_NO_EXCLUDE=1`
//! skips it for manual checks of the pill's own layout.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Stroke, StrokeKind, Vec2};

use crate::overlay::{self, Accent, Content, Flash, Glyph, IdleState, Pill, Visibility};
use crate::ui::theme;
use rewind_engine::{Mode, Settings, State};

/// Base size in points, before the size setting scales it.
const BASE_W: f32 = 240.0;
const BASE_H: f32 = 46.0;
/// Distance from the screen edge, in points.
const MARGIN: f32 = 4.0;
/// How long the auto-hide fade lasts.
const FADE: f32 = 0.45;
/// Title used to find the native window for capture exclusion.
pub const TITLE: &str = "Rewind status";

pub struct PillWindow {
    /// The message being flashed, and when it started.
    flash: Option<(Flash, Instant)>,
    /// When something last happened, for the auto-hide countdown.
    last_activity: Instant,
    excluded: bool,
    /// Test switch: leave the window capturable so a screenshot can show it.
    no_exclude: bool,
    hidden: bool,
    /// The screen the pill is positioned in, refreshed occasionally: enumerating
    /// monitors costs a system call, and none of them move often.
    screen: Option<(egui::Rect, Instant)>,
    /// What the child window last painted. A deferred viewport only repaints when it is
    /// asked to, so this is compared each frame and a new key means "ask it now".
    painted: Option<u64>,
}

impl PillWindow {
    pub fn new() -> PillWindow {
        PillWindow {
            flash: None,
            last_activity: Instant::now(),
            excluded: false,
            no_exclude: std::env::var_os("REWIND_PILL_NO_EXCLUDE").is_some(),
            hidden: false,
            screen: None,
            painted: None,
        }
    }

    pub fn flash(&mut self, flash: Flash, now: Instant) {
        self.flash = Some((flash, now));
        self.last_activity = now;
    }

    /// Something worth showing happened (a state change, a tray action).
    pub fn note_activity(&mut self, now: Instant) {
        self.last_activity = now;
    }

    /// Draw the pill, or leave it hidden.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        settings: &Settings,
        state: State,
        mode: Mode,
        now: Instant,
    ) {
        let id = egui::ViewportId::from_hash_of("rewind-status-pill");

        let pill = self.current(state, mode, settings.clip_seconds, now);
        let always = settings.overlay_mode != "auto";
        let flashing = self
            .flash
            .as_ref()
            .is_some_and(|(flash, at)| now.duration_since(*at) < flash.duration());
        let activity_age = now.duration_since(self.last_activity).as_secs_f32();
        let visibility = overlay::visibility(
            settings.overlay_enabled,
            always,
            flashing,
            activity_age < overlay::AUTO_HIDE_AFTER.as_secs_f32(),
        );

        let alpha = match visibility {
            Visibility::Hidden => {
                if !self.hidden {
                    ctx.send_viewport_cmd_to(id, egui::ViewportCommand::Visible(false));
                    self.hidden = true;
                }
                return;
            }
            Visibility::Shown => 1.0,
            Visibility::ShownTemporarily => {
                let left = overlay::AUTO_HIDE_AFTER.as_secs_f32() - activity_age;
                (left / FADE).clamp(0.0, 1.0)
            }
        } * (settings.overlay_opacity as f32 / 100.0);

        if self.hidden {
            ctx.send_viewport_cmd_to(id, egui::ViewportCommand::Visible(true));
            self.hidden = false;
        }
        if !self.excluded && !self.no_exclude {
            self.excluded = exclude_by_title();
        }

        let scale = settings.overlay_scale as f32 / 100.0;
        let size = Vec2::new(BASE_W * scale, BASE_H * scale);
        let position = self.corner_position(&settings.overlay_corner, size, ctx, now);
        let builder = egui::ViewportBuilder::default()
            .with_title(TITLE)
            .with_transparent(true)
            .with_decorations(false)
            .with_always_on_top()
            .with_taskbar(false)
            .with_resizable(false)
            .with_mouse_passthrough(true)
            .with_inner_size(size)
            .with_position(position);

        let content = overlay::content(pill);
        let phase = self
            .flash
            .as_ref()
            .map(|(_, at)| overlay::phase(now.duration_since(*at)))
            .unwrap_or_else(|| overlay::phase(now.duration_since(self.last_activity)));

        let debug = std::env::var_os("REWIND_PILL_DEBUG").is_some();
        if debug {
            eprintln!(
                "pill: state={state:?} mode={mode:?} {visibility:?} alpha={alpha:.2} pos={position:?} size={size:?} ppp={}",
                ctx.pixels_per_point()
            );
        }
        // A deferred viewport is its own event loop: nothing repaints it unless this
        // viewport says so. Compare what would be drawn, and ask for a frame only when it
        // differs (plus a steady cadence while an animation runs).
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        use std::hash::{Hash as _, Hasher as _};
        content.title.hash(&mut hash);
        content.detail.hash(&mut hash);
        alpha.to_bits().hash(&mut hash);
        scale.to_bits().hash(&mut hash);
        position.x.to_bits().hash(&mut hash);
        position.y.to_bits().hash(&mut hash);
        let key = hash.finish();
        if self.painted != Some(key) {
            self.painted = Some(key);
            ctx.request_repaint_of(id);
        }
        if flashing || visibility == Visibility::ShownTemporarily {
            ctx.request_repaint_after_for(Duration::from_millis(33), id);
        }

        ctx.show_viewport_deferred(id, builder, move |ui, _class| {
            if debug {
                eprintln!("pill: drawing in {:?}", ui.ctx().pixels_per_point());
            }
            draw(ui, &content, alpha, scale, phase);
        });
    }

    /// Where the pill's top-left corner goes, in points.
    fn corner_position(
        &mut self,
        corner: &str,
        size: Vec2,
        ctx: &egui::Context,
        now: Instant,
    ) -> Pos2 {
        let screen = match self.screen {
            Some((rect, at)) if now.duration_since(at) < Duration::from_secs(2) => rect,
            _ => {
                let rect = screen_rect(ctx);
                self.screen = Some((rect, now));
                rect
            }
        };
        match corner {
            "bottom-left" => egui::pos2(screen.left() + MARGIN, screen.bottom() - size.y - MARGIN),
            "top-right" => egui::pos2(screen.right() - size.x - MARGIN, screen.top() + MARGIN),
            "top-left" => egui::pos2(screen.left() + MARGIN, screen.top() + MARGIN),
            _ => egui::pos2(
                screen.right() - size.x - MARGIN,
                screen.bottom() - size.y - MARGIN,
            ),
        }
    }

    /// A live flash outranks the idle state; an expired one is dropped.
    fn current(&mut self, state: State, mode: Mode, clip_seconds: u32, now: Instant) -> Pill {
        if let Some((flash, at)) = &self.flash {
            if now.duration_since(*at) < flash.duration() {
                return Pill::Flash(flash.clone());
            }
        }
        self.flash = None;
        Pill::Idle(idle_state(state, mode, clip_seconds))
    }
}

impl Default for PillWindow {
    fn default() -> Self {
        Self::new()
    }
}

fn idle_state(state: State, mode: Mode, clip_seconds: u32) -> IdleState {
    if state == State::Saving {
        return IdleState::SavingRecording;
    }
    if mode == Mode::Recording {
        return match state {
            State::Starting => IdleState::Starting,
            _ => IdleState::Recording,
        };
    }
    match state {
        State::On => IdleState::On { clip_seconds },
        State::Starting => IdleState::Starting,
        _ => IdleState::Off,
    }
}

/// The primary display's usable area, in points.
fn screen_rect(ctx: &egui::Context) -> egui::Rect {
    let pixels_per_point = ctx.pixels_per_point().max(0.1);
    let (x, y, w, h) = match crate::overlay_window::work_area() {
        Some(area) => area,
        None => {
            let monitors = rewind_engine::list_monitors();
            monitors
                .iter()
                .find(|m| m.primary)
                .or_else(|| monitors.first())
                .map(|m| (m.x, m.y, m.w as i32, m.h as i32))
                .unwrap_or((0, 0, 1920, 1080))
        }
    };
    let left = x as f32 / pixels_per_point;
    let top = y as f32 / pixels_per_point;
    let right = (x as i64 + i64::from(w)) as f32 / pixels_per_point;
    let bottom = (y as i64 + i64::from(h)) as f32 / pixels_per_point;
    egui::Rect::from_min_max(egui::pos2(left, top), egui::pos2(right, bottom))
}

/// Find the pill's native window by title and exclude it from capture.
fn exclude_by_title() -> bool {
    let Some(handle) = crate::overlay_window::window_handle_by_title(TITLE) else {
        return false;
    };
    crate::overlay_window::register_window(handle);
    crate::overlay_window::exclude_from_capture(handle)
}

fn fade(color: Color32, alpha: f32) -> Color32 {
    let alpha = (f32::from(color.a()) * alpha.clamp(0.0, 1.0)) as u8;
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

fn accent_colour(accent: Accent, alpha: f32) -> Color32 {
    fade(
        match accent {
            Accent::Active => theme::ACCENT,
            Accent::Success => theme::GREEN,
            Accent::Warning => theme::WARN,
            Accent::Quiet => theme::SUBTLE,
        },
        alpha,
    )
}

fn draw(ui: &mut egui::Ui, content: &Content, alpha: f32, scale: f32, phase: f32) {
    let rect = ui.max_rect();
    let painter = ui.painter();
    let radius = CornerRadius::same((14.0 * scale) as u8);
    painter.rect_filled(rect, radius, fade(theme::CARD, alpha));
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, fade(theme::BORDER, alpha)),
        StrokeKind::Inside,
    );

    let inset = 10.0 * scale;
    let disc = 26.0 * scale;
    let centre = egui::pos2(rect.left() + inset + disc / 2.0, rect.center().y);
    let accent = accent_colour(content.accent, alpha);
    painter.circle_filled(centre, disc / 2.0, fade(theme::CARD_2, alpha));

    // The active state breathes; the other states stay still.
    if content.accent == Accent::Active {
        let pulse = 0.5 + 0.5 * (phase * std::f32::consts::TAU).sin();
        painter.circle_stroke(
            centre,
            disc / 2.0 + 1.5 * scale + 2.5 * scale * pulse,
            Stroke::new(1.0, fade(accent, alpha * (0.15 + 0.35 * pulse))),
        );
    }

    draw_glyph(painter, content.glyph, centre, disc / 2.0, accent, phase);

    let text_x = rect.left() + inset + disc + 10.0 * scale;
    painter.text(
        egui::pos2(text_x, rect.center().y - 9.0 * scale),
        Align2::LEFT_CENTER,
        &content.title,
        FontId::proportional(12.5 * scale),
        fade(theme::TEXT, alpha),
    );
    painter.text(
        egui::pos2(text_x, rect.center().y + 9.0 * scale),
        Align2::LEFT_CENTER,
        &content.detail,
        FontId::proportional(11.0 * scale),
        fade(theme::MUTED, alpha),
    );
}

fn draw_glyph(
    painter: &egui::Painter,
    glyph: Glyph,
    centre: Pos2,
    radius: f32,
    colour: Color32,
    phase: f32,
) {
    match glyph {
        Glyph::Rewind => {
            // The app mark: two left-pointing triangles.
            let height = radius * 0.9;
            let width = height * 0.62;
            let gap = width * 0.92;
            let left = centre.x - (gap + width) / 2.0;
            for step in 0..2 {
                let x = left + gap * step as f32;
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        egui::pos2(x, centre.y),
                        egui::pos2(x + width, centre.y - height / 2.0),
                        egui::pos2(x + width, centre.y + height / 2.0),
                    ],
                    colour,
                    Stroke::NONE,
                ));
            }
        }
        Glyph::Check => {
            painter.add(egui::Shape::line(
                vec![
                    centre + Vec2::new(-0.42, 0.02) * radius,
                    centre + Vec2::new(-0.12, 0.34) * radius,
                    centre + Vec2::new(0.46, -0.32) * radius,
                ],
                Stroke::new(2.2 * (radius / 16.0) * 2.0, colour),
            ));
        }
        Glyph::Bang => {
            painter.rect_filled(
                egui::Rect::from_center_size(
                    centre + Vec2::new(0.0, -0.18) * radius,
                    Vec2::new(3.4, radius * 0.9),
                ),
                CornerRadius::same(2),
                colour,
            );
            painter.circle_filled(centre + Vec2::new(0.0, 0.42) * radius, 1.8, colour);
        }
        Glyph::Spinner => {
            // A three-quarter arc that turns with the phase.
            let segments = 24;
            let start = phase * std::f32::consts::TAU;
            let points: Vec<Pos2> = (0..=segments)
                .map(|i| {
                    let angle = start + (i as f32 / segments as f32) * 1.5 * std::f32::consts::PI;
                    centre + Vec2::angled(angle) * radius * 0.72
                })
                .collect();
            painter.add(egui::Shape::line(
                points,
                Stroke::new(2.2 * (radius / 16.0) * 2.0, colour),
            ));
        }
    }
}
