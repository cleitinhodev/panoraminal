//! The custom brush library panel (shown in the brush card for the custom tool).

use crate::app::PanoApp;
use crate::brushlib::LibFilter;
use crate::icons::{self, Glyph};
use crate::theme::{brand, with_alpha};
use crate::ui::widgets::{icon_button, slider_row, toggle};
use eframe::egui::{self, Color32, Rect, Sense, Stroke, Vec2};
use panoraminal_core::ToolKind;
use panoraminal_i18n::{tr, trf};

const TILE: f32 = 50.0;
const GOLD: Color32 = Color32::from_rgb(0xff, 0xc8, 0x3d);

pub fn library(app: &mut PanoApp, ui: &mut egui::Ui) {
    let pal = app.pal;
    let ctx = ui.ctx().clone();
    ui.horizontal(|ui| {
        if icon_button(
            ui,
            &pal,
            Glyph::Plus,
            false,
            &format!(
                "{} ({})",
                tr("brushlib.import"),
                crate::shortcuts::first_text(&ctx, crate::shortcuts::Action::ImportBrushes)
            ),
            28.0,
        )
        .clicked()
        {
            app.import_brushes_dialog(&ctx);
        }
        let cats = app.blib.lib.categories();
        let label = match &app.blib.filter {
            LibFilter::All => tr("brushlib.all").to_string(),
            LibFilter::Favorites => format!("★ {}", tr("brushlib.favorites")),
            LibFilter::Category(c) => c.clone(),
        };
        egui::ComboBox::from_id_salt("brushlib_cat")
            .width(96.0)
            .selected_text(label)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut app.blib.filter, LibFilter::All, tr("brushlib.all"));
                ui.selectable_value(
                    &mut app.blib.filter,
                    LibFilter::Favorites,
                    format!("★ {}", tr("brushlib.favorites")),
                );
                for c in cats {
                    ui.selectable_value(&mut app.blib.filter, LibFilter::Category(c.clone()), c);
                }
            });
        ui.add(
            egui::TextEdit::singleline(&mut app.blib.search)
                .hint_text(tr("brushlib.search"))
                .desired_width(ui.available_width()),
        );
    });
    ui.add_space(4.0);

    // ---- thumbnail grid
    let ids: Vec<(String, String, bool)> = app
        .blib
        .visible()
        .iter()
        .map(|b| (b.id.clone(), b.name.clone(), b.favorite))
        .collect();
    if ids.is_empty() {
        ui.label(
            egui::RichText::new(tr(if app.blib.lib.brushes.is_empty() {
                "brushlib.empty"
            } else {
                "brushlib.no_match"
            }))
            .size(12.0)
            .color(pal.text_weak),
        );
    }
    let mut select = None;
    let mut toggle_fav = None;
    let mut delete = None;
    egui::ScrollArea::vertical()
        .id_salt("brushlib_grid")
        .max_height(TILE * 2.0 + 16.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(4.0);
                for (id, name, fav) in &ids {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(TILE), Sense::click());
                    let sel = app.blib.selected.as_deref() == Some(id.as_str());
                    let hover = resp.hovered();
                    let p = ui.painter();
                    p.rect_filled(
                        rect,
                        8.0,
                        if pal.dark {
                            Color32::from_rgb(14, 18, 30)
                        } else {
                            Color32::from_rgb(236, 240, 248)
                        },
                    );
                    if let Some(t) = app.blib.thumb(&ctx, id, [255, 255, 255]) {
                        egui::Image::new((t, Vec2::splat(TILE - 10.0)))
                            .tint(pal.text)
                            .paint_at(ui, rect.shrink(5.0));
                    }
                    let p = ui.painter();
                    let border = if sel {
                        Stroke::new(2.0, brand::CYAN)
                    } else if hover {
                        Stroke::new(1.0, with_alpha(pal.neon, 0.8))
                    } else {
                        Stroke::new(1.0, pal.stroke)
                    };
                    p.rect_stroke(rect, 8.0, border, egui::StrokeKind::Inside);
                    if *fav {
                        let r = Rect::from_min_size(
                            rect.right_top() + Vec2::new(-15.0, 3.0),
                            Vec2::splat(12.0),
                        );
                        icons::draw(p, r, Glyph::Star, GOLD);
                    }
                    let resp = resp.on_hover_text(name);
                    if resp.clicked() {
                        select = Some(id.clone());
                    }
                    resp.context_menu(|ui| {
                        if ui
                            .button(if *fav {
                                tr("brushlib.unfavorite")
                            } else {
                                tr("brushlib.favorite")
                            })
                            .clicked()
                        {
                            toggle_fav = Some(id.clone());
                            ui.close();
                        }
                        if ui.button(tr("brushlib.delete")).clicked() {
                            delete = Some(id.clone());
                            ui.close();
                        }
                    });
                }
            });
        });
    if let Some(id) = select {
        app.select_brush(&id);
    }
    if let Some(id) = toggle_fav {
        if let Some(b) = app.blib.lib.find_mut(&id) {
            b.favorite = !b.favorite;
        }
        app.blib.save();
    }
    if let Some(id) = delete {
        app.blib.delete(&id);
        if let Some(sel) = app.blib.selected.clone() {
            app.select_brush(&sel);
        } else {
            app.sync_tip();
        }
    }

    // ---- selected brush
    let settings = app
        .brushes
        .get(&ToolKind::Custom)
        .copied()
        .unwrap_or_else(|| panoraminal_core::BrushSettings::preset(ToolKind::Custom));
    if app.blib.selected().is_none() {
        return;
    }
    ui.add_space(6.0);
    let w = ui.available_width().max(120.0);
    let (r, _) = ui.allocate_exact_size(Vec2::new(w, 62.0), Sense::hover());
    ui.painter().rect_filled(
        r,
        8.0,
        if pal.dark {
            Color32::from_rgb(14, 18, 30)
        } else {
            Color32::from_rgb(236, 240, 248)
        },
    );
    let size = [(w as u32).clamp(64, 480), 62];
    let prim = Color32::from_rgb(app.primary[0], app.primary[1], app.primary[2]);
    if let Some(t) = app.blib.preview(&ctx, size, &settings, [255, 255, 255]) {
        egui::Image::new((t, r.size())).tint(prim).paint_at(ui, r);
    }

    let mut save = false;
    let Some(b) = app.blib.selected_mut() else {
        return;
    };
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_sized(
            [86.0, 18.0],
            egui::Label::new(egui::RichText::new(tr("brushlib.name")).size(12.5)),
        );
        let r = ui.add(egui::TextEdit::singleline(&mut b.name).desired_width(f32::INFINITY));
        save |= r.lost_focus();
    });
    ui.horizontal(|ui| {
        ui.add_sized(
            [86.0, 18.0],
            egui::Label::new(egui::RichText::new(tr("brushlib.category")).size(12.5)),
        );
        let r = ui.add(egui::TextEdit::singleline(&mut b.category).desired_width(f32::INFINITY));
        save |= r.lost_focus();
    });
    if toggle(ui, &pal, &mut b.favorite, tr("brushlib.favorite")).changed() {
        save = true;
    }
    egui::CollapsingHeader::new(tr("brushlib.dynamics"))
        .id_salt("brush_dyn")
        .show(ui, |ui| {
            let d = &mut b.dynamics;
            let mut rs = Vec::new();
            rs.push(slider_row(
                ui,
                tr("brushlib.angle"),
                &mut d.angle,
                0.0..=359.0,
                "°",
                false,
            ));
            let mut rd = d.roundness * 100.0;
            rs.push(slider_row(
                ui,
                tr("brushlib.roundness"),
                &mut rd,
                1.0..=100.0,
                "%",
                false,
            ));
            d.roundness = rd / 100.0;
            let pct = |ui: &mut egui::Ui, key: &str, v: &mut f32, rs: &mut Vec<egui::Response>| {
                let mut x = *v * 100.0;
                rs.push(slider_row(ui, tr(key), &mut x, 0.0..=100.0, "%", false));
                *v = x / 100.0;
            };
            pct(ui, "brushlib.size_jitter", &mut d.size_jitter, &mut rs);
            pct(ui, "brushlib.min_diameter", &mut d.min_diameter, &mut rs);
            pct(ui, "brushlib.angle_jitter", &mut d.angle_jitter, &mut rs);
            pct(
                ui,
                "brushlib.roundness_jitter",
                &mut d.roundness_jitter,
                &mut rs,
            );
            let mut sc = d.scatter * 100.0;
            rs.push(slider_row(
                ui,
                tr("brushlib.scatter"),
                &mut sc,
                0.0..=1000.0,
                "%",
                false,
            ));
            d.scatter = sc / 100.0;
            rs.push(slider_row(
                ui,
                tr("brushlib.count"),
                &mut d.count,
                1..=16,
                "",
                false,
            ));
            pct(
                ui,
                "brushlib.opacity_jitter",
                &mut d.opacity_jitter,
                &mut rs,
            );
            pct(ui, "brushlib.flow_jitter", &mut d.flow_jitter, &mut rs);
            rs.push(toggle(
                ui,
                &pal,
                &mut d.scatter_both_axes,
                tr("brushlib.both_axes"),
            ));
            rs.push(toggle(
                ui,
                &pal,
                &mut d.follow_direction,
                tr("brushlib.follow"),
            ));
            rs.push(toggle(
                ui,
                &pal,
                &mut d.flip_x_jitter,
                tr("brushlib.flip_x_jitter"),
            ));
            rs.push(toggle(
                ui,
                &pal,
                &mut d.flip_y_jitter,
                tr("brushlib.flip_y_jitter"),
            ));
            d.clamp();
            save |= rs
                .iter()
                .any(|r| r.drag_stopped() || (r.changed() && !r.dragged()));
        });
    if !b.notices.is_empty() {
        ui.add_space(2.0);
        for n in &b.notices {
            ui.label(
                egui::RichText::new(format!("⚠ {}", tr(n.key())))
                    .size(11.5)
                    .color(GOLD),
            )
            .on_hover_text(tr("brushlib.notice_tip"));
        }
    }
    if !b.source.is_empty() {
        ui.label(
            egui::RichText::new(trf(
                "brushlib.source",
                &[(
                    "src",
                    &if b.source == "built-in" {
                        tr("brushlib.builtin")
                    } else {
                        b.source.as_str()
                    },
                )],
            ))
            .size(11.0)
            .color(pal.text_weak),
        );
    }
    if save {
        app.blib.forget_thumbs();
        app.blib.save();
    }
}
