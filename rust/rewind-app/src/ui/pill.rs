//! The status pill: a tiny always-on-top window that never lands in a clip.
//!
//! The window is a fixed 240x46, exactly like the Python overlay, and the capsule inside
//! is sized to its content and aligned to the chosen corner. It lives in egui's viewport
//! API with no decorations, a transparent background and mouse passthrough, so only the
//! capsule is visible and clicks go to whatever is underneath.
//!
//! Capture exclusion is the one thing egui cannot express: `SetWindowDisplayAffinity` /
//! `NSWindowSharingNone` need the native handle, so the window is found by its title and
//! handed to `overlay_window`, which validates it before touching the OS.
//!
//! Because exclusion also hides the pill from screenshots, `REWIND_PILL_NO_EXCLUDE=1`
//! skips it for manual checks of the pill's own layout.

use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Stroke, StrokeKind, Vec2,
};

use crate::overlay::{self, Accent, Content, Flash, Glyph, IdleState, Pill, Visibility};
use crate::ui::theme;
use rewind_engine::{Mode, Settings, State};

/// The window's size in points. The capsule inside is smaller and content-sized.
const WINDOW_W: f32 = 240.0;
const WINDOW_H: f32 = 46.0;
/// The capsule's height, and its inset from the window's top and bottom.
const BODY_H: f32 = 34.0;
/// Distance from the screen edge, in points. The Python overlay used 4.
const MARGIN: f32 = 4.0;
/// How long the auto-hide fade lasts.
const FADE: f32 = 0.45;
/// How long the capsule takes to resize when the content changes.
const WIDTH_MOVE: f32 = 0.32;
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
    /// The capsule's width animation: from, to, started.
    width: Option<(f32, f32, Instant)>,
    /// The width last drawn, so a retarget can start from it.
    drawn_width: f32,
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
            width: None,
            drawn_width: 0.0,
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
        let flashing = matches!(pill, Pill::Flash(_));
        let idling_live = matches!(
            pill,
            Pill::Idle(IdleState::On { .. } | IdleState::Recording)
        );
        let always = settings.overlay_mode != "auto";
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
        let size = Vec2::new(WINDOW_W * scale, WINDOW_H * scale);
        let position = self.corner_position(&settings.overlay_corner, size, ctx, now);
        let builder = egui::ViewportBuilder::default()
            .with_title(TITLE)
            .with_transparent(true)
            .with_decorations(false)
            .with_always_on_top()
            .with_taskbar(false)
            .with_resizable(false)
            // The Python overlay sets NoDropShadowWindowHint: the capsule paints its own
            // soft shadow, and the window server's would be a black rectangle around it.
            .with_has_shadow(false)
            .with_mouse_passthrough(true)
            .with_inner_size(size)
            .with_position(position);

        let content = overlay::content(pill);
        let right = !settings.overlay_corner.contains("left");
        let width = self.width_for(&content, scale, ctx, now);
        let phase = self
            .flash
            .as_ref()
            .map(|(_, at)| overlay::phase(now.duration_since(*at)))
            .unwrap_or_else(|| overlay::phase(now.duration_since(self.last_activity)));

        // A deferred viewport is its own event loop: nothing repaints it unless this
        // viewport says so. Compare what would be drawn, and ask for a frame only when it
        // differs (plus a steady cadence while an animation runs).
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        use std::hash::{Hash as _, Hasher as _};
        content.title.hash(&mut hash);
        content.detail.hash(&mut hash);
        content.accent.hash(&mut hash);
        content.glyph.hash(&mut hash);
        flashing.hash(&mut hash);
        alpha.to_bits().hash(&mut hash);
        scale.to_bits().hash(&mut hash);
        width.to_bits().hash(&mut hash);
        position.x.to_bits().hash(&mut hash);
        position.y.to_bits().hash(&mut hash);
        let key = hash.finish();
        if self.painted != Some(key) {
            self.painted = Some(key);
            ctx.request_repaint_of(id);
        }
        let animating = flashing || idling_live || visibility == Visibility::ShownTemporarily;
        if animating || self.width.is_some() {
            ctx.request_repaint_after_for(Duration::from_millis(33), id);
        }

        let debug = std::env::var_os("REWIND_PILL_DEBUG").is_some();
        if debug {
            eprintln!(
                "pill: state={state:?} mode={mode:?} {visibility:?} alpha={alpha:.2} width={width:.1} pos={position:?} ppp={}",
                ctx.pixels_per_point()
            );
        }

        ctx.show_viewport_deferred(id, builder, move |ui, _class| {
            draw(
                ui,
                &Paint {
                    content: &content,
                    flashing,
                    live: idling_live,
                    alpha,
                    scale,
                    width,
                    right,
                    phase,
                },
            );
        });
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

    /// The capsule's width this frame, easing towards the content's target width.
    fn width_for(
        &mut self,
        content: &Content,
        scale: f32,
        ctx: &egui::Context,
        now: Instant,
    ) -> f32 {
        let target = target_width(ctx, content, scale);
        match self.width {
            Some((from, to, started)) if (to - target).abs() > 0.5 => {
                // The content changed again: start a new ease from where it is now.
                let current = ease(from, to, started, now);
                self.width = Some((current, target, now));
                self.drawn_width = current;
            }
            Some((from, to, started)) => {
                let t = ((now - started).as_secs_f32() / WIDTH_MOVE).clamp(0.0, 1.0);
                let current = ease(from, to, started, now);
                self.drawn_width = current;
                if t >= 1.0 {
                    self.width = None;
                    self.drawn_width = to;
                }
            }
            None if self.painted.is_none() => {
                // First frame: appear at the right size rather than growing from zero.
                self.drawn_width = target;
            }
            None if (self.drawn_width - target).abs() > 0.5 => {
                self.width = Some((self.drawn_width, target, now));
            }
            None => {}
        }
        self.drawn_width
    }

    /// Where the window's top-left corner goes, in points.
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
}

