//! Panoraminal's look: a palette taken from the app icons, dark/light variants,
//! gradients and neon glow helpers.

use eframe::egui::{self, epaint, Color32, CornerRadius, Pos2, Rect, Shadow, Stroke, Vec2};

/// Colours sampled from `panoraminal_icon` / `panoraminal_favicon`.
pub mod brand {
    use eframe::egui::Color32;
    pub const DEEP: Color32 = Color32::from_rgb(0x01, 0x30, 0xa0);
    pub const ROYAL: Color32 = Color32::from_rgb(0x06, 0x60, 0xe9);
    /// Not used directly by the UI; kept documented for the README palette.
    #[allow(dead_code)]
    pub const ELECTRIC: Color32 = Color32::from_rgb(0x00, 0x39, 0xe8);
    pub const AZURE: Color32 = Color32::from_rgb(0x1b, 0x99, 0xf9);
    pub const CYAN: Color32 = Color32::from_rgb(0x5f, 0xd0, 0xfd);
    pub const ICE: Color32 = Color32::from_rgb(0xa1, 0xdb, 0xfb);
    pub const SNOW: Color32 = Color32::from_rgb(0xe8, 0xf4, 0xfa);
    /// The brand gradient, left to right.
    #[allow(dead_code)]
    pub const GRADIENT: [Color32; 4] = [DEEP, ROYAL, AZURE, CYAN];
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    pub bg: Color32,
    pub panel: Color32,
    pub panel_hi: Color32,
    pub raised: Color32,
    pub stroke: Color32,
    pub text: Color32,
    pub text_weak: Color32,
    pub accent: Color32,
    pub accent2: Color32,
    pub neon: Color32,
    pub danger: Color32,
    pub ok: Color32,
    pub canvas_bg: Color32,
    pub header_a: Color32,
    pub header_b: Color32,
}

impl Palette {
    pub fn dark() -> Self {
        Self {
            dark: true,
            bg: Color32::from_rgb(0x06, 0x0a, 0x18),
            panel: Color32::from_rgb(0x0b, 0x14, 0x30),
            panel_hi: Color32::from_rgb(0x10, 0x1d, 0x42),
            raised: Color32::from_rgb(0x16, 0x27, 0x57),
            stroke: Color32::from_rgb(0x22, 0x3a, 0x78),
            text: brand::SNOW,
            text_weak: Color32::from_rgb(0x8f, 0xb0, 0xd8),
            accent: brand::AZURE,
            accent2: brand::ROYAL,
            neon: brand::CYAN,
            danger: Color32::from_rgb(0xff, 0x5c, 0x7a),
            ok: Color32::from_rgb(0x4d, 0xe0, 0xb0),
            canvas_bg: Color32::from_rgb(0x04, 0x07, 0x12),
            header_a: Color32::from_rgb(0x08, 0x12, 0x36),
            header_b: Color32::from_rgb(0x0a, 0x24, 0x66),
        }
    }
    pub fn light() -> Self {
        Self {
            dark: false,
            bg: Color32::from_rgb(0xe6, 0xf0, 0xfb),
            panel: Color32::from_rgb(0xf6, 0xfa, 0xff),
            panel_hi: Color32::from_rgb(0xe7, 0xf1, 0xfc),
            raised: Color32::from_rgb(0xff, 0xff, 0xff),
            stroke: Color32::from_rgb(0xbf, 0xd5, 0xf0),
            text: Color32::from_rgb(0x0b, 0x1a, 0x3a),
            text_weak: Color32::from_rgb(0x4a, 0x63, 0x90),
            accent: brand::ROYAL,
            accent2: Color32::from_rgb(0x00, 0x58, 0xfc),
            neon: brand::AZURE,
            danger: Color32::from_rgb(0xd6, 0x2a, 0x4e),
            ok: Color32::from_rgb(0x11, 0x9a, 0x6e),
            canvas_bg: Color32::from_rgb(0xd5, 0xe4, 0xf5),
            header_a: Color32::from_rgb(0xf9, 0xfc, 0xff),
            header_b: Color32::from_rgb(0xd9, 0xea, 0xfd),
        }
    }
}

/// Glow intensity chosen in the settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum Glow {
    Off,
    Reduced,
    #[default]
    Full,
}

impl Glow {
    pub fn factor(self) -> f32 {
        match self {
            Glow::Off => 0.0,
            Glow::Reduced => 0.45,
            Glow::Full => 1.0,
        }
    }
}

