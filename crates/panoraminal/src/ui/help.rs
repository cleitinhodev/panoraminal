//! Help (shortcuts, tool guide, tips) and About windows.

use super::widgets::{open_url, section_title};
use crate::app::{PanoApp, Tool};
use crate::icons::{self, Glyph};
use crate::shortcuts::{Group, GESTURES, TABLE};
use crate::theme::{self, brand, with_alpha};
use eframe::egui::{self, Color32, FontId, Id, RichText, Sense, Stroke, Vec2};
use panoraminal_i18n::{tr, trf};

fn key_chip(ui: &mut egui::Ui, pal: &crate::theme::Palette, text: &str) {
    let font = FontId::monospace(12.0);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font, pal.text);
    let size = galley.size() + Vec2::new(14.0, 6.0);
    let (r, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter();
    p.rect_filled(r, 6.0, if pal.dark { pal.raised } else { Color32::WHITE });
    p.rect_stroke(
        r,
        6.0,
        Stroke::new(1.0, with_alpha(pal.accent, 0.7)),
        egui::StrokeKind::Inside,
    );
    p.hline(
        r.shrink(3.0).x_range(),
        r.bottom() - 1.5,
        Stroke::new(1.5, with_alpha(pal.neon, 0.45)),
    );
    p.galley(r.center() - galley.size() * 0.5, galley, pal.text);
}

pub fn help_window(app: &mut PanoApp, ctx: &egui::Context) {
    if !app.dlg.help {
        return;
    }
    let mut open = true;
    let pal = app.pal;
    egui::Window::new(tr("help.title"))
        .id(Id::unique("win_help"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size([660.0, 560.0])
        .max_width((ctx.content_rect().width() - 40.0).max(320.0))
        .max_height((ctx.content_rect().height() - 60.0).max(240.0))
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (i, k) in ["help.tab.shortcuts", "help.tab.tools", "help.tab.tips"]
                    .iter()
                    .enumerate()
                {
                    let sel = app.dlg.help_tab == i;
                    let r = ui.selectable_label(sel, RichText::new(tr(k)).size(14.0));
                    if r.clicked() {
                        app.dlg.help_tab = i;
                    }
                    if sel {
                        let line = egui::Rect::from_min_max(
                            egui::pos2(r.rect.left() + 4.0, r.rect.bottom() + 1.0),
                            egui::pos2(r.rect.right() - 4.0, r.rect.bottom() + 3.0),
                        );
                        theme::gradient_rect(
                            ui.painter(),
                            line,
                            &[brand::ROYAL, brand::CYAN],
                            true,
                        );
                    }
                }
            });
            ui.add_space(8.0);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| match app.dlg.help_tab {
                    0 => shortcuts_tab(ui, ctx, &pal),
                    1 => tools_tab(ui, &pal),
                    _ => tips_tab(ui, &pal),
                });
        });
    if !open {
        app.dlg.help = false;
    }
}

fn shortcuts_tab(ui: &mut egui::Ui, ctx: &egui::Context, pal: &crate::theme::Palette) {
    ui.label(RichText::new(tr("help.shortcuts_intro")).color(pal.text_weak));
    ui.add_space(6.0);
    let cols = if ui.available_width() > 560.0 { 2 } else { 1 };
    ui.columns(cols, |cols| {
        for (gi, g) in Group::ALL.iter().enumerate() {
            let ui = &mut cols[gi % cols.len()];
            theme::card(pal).show(ui, |ui| {
                ui.set_width(ui.available_width());
                section_title(ui, pal, tr(g.label()));
                egui::Grid::new(("sc", gi))
                    .num_columns(2)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        for sc in TABLE.iter().filter(|s| s.group == *g) {
                            ui.horizontal(|ui| {
                                for (i, k) in sc.keys.iter().enumerate() {
                                    if i > 0 {
                                        ui.label(RichText::new("/").color(pal.text_weak));
                                    }
                                    let t = crate::shortcuts::shortcut_text_sep(ctx, k, " + ");
                                    key_chip(ui, pal, &t);
                                }
                            });
                            ui.label(tr(sc.label));
                            ui.end_row();
                        }
                        for (keys, label, gg) in GESTURES.iter() {
                            if gg == g {
                                // Chips mix literal keys with translated words,
                                // e.g. "Shift + help.key.click".
                                let k: Vec<&str> = keys
                                    .split(" + ")
                                    .map(|p| if p.starts_with("help.") { tr(p) } else { p })
                                    .collect();
                                key_chip(ui, pal, &k.join(" + "));
                                ui.label(tr(label));
                                ui.end_row();
                            }
                        }
                    });
            });
            ui.add_space(8.0);
        }
    });
}

