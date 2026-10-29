//! Application icons (embedded PNGs) and hand-drawn vector glyphs for tools.

use eframe::egui::{self, Color32, Painter, Pos2, Rect, Shape, Stroke};

pub static ICON_256: &[u8] = include_bytes!("../assets/icons/panoraminal_icon_256.png");
pub static ICON_128: &[u8] = include_bytes!("../assets/icons/panoraminal_icon_128.png");
pub static FAVICON_256: &[u8] = include_bytes!("../assets/icons/panoraminal_favicon_256.png");
pub static FAVICON_32: &[u8] = include_bytes!("../assets/icons/panoraminal_favicon_32.png");

fn decode(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .expect("embedded icon is a valid PNG")
        .to_rgba8();
    let (w, h) = img.dimensions();
    (w, h, img.into_raw())
}

/// Small titlebar / taskbar icon of the window (the favicon).
pub fn favicon_icon_data() -> egui::IconData {
    let (width, height, rgba) = decode(FAVICON_256);
    egui::IconData {
        rgba,
        width,
        height,
    }
}

pub fn texture(ctx: &egui::Context, name: &str, bytes: &[u8]) -> egui::TextureHandle {
    let (w, h, px) = decode(bytes);
    let img = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &px);
    ctx.load_texture(name, img, egui::TextureOptions::LINEAR)
}

pub struct Icons {
    pub logo: egui::TextureHandle,
    pub logo_small: egui::TextureHandle,
    pub favicon: egui::TextureHandle,
}

impl Icons {
    pub fn load(ctx: &egui::Context) -> Self {
        Self {
            logo: texture(ctx, "icon256", ICON_256),
            logo_small: texture(ctx, "icon128", ICON_128),
            favicon: texture(ctx, "favicon32", FAVICON_32),
        }
    }
}

/// Vector glyphs, drawn into a square `rect`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Pen,
    Brush,
    Airbrush,
    Eraser,
    Eyedropper,
    Bucket,
    Eye,
    EyeOff,
    Lock,
    Unlock,
    Plus,
    Duplicate,
    Trash,
    MergeDown,
    Up,
    Down,
    Sphere,
    Flat,
    Grid,
    Sun,
    Moon,
    Help,
    Swap,
    Undo,
    Redo,
    Clear,
}