pub fn with_alpha(c: Color32, a: f32) -> Color32 {
    let a = (a.clamp(0.0, 1.0) * c.a() as f32) as u8;
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(
        l(a.r(), b.r()),
        l(a.g(), b.g()),
        l(a.b(), b.b()),
        l(a.a(), b.a()),
    )
}

/// Samples a multi-stop gradient at `t` in `[0,1]`.
pub fn gradient_at(stops: &[Color32], t: f32) -> Color32 {
    if stops.len() == 1 {
        return stops[0];
    }
    let t = t.clamp(0.0, 1.0) * (stops.len() - 1) as f32;
    let i = (t.floor() as usize).min(stops.len() - 2);
    lerp_color(stops[i], stops[i + 1], t - i as f32)
}

pub fn neon_shadow(p: &Palette, glow: Glow, strength: f32) -> Shadow {
    let f = glow.factor() * strength;
    if f <= 0.0 {
        return Shadow::NONE;
    }
    Shadow {
        offset: [0, 0],
        blur: (18.0 * f) as u8,
        spread: (1.0 * f) as u8,
        color: with_alpha(p.neon, if p.dark { 0.55 } else { 0.35 } * f),
    }
}

/// Builds egui visuals from a palette.
pub fn visuals(p: &Palette, glow: Glow) -> egui::Visuals {
    let mut v = if p.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let r = CornerRadius::same(8);
    v.override_text_color = Some(p.text);
    v.weak_text_color = Some(p.text_weak);
    v.hyperlink_color = p.accent;
    v.panel_fill = p.panel;
    v.window_fill = p.panel;
    v.faint_bg_color = p.panel_hi;
    v.extreme_bg_color = if p.dark {
        Color32::from_rgb(0x05, 0x0b, 0x22)
    } else {
        Color32::WHITE
    };
    v.code_bg_color = p.panel_hi;
    v.window_stroke = Stroke::new(1.0, with_alpha(p.accent, 0.55));
    v.window_corner_radius = CornerRadius::same(14);
    v.menu_corner_radius = CornerRadius::same(10);
    let mut ws = neon_shadow(p, glow, 1.0);
    if glow == Glow::Off {
        ws = Shadow {
            offset: [0, 6],
            blur: 18,
            spread: 0,
            color: Color32::from_black_alpha(if p.dark { 120 } else { 40 }),
        };
    }
    v.window_shadow = ws;
    v.popup_shadow = Shadow {
        blur: ws.blur / 2,
        ..ws
    };
    v.selection.bg_fill = with_alpha(p.accent, 0.55);
    v.selection.stroke = Stroke::new(1.0, p.neon);
    v.warn_fg_color = Color32::from_rgb(0xff, 0xc1, 0x4d);
    v.error_fg_color = p.danger;
    v.slider_trailing_fill = true;
    v.striped = false;
    v.handle_shape = egui::style::HandleShape::Circle;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.panel;
    w.noninteractive.weak_bg_fill = p.panel;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.stroke);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = r;

    w.inactive.bg_fill = p.raised;
    w.inactive.weak_bg_fill = p.raised;
    w.inactive.bg_stroke = Stroke::new(1.0, p.stroke);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = r;

    w.hovered.bg_fill = lerp_color(p.raised, p.accent, 0.25);
    w.hovered.weak_bg_fill = lerp_color(p.raised, p.accent, 0.22);
    w.hovered.bg_stroke = Stroke::new(1.0, p.neon);
    w.hovered.fg_stroke = Stroke::new(1.5, p.text);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 1.0;

    w.active.bg_fill = p.accent2;
    w.active.weak_bg_fill = p.accent2;
    w.active.bg_stroke = Stroke::new(1.5, p.neon);
    w.active.fg_stroke = Stroke::new(2.0, Color32::WHITE);
    w.active.corner_radius = r;

    w.open.bg_fill = lerp_color(p.raised, p.accent, 0.3);
    w.open.weak_bg_fill = lerp_color(p.raised, p.accent, 0.3);
    w.open.bg_stroke = Stroke::new(1.0, p.accent);
    w.open.fg_stroke = Stroke::new(1.0, p.text);
    w.open.corner_radius = r;
    v
}