fn tools_tab(ui: &mut egui::Ui, pal: &crate::theme::Palette) {
    for t in Tool::ALL {
        theme::card(pal).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::hover());
                theme::gradient_pill(ui.painter(), r, &[brand::ROYAL, brand::AZURE], 9.0);
                icons::draw(ui.painter(), r.shrink(8.0), t.glyph(), Color32::WHITE);
                ui.vertical(|ui| {
                    ui.label(RichText::new(tr(t.label())).strong().size(15.0));
                    let key = format!("{}.desc", t.label());
                    ui.label(RichText::new(tr(&key)).color(pal.text_weak));
                });
            });
        });
        ui.add_space(6.0);
    }
    theme::card(pal).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::hover());
            theme::gradient_pill(ui.painter(), r, &[brand::DEEP, brand::ROYAL], 9.0);
            icons::draw(ui.painter(), r.shrink(8.0), Glyph::Lock, Color32::WHITE);
            ui.vertical(|ui| {
                ui.label(RichText::new(tr("layer.alpha_lock")).strong().size(15.0));
                ui.label(RichText::new(tr("layer.alpha_lock.desc")).color(pal.text_weak));
            });
        });
    });
    ui.add_space(6.0);
    theme::card(pal).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::hover());
            theme::gradient_pill(ui.painter(), r, &[brand::DEEP, brand::ROYAL], 9.0);
            icons::draw(ui.painter(), r.shrink(8.0), Glyph::Plus, Color32::WHITE);
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(tr("menu.file.insert_image"))
                        .strong()
                        .size(15.0),
                );
                ui.label(RichText::new(tr("help.place.desc")).color(pal.text_weak));
            });
        });
    });
}

fn tips_tab(ui: &mut egui::Ui, pal: &crate::theme::Palette) {
    for i in 1..=11 {
        let key = format!("help.tip.{i}");
        ui.horizontal_top(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::new(22.0, 22.0), Sense::hover());
            ui.painter()
                .circle_filled(r.center(), 10.0, with_alpha(pal.accent, 0.25));
            ui.painter().text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                i.to_string(),
                FontId::proportional(12.0),
                pal.accent,
            );
            ui.add(egui::Label::new(tr(&key)).wrap());
        });
        ui.add_space(6.0);
    }
}