pub fn draw(p: &Painter, rect: Rect, g: Glyph, color: Color32) {
    let c = rect.center();
    let s = rect.width().min(rect.height()) / 24.0; // glyphs are designed on a 24-unit grid
    let pt = |x: f32, y: f32| Pos2::new(c.x + (x - 12.0) * s, c.y + (y - 12.0) * s);
    let st = Stroke::new(1.7 * s.max(0.7), color);
    let line = |pts: &[(f32, f32)]| {
        p.add(Shape::line(
            pts.iter().map(|&(x, y)| pt(x, y)).collect(),
            st,
        ));
    };
    let closed = |pts: &[(f32, f32)]| {
        p.add(Shape::closed_line(
            pts.iter().map(|&(x, y)| pt(x, y)).collect(),
            st,
        ));
    };
    let fill = |pts: &[(f32, f32)]| {
        p.add(Shape::convex_polygon(
            pts.iter().map(|&(x, y)| pt(x, y)).collect(),
            color,
            Stroke::NONE,
        ));
    };
    match g {
        Glyph::Pen => {
            closed(&[
                (5.0, 19.0),
                (6.5, 14.5),
                (16.0, 5.0),
                (19.0, 8.0),
                (9.5, 17.5),
            ]);
            line(&[(14.0, 7.0), (17.0, 10.0)]);
            fill(&[(5.0, 19.0), (6.0, 16.0), (8.0, 18.0)]);
        }
        Glyph::Brush => {
            line(&[(19.5, 4.5), (11.5, 12.5)]);
            closed(&[(10.0, 11.0), (13.0, 14.0), (11.0, 17.0), (8.0, 14.0)]);
            fill(&[
                (8.0, 14.5),
                (10.5, 17.0),
                (7.0, 20.0),
                (4.0, 20.0),
                (4.5, 18.0),
            ]);
        }
        Glyph::Airbrush => {
            closed(&[(9.0, 9.0), (17.0, 9.0), (17.0, 14.0), (9.0, 14.0)]);
            line(&[(13.0, 14.0), (13.0, 20.0)]);
            line(&[(17.0, 11.5), (20.0, 11.5)]);
            for (x, y) in [(5.0, 8.0), (3.5, 11.5), (5.0, 15.0), (6.5, 11.5)] {
                p.circle_filled(pt(x, y), 1.1 * s, color);
            }
        }
        Glyph::Eraser => {
            closed(&[
                (4.0, 15.0),
                (13.0, 6.0),
                (20.0, 13.0),
                (13.5, 19.5),
                (8.5, 19.5),
            ]);
            line(&[(8.5, 10.5), (15.5, 17.5)]);
            line(&[(13.5, 19.5), (20.0, 19.5)]);
        }
        Glyph::Eyedropper => {
            line(&[(5.0, 19.0), (6.0, 15.0), (13.5, 7.5)]);
            line(&[(5.0, 19.0), (9.0, 18.0), (16.5, 10.5)]);
            line(&[(12.0, 6.0), (18.0, 12.0)]);
            fill(&[(14.5, 6.5), (17.0, 4.0), (20.0, 7.0), (17.5, 9.5)]);
        }
        Glyph::Bucket => {
            closed(&[(4.5, 11.0), (11.0, 4.5), (18.0, 11.5), (11.5, 18.0)]);
            line(&[(4.5, 11.0), (16.0, 11.0)]);
            p.add(Shape::convex_polygon(
                vec![
                    pt(19.5, 14.0),
                    pt(21.0, 17.5),
                    pt(19.5, 19.5),
                    pt(18.0, 17.5),
                ],
                color,
                Stroke::NONE,
            ));
        }
        Glyph::Eye | Glyph::EyeOff => {
            let n = 20;
            let top: Vec<_> = (0..=n)
                .map(|i| {
                    let t = i as f32 / n as f32;
                    (
                        3.0 + 18.0 * t,
                        12.0 - 6.0 * (std::f32::consts::PI * t).sin(),
                    )
                })
                .collect();
            let bot: Vec<_> = (0..=n)
                .map(|i| {
                    let t = i as f32 / n as f32;
                    (
                        21.0 - 18.0 * t,
                        12.0 + 6.0 * (std::f32::consts::PI * t).sin(),
                    )
                })
                .collect();
            let mut all = top;
            all.extend(bot);
            closed(&all);
            p.circle_stroke(pt(12.0, 12.0), 2.8 * s, st);
            if g == Glyph::EyeOff {
                line(&[(4.0, 20.0), (20.0, 4.0)]);
            }
        }
        Glyph::Lock | Glyph::Unlock => {
            p.rect_stroke(
                Rect::from_min_max(pt(5.5, 11.0), pt(18.5, 20.0)),
                2.0 * s,
                st,
                egui::StrokeKind::Middle,
            );
            let arc: Vec<(f32, f32)> = (0..=12)
                .map(|i| {
                    let a = std::f32::consts::PI * (1.0 + i as f32 / 12.0);
                    (12.0 + 4.5 * a.cos(), 8.5 + 4.5 * a.sin())
                })
                .collect();
            let mut pts = vec![(7.5, 11.0)];
            pts.extend(arc);
            if g == Glyph::Lock {
                pts.push((16.5, 11.0));
            } else {
                pts.push((16.5, 9.0));
            }
            line(&pts);
            p.circle_filled(pt(12.0, 15.5), 1.3 * s, color);
        }
        Glyph::Plus => {
            line(&[(12.0, 5.0), (12.0, 19.0)]);
            line(&[(5.0, 12.0), (19.0, 12.0)]);
        }
        Glyph::Duplicate => {
            p.rect_stroke(
                Rect::from_min_max(pt(8.0, 8.0), pt(19.0, 19.0)),
                2.0 * s,
                st,
                egui::StrokeKind::Middle,
            );
            line(&[(5.0, 15.0), (5.0, 5.0), (15.0, 5.0)]);
        }
        Glyph::Trash => {
            line(&[(4.5, 7.0), (19.5, 7.0)]);
            line(&[(9.5, 7.0), (10.0, 4.5), (14.0, 4.5), (14.5, 7.0)]);
            closed(&[(6.5, 7.0), (7.5, 20.0), (16.5, 20.0), (17.5, 7.0)]);
            line(&[(10.0, 10.5), (10.0, 16.5)]);
            line(&[(14.0, 10.5), (14.0, 16.5)]);
        }
        Glyph::MergeDown => {
            line(&[(12.0, 4.0), (12.0, 14.0)]);
            line(&[(8.0, 10.0), (12.0, 14.0), (16.0, 10.0)]);
            line(&[(5.0, 19.0), (19.0, 19.0)]);
        }
        Glyph::Up => line(&[(6.0, 15.0), (12.0, 9.0), (18.0, 15.0)]),
        Glyph::Down => line(&[(6.0, 9.0), (12.0, 15.0), (18.0, 9.0)]),
        Glyph::Sphere => {
            p.circle_stroke(pt(12.0, 12.0), 8.0 * s, st);
            let ell: Vec<_> = (0..=24)
                .map(|i| {
                    let a = std::f32::consts::TAU * i as f32 / 24.0;
                    (12.0 + 8.0 * a.cos(), 12.0 + 3.0 * a.sin())
                })
                .collect();
            line(&ell);
            let mer: Vec<_> = (0..=24)
                .map(|i| {
                    let a = std::f32::consts::TAU * i as f32 / 24.0;
                    (12.0 + 3.5 * a.cos(), 12.0 + 8.0 * a.sin())
                })
                .collect();
            line(&mer);
        }
        Glyph::Flat => {
            p.rect_stroke(
                Rect::from_min_max(pt(3.5, 7.0), pt(20.5, 17.0)),
                1.5 * s,
                st,
                egui::StrokeKind::Middle,
            );
            line(&[(3.5, 12.0), (20.5, 12.0)]);
            line(&[(12.0, 7.0), (12.0, 17.0)]);
        }
        Glyph::Grid => {
            for v in [8.0, 16.0] {
                line(&[(v, 4.0), (v, 20.0)]);
                line(&[(4.0, v), (20.0, v)]);
            }
        }
        Glyph::Sun => {
            p.circle_stroke(pt(12.0, 12.0), 4.0 * s, st);
            for i in 0..8 {
                let a = std::f32::consts::TAU * i as f32 / 8.0;
                line(&[
                    (12.0 + 6.5 * a.cos(), 12.0 + 6.5 * a.sin()),
                    (12.0 + 9.0 * a.cos(), 12.0 + 9.0 * a.sin()),
                ]);
            }
        }
        Glyph::Moon => {
            let outer: Vec<_> = (0..=28)
                .map(|i| {
                    let a = 0.9 + 4.6 * i as f32 / 28.0;
                    (12.0 + 8.0 * a.cos(), 12.0 + 8.0 * a.sin())
                })
                .collect();
            let inner: Vec<_> = (0..=28)
                .rev()
                .map(|i| {
                    let a = 1.75 + 2.9 * i as f32 / 28.0;
                    (15.5 + 6.2 * a.cos(), 9.0 + 6.2 * a.sin())
                })
                .collect();
            let mut pts = outer;
            pts.extend(inner);
            closed(&pts);
        }
        Glyph::Help => {
            p.circle_stroke(pt(12.0, 12.0), 8.5 * s, st);
            line(&[
                (9.3, 9.5),
                (9.8, 7.8),
                (12.0, 7.0),
                (14.3, 7.8),
                (14.8, 9.8),
                (12.0, 12.0),
                (12.0, 14.0),
            ]);
            p.circle_filled(pt(12.0, 17.0), 1.2 * s, color);
        }
        Glyph::Swap => {
            line(&[(6.0, 9.0), (18.0, 9.0)]);
            line(&[(15.0, 6.0), (18.0, 9.0), (15.0, 12.0)]);
            line(&[(18.0, 15.0), (6.0, 15.0)]);
            line(&[(9.0, 12.0), (6.0, 15.0), (9.0, 18.0)]);
        }
        Glyph::Undo | Glyph::Redo => {
            let flip = if g == Glyph::Redo { -1.0 } else { 1.0 };
            let fx = |x: f32| 12.0 + (x - 12.0) * flip;
            let arc: Vec<_> = (0..=16)
                .map(|i| {
                    let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / 16.0;
                    (fx(12.0 + 6.0 * a.cos()), 12.0 + 4.5 * a.sin())
                })
                .collect();
            let mut pts = vec![(fx(5.0), 7.5)];
            pts.extend(arc);
            pts.push((fx(7.0), 16.5));
            line(&pts);
            line(&[(fx(8.0), 4.5), (fx(5.0), 7.5), (fx(8.0), 10.5)]);
        }
        Glyph::Clear => {
            p.rect_stroke(
                Rect::from_min_max(pt(5.0, 5.0), pt(19.0, 19.0)),
                2.0 * s,
                Stroke::new(st.width, color.gamma_multiply(0.6)),
                egui::StrokeKind::Middle,
            );
            line(&[(8.0, 8.0), (16.0, 16.0)]);
            line(&[(16.0, 8.0), (8.0, 16.0)]);
        }
    }
}