impl Default for PillWindow {
    fn default() -> Self {
        Self::new()
    }
}

fn ease(from: f32, to: f32, started: Instant, now: Instant) -> f32 {
    let t = ((now - started).as_secs_f32() / WIDTH_MOVE).clamp(0.0, 1.0);
    let eased = 1.0 - (1.0 - t).powi(3);
    from + (to - from) * eased
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

fn title_font(scale: f32) -> FontId {
    FontId::proportional((8.0 * scale).max(7.0))
}

fn detail_font(scale: f32) -> FontId {
    FontId::proportional((11.0 * scale).max(9.0))
}

/// How wide the capsule wants to be: the text plus the icon and padding, capped at the
/// window. Mirrors `_target_width` in `app/overlay.py`.
fn target_width(ctx: &egui::Context, content: &Content, scale: f32) -> f32 {
    let title = spaced_width(ctx, &content.title, &title_font(scale), scale);
    let detail = text_width(ctx, &content.detail, &detail_font(scale));
    let text = title.max(detail);
    (37.0 * scale + text + 13.0 * scale).min(WINDOW_W * scale - 4.0 * scale)
}

fn text_width(ctx: &egui::Context, text: &str, font: &FontId) -> f32 {
    ctx.fonts_mut(|fonts| {
        fonts
            .layout_no_wrap(text.to_string(), font.clone(), Color32::WHITE)
            .size()
            .x
    })
}

/// Width of the title including its 1 pt letter spacing.
fn spaced_width(ctx: &egui::Context, text: &str, font: &FontId, scale: f32) -> f32 {
    text_width(ctx, text, font) + scale * text.chars().count().saturating_sub(1) as f32
}

fn fade(color: Color32, alpha: f32) -> Color32 {
    let alpha = (f32::from(color.a()) * alpha.clamp(0.0, 1.0)) as u8;
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

fn accent_colour(accent: Accent) -> Color32 {
    match accent {
        Accent::Active => theme::ACCENT,
        Accent::Success => theme::GREEN,
        Accent::Warning => theme::WARN,
        Accent::Quiet => Color32::from_rgb(0x77, 0x77, 0x77),
        Accent::Disabled => Color32::from_rgb(0x55, 0x55, 0x55),
    }
}

/// Qt's `QColor.lightness`: the midpoint of the HSL range. Used to pick a glyph colour
/// that stays readable on a filled disc.
fn lightness(color: Color32) -> u8 {
    let max = color.r().max(color.g()).max(color.b());
    let min = color.r().min(color.g()).min(color.b());
    ((u16::from(max) + u16::from(min)) / 2) as u8
}

/// Everything the paint pass needs, gathered so the draw helpers stay short.
struct Paint<'a> {
    content: &'a Content,
    flashing: bool,
    live: bool,
    alpha: f32,
    scale: f32,
    width: f32,
    right: bool,
    phase: f32,
}

fn draw(ui: &mut egui::Ui, p: &Paint<'_>) {
    let Paint {
        content,
        flashing,
        live,
        alpha,
        scale,
        width,
        right,
        phase,
    } = *p;
    let painter = ui.painter();
    let window = ui.max_rect();
    let height = BODY_H * scale;
    let top = window.top() + (window.height() - height) / 2.0;
    let left = if right {
        window.right() - width - scale
    } else {
        window.left() + 2.0 * scale
    };
    let body = Rect::from_min_size(egui::pos2(left, top), Vec2::new(width, height));
    let radius = CornerRadius::same((height / 2.0) as u8);
    let accent = accent_colour(content.accent);

    // Two soft shadows under the capsule, as the Python overlay draws them.
    for (spread, opacity) in [(6.0, 18u8), (3.0, 30u8)] {
        let grow = spread * scale;
        painter.rect_filled(
            Rect::from_min_max(
                body.min + Vec2::new(-grow * 0.5, -grow * 0.3),
                body.max + Vec2::new(grow * 0.5, grow * 0.7),
            ),
            CornerRadius::same((height / 2.0 + grow) as u8),
            fade(Color32::BLACK, f32::from(opacity) / 255.0 * alpha),
        );
    }

    painter.rect_filled(
        body,
        radius,
        fade(Color32::from_rgb(22, 22, 22), 0.933 * alpha),
    );
    painter.rect_stroke(
        body,
        radius,
        Stroke::new(1.0, fade(Color32::from_white_alpha(26), alpha)),
        StrokeKind::Inside,
    );

    let disc = 24.0 * scale;
    let centre = egui::pos2(left + 17.0 * scale, top + height / 2.0);

    // The live "on"/"recording" states breathe a halo around the disc.
    if live {
        let pulse = (phase * 0.9) % 1.0;
        painter.circle_filled(
            centre,
            disc / 2.0 + 5.0 * scale * pulse,
            fade(accent, 0.40 * (1.0 - pulse) * alpha),
        );
    }

    painter.circle_filled(centre, disc / 2.0, fade(accent, alpha));
    // The Python overlay keeps the check and spinner white and only darkens the mark
    // that sits alone on a light disc; the bang is always dark.
    let glyph_colour = match content.glyph {
        Glyph::Rewind if lightness(accent) > 150 => Color32::from_rgb(0x11, 0x11, 0x11),
        Glyph::Bang => Color32::from_rgb(0x1A, 0x13, 0x05),
        _ => Color32::WHITE,
    };
    draw_glyph(
        painter,
        content.glyph,
        centre,
        disc / 2.0,
        &GlyphPaint {
            colour: fade(glyph_colour, alpha),
            alpha,
            phase,
            scale,
        },
    );

    let text_x = left + 37.0 * scale;
    let title_colour = if flashing { accent } else { theme::MUTED };
    draw_spaced(
        painter,
        egui::pos2(text_x, top + 9.5 * scale),
        &content.title,
        title_font(scale),
        fade(title_colour, alpha),
        scale,
    );
    painter.text(
        egui::pos2(text_x, top + 22.0 * scale),
        Align2::LEFT_CENTER,
        &content.detail,
        detail_font(scale),
        fade(theme::TEXT, alpha),
    );
}

/// Text with the Python overlay's 1 pt letter spacing, drawn one glyph at a time.
fn draw_spaced(
    painter: &egui::Painter,
    pos: Pos2,
    text: &str,
    font: FontId,
    colour: Color32,
    scale: f32,
) {
    let mut x = pos.x;
    for ch in text.chars() {
        let glyph = ch.to_string();
        painter.text(
            egui::pos2(x, pos.y),
            Align2::LEFT_CENTER,
            &glyph,
            font.clone(),
            colour,
        );
        let width = painter.layout_no_wrap(glyph, font.clone(), colour).size().x;
        x += width + scale;
    }
}

/// The glyph's colours and animation state.
struct GlyphPaint {
    colour: Color32,
    alpha: f32,
    phase: f32,
    scale: f32,
}

fn draw_glyph(painter: &egui::Painter, glyph: Glyph, centre: Pos2, radius: f32, p: &GlyphPaint) {
    let GlyphPaint {
        colour,
        alpha,
        phase,
        scale,
    } = *p;
    match glyph {
        Glyph::Rewind => theme::glyph(painter, centre, radius * 2.0 * 0.42, colour),
        Glyph::Check => {
            let s = radius * 2.0 * 0.2;
            let points = vec![
                centre + Vec2::new(-s * 1.05, s * 0.05),
                centre + Vec2::new(-s * 0.25, s * 0.85),
                centre + Vec2::new(s * 1.15, -s * 0.75),
            ];
            let width = 2.0 * scale;
            painter.add(egui::Shape::line(
                points.clone(),
                Stroke::new(width, colour),
            ));
            painter.circle_filled(points[0], width / 2.0, colour);
            painter.circle_filled(points[2], width / 2.0, colour);
        }
        Glyph::Bang => {
            painter.rect_filled(
                Rect::from_min_size(
                    centre + Vec2::new(-1.1 * scale, -5.2 * scale),
                    Vec2::new(2.2 * scale, 6.8 * scale),
                ),
                CornerRadius::same((1.1 * scale) as u8),
                fade(Color32::from_rgb(0x1A, 0x13, 0x05), alpha),
            );
            painter.circle_filled(
                centre + Vec2::new(0.0, 3.95 * scale),
                1.35 * scale,
                fade(Color32::from_rgb(0x1A, 0x13, 0x05), alpha),
            );
        }
        Glyph::Spinner => {
            // A 260-degree arc, turning with the phase, as `drawArc` does.
            let r = radius * 2.0 * 0.26;
            let start = -((phase * 360.0 * 1.6) % 360.0).to_radians();
            let segments = 24;
            let points: Vec<Pos2> = (0..=segments)
                .map(|i| {
                    let angle = start - (i as f32 / segments as f32) * 260.0_f32.to_radians();
                    centre + Vec2::new(r * angle.cos(), r * angle.sin())
                })
                .collect();
            let width = 1.8 * scale;
            painter.add(egui::Shape::line(
                points.clone(),
                Stroke::new(width, colour),
            ));
            painter.circle_filled(points[0], width / 2.0, colour);
            painter.circle_filled(*points.last().unwrap_or(&centre), width / 2.0, colour);
        }
    }
}