pub fn about_window(app: &mut PanoApp, ctx: &egui::Context) {
    if !app.dlg.about {
        return;
    }
    let mut open = true;
    let pal = app.pal;
    let glow = app.s.glow.factor();
    let now = ctx.input(|i| i.time);
    egui::Window::new(tr("about.title"))
        .id(Id::unique("win_about"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(460.0)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::new(150.0, 150.0), Sense::hover());
                let c = r.center();
                let phase = if app.anim { now as f32 * 0.8 } else { 0.0 };
                theme::glow_circle(ui.painter(), c, 62.0, brand::AZURE, 0.7 * glow);
                ui.painter()
                    .circle_stroke(c, 66.0, Stroke::new(1.5, with_alpha(brand::CYAN, 0.6)));
                if app.anim {
                    let d = c + Vec2::angled(phase) * 66.0;
                    theme::glow_circle(ui.painter(), d, 3.5, brand::CYAN, glow.max(0.4));
                    ui.painter().circle_filled(d, 3.5, brand::ICE);
                    ctx.request_repaint_after(std::time::Duration::from_millis(33));
                }
                egui::Image::new(&app.icons.logo)
                    .paint_at(ui, egui::Rect::from_center_size(c, Vec2::splat(118.0)));
                let stops = if pal.dark {
                    [brand::SNOW, brand::ICE, brand::CYAN, brand::AZURE]
                } else {
                    [brand::DEEP, brand::ROYAL, brand::AZURE, brand::ROYAL]
                };
                ui.label(theme::gradient_text(
                    crate::APP_NAME,
                    FontId::proportional(30.0),
                    &stops,
                    phase * 0.1,
                ));
                ui.label(
                    RichText::new(trf("about.version", &[("v", &crate::VERSION)]))
                        .color(pal.accent)
                        .strong(),
                );
                ui.add_space(4.0);
                ui.label(RichText::new(tr("about.description")).color(pal.text));
                ui.add_space(12.0);
            });
            let links = [
                (Glyph::Duplicate, "about.repo", crate::REPO_URL),
                (Glyph::Sphere, "about.site", crate::SITE_URL),
                (Glyph::Plus, "about.donate", crate::DONATE_URL),
            ];
            for (g, k, url) in links {
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 46.0), Sense::click());
                let hover = ui.ctx().animate_bool(resp.id, resp.hovered());
                let p = ui.painter();
                if k == "about.donate" {
                    theme::glow_rect(p, rect, 12.0, brand::CYAN, (0.3 + 0.5 * hover) * glow);
                    theme::gradient_pill(
                        p,
                        rect,
                        &[
                            brand::ROYAL,
                            theme::lerp_color(brand::AZURE, brand::CYAN, hover * 0.6),
                        ],
                        12.0,
                    );
                } else {
                    p.rect_filled(
                        rect,
                        12.0,
                        theme::lerp_color(
                            if pal.dark { pal.raised } else { Color32::WHITE },
                            pal.accent,
                            hover * 0.18,
                        ),
                    );
                    p.rect_stroke(
                        rect,
                        12.0,
                        Stroke::new(1.0, theme::lerp_color(pal.stroke, pal.neon, hover)),
                        egui::StrokeKind::Inside,
                    );
                }
                let fg = if k == "about.donate" {
                    Color32::WHITE
                } else {
                    pal.text
                };
                let sub = if k == "about.donate" {
                    with_alpha(Color32::WHITE, 0.85)
                } else {
                    pal.text_weak
                };
                icons::draw(
                    p,
                    egui::Rect::from_center_size(
                        egui::pos2(rect.left() + 24.0, rect.center().y),
                        Vec2::splat(18.0),
                    ),
                    g,
                    fg,
                );
                p.text(
                    egui::pos2(rect.left() + 46.0, rect.top() + 8.0),
                    egui::Align2::LEFT_TOP,
                    tr(k),
                    FontId::proportional(14.5),
                    fg,
                );
                p.text(
                    egui::pos2(rect.left() + 46.0, rect.bottom() - 7.0),
                    egui::Align2::LEFT_BOTTOM,
                    url,
                    FontId::proportional(11.5),
                    sub,
                );
                p.text(
                    egui::pos2(rect.right() - 16.0, rect.center().y),
                    egui::Align2::RIGHT_CENTER,
                    "↗",
                    FontId::proportional(16.0),
                    fg,
                );
                let resp = resp
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(url);
                if resp.clicked() {
                    open_url(ctx, url);
                }
                ui.add_space(6.0);
            }
            ui.add_space(6.0);
            egui::CollapsingHeader::new(tr("about.credits"))
                .id_salt("about_credits")
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(tr("about.credits_text"))
                            .size(12.0)
                            .color(pal.text_weak),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!("GPU: {}", app.gpu_info))
                            .size(11.5)
                            .color(pal.text_weak),
                    );
                    ui.label(
                        RichText::new(format!(
                            "{} {} · {}",
                            std::env::consts::OS,
                            std::env::consts::ARCH,
                            trf("about.max_size", &[("px", &app.max_dim)])
                        ))
                        .size(11.5)
                        .color(pal.text_weak),
                    );
                });
            ui.add_space(4.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(tr("about.license"))
                        .size(11.5)
                        .color(pal.text_weak),
                );
            });
        });
    if !open {
        app.dlg.about = false;
    }
}