pub fn apply_style(ctx: &egui::Context, p: &Palette, glow: Glow, animations: bool) {
    let vis = visuals(p, glow);
    ctx.global_style_mut(|s| {
        s.visuals = vis.clone();
        s.animation_time = if animations { 0.18 } else { 0.0 };
        s.spacing.item_spacing = Vec2::new(8.0, 6.0);
        s.spacing.button_padding = Vec2::new(8.0, 4.0);
        s.spacing.slider_width = 150.0;
        s.spacing.interact_size.y = 22.0;
        s.spacing.window_margin = egui::Margin::same(14);
        s.spacing.menu_margin = egui::Margin::same(8);
        s.interaction.tooltip_delay = 0.35;
    });
}

/// A quad with a horizontal (or vertical) colour gradient.
pub fn gradient_rect(painter: &egui::Painter, rect: Rect, stops: &[Color32], horizontal: bool) {
    let mut mesh = epaint::Mesh::default();
    let n = stops.len().max(2);
    for i in 0..n {
        let t = i as f32 / (n - 1) as f32;
        let c = gradient_at(stops, t);
        let (a, b) = if horizontal {
            (
                Pos2::new(rect.left() + rect.width() * t, rect.top()),
                Pos2::new(rect.left() + rect.width() * t, rect.bottom()),
            )
        } else {
            (
                Pos2::new(rect.left(), rect.top() + rect.height() * t),
                Pos2::new(rect.right(), rect.top() + rect.height() * t),
            )
        };
        mesh.colored_vertex(a, c);
        mesh.colored_vertex(b, c);
        if i > 0 {
            let k = (i * 2) as u32;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k, k + 1);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// A rounded rectangle filled with a gradient (approximated by clipping corners
/// with a rounded fill underneath; good enough for buttons and chips).
pub fn gradient_pill(painter: &egui::Painter, rect: Rect, stops: &[Color32], radius: f32) {
    let r = radius.min(rect.height() * 0.5).min(rect.width() * 0.5);
    painter.rect_filled(
        Rect::from_min_max(rect.min, Pos2::new(rect.left() + r * 2.0, rect.bottom())),
        r,
        stops[0],
    );
    painter.rect_filled(
        Rect::from_min_max(Pos2::new(rect.right() - r * 2.0, rect.top()), rect.max),
        r,
        *stops.last().unwrap(),
    );
    gradient_rect(
        painter,
        Rect::from_min_max(
            Pos2::new(rect.left() + r, rect.top()),
            Pos2::new(rect.right() - r, rect.bottom()),
        ),
        stops,
        true,
    );
}

/// Soft neon glow around a rect, drawn as concentric translucent strokes.
pub fn glow_rect(painter: &egui::Painter, rect: Rect, radius: f32, color: Color32, strength: f32) {
    if strength <= 0.01 {
        return;
    }
    let layers = 6;
    for i in (1..=layers).rev() {
        let k = i as f32 / layers as f32;
        let a = (1.0 - k) * 0.35 * strength + 0.02 * strength;
        painter.rect_stroke(
            rect.expand(k * 7.0),
            radius + k * 7.0,
            Stroke::new(2.2, with_alpha(color, a)),
            egui::StrokeKind::Middle,
        );
    }
}

/// Glow around a circle.
pub fn glow_circle(painter: &egui::Painter, c: Pos2, r: f32, color: Color32, strength: f32) {
    if strength <= 0.01 {
        return;
    }
    for i in (1..=6).rev() {
        let k = i as f32 / 6.0;
        painter.circle_stroke(
            c,
            r + k * 8.0,
            Stroke::new(
                2.2,
                with_alpha(color, (1.0 - k) * 0.4 * strength + 0.02 * strength),
            ),
        );
    }
}

/// Text whose characters follow the brand gradient (for titles).
pub fn gradient_text(
    text: &str,
    font: egui::FontId,
    stops: &[Color32],
    phase: f32,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let n = text.chars().count().max(1) as f32;
    for (i, ch) in text.chars().enumerate() {
        let t = ((i as f32 / n) + phase).rem_euclid(2.0);
        let t = if t > 1.0 { 2.0 - t } else { t };
        let mut buf = [0u8; 4];
        job.append(
            ch.encode_utf8(&mut buf),
            0.0,
            egui::TextFormat {
                font_id: font.clone(),
                color: gradient_at(stops, t),
                ..Default::default()
            },
        );
    }
    job
}

/// Card frame used by side panel sections.
pub fn card(p: &Palette) -> egui::Frame {
    egui::Frame::NONE
        .fill(if p.dark {
            with_alpha(p.panel_hi, 0.9)
        } else {
            p.raised
        })
        .stroke(Stroke::new(1.0, with_alpha(p.stroke, 0.9)))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(egui::Margin::same(10))
}
