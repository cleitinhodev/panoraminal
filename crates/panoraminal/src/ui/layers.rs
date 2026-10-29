//! Layers card: thumbnails, visibility/lock toggles, drag-to-reorder, properties.

use super::widgets::icon_button;
use crate::app::PanoApp;
use crate::icons::{self, Glyph};
use crate::shortcuts::{first_text, Action};
use crate::theme::{self, brand, with_alpha};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use panoraminal_core::history::ops;
use panoraminal_core::BlendMode;
use panoraminal_i18n::tr;

const ROW_H: f32 = 40.0;

fn checker(p: &egui::Painter, r: Rect, dark: bool) {
    let (a, b) = if dark {
        (Color32::from_gray(58), Color32::from_gray(40))
    } else {
        (Color32::from_gray(235), Color32::from_gray(205))
    };
    p.rect_filled(r, 0.0, a);
    let cell = 6.0;
    let nx = (r.width() / cell).ceil() as i32;
    let ny = (r.height() / cell).ceil() as i32;
    for j in 0..ny {
        for i in 0..nx {
            if (i + j) % 2 == 0 {
                let c = Rect::from_min_size(
                    r.min + Vec2::new(i as f32 * cell, j as f32 * cell),
                    Vec2::splat(cell),
                )
                .intersect(r);
                p.rect_filled(c, 0.0, b);
            }
        }
    }
}

pub fn layers_card(app: &mut PanoApp, ui: &mut egui::Ui, now: f64) {
    let pal = app.pal;
    let ctx = ui.ctx().clone();
    let _ = now;
    // Action row.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if icon_button(
            ui,
            &pal,
            Glyph::Plus,
            false,
            &format!(
                "{} ({})",
                tr("menu.layer.new"),
                first_text(&ctx, Action::NewLayer)
            ),
            28.0,
        )
        .clicked()
        {
            app.new_layer(&ctx);
        }
        if icon_button(
            ui,
            &pal,
            Glyph::Duplicate,
            false,
            &format!(
                "{} ({})",
                tr("menu.layer.duplicate"),
                first_text(&ctx, Action::DuplicateLayer)
            ),
            28.0,
        )
        .clicked()
        {
            app.duplicate_layer(&ctx);
        }
        let can_merge = app.doc.active_index() > 0;
        if ui
            .add_enabled_ui(can_merge, |ui| {
                icon_button(
                    ui,
                    &pal,
                    Glyph::MergeDown,
                    false,
                    &format!(
                        "{} ({})",
                        tr("menu.layer.merge_down"),
                        first_text(&ctx, Action::MergeDown)
                    ),
                    28.0,
                )
            })
            .inner
            .clicked()
        {
            app.merge_down(&ctx);
        }
        let ai = app.doc.active_index();
        let top = app.doc.layers.len().saturating_sub(1);
        if ui
            .add_enabled_ui(ai < top, |ui| {
                icon_button(ui, &pal, Glyph::Up, false, tr("layer.move_up"), 28.0)
            })
            .inner
            .clicked()
        {
            app.move_layer(ai, ai + 1);
        }
        if ui
            .add_enabled_ui(ai > 0, |ui| {
                icon_button(ui, &pal, Glyph::Down, false, tr("layer.move_down"), 28.0)
            })
            .inner
            .clicked()
        {
            app.move_layer(ai, ai - 1);
        }
        if icon_button(
            ui,
            &pal,
            Glyph::Clear,
            false,
            &format!("{} (Del)", tr("menu.edit.clear")),
            28.0,
        )
        .clicked()
        {
            app.clear_layer();
        }
        let can_del = app.doc.layers.len() > 1;
        if ui
            .add_enabled_ui(can_del, |ui| {
                icon_button(ui, &pal, Glyph::Trash, false, tr("menu.layer.delete"), 28.0)
            })
            .inner
            .clicked()
        {
            app.delete_layer(&ctx);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let n = app.doc.layers.len();
            ui.label(
                egui::RichText::new(format!("{n}/{}", panoraminal_core::MAX_LAYERS))
                    .size(11.5)
                    .color(pal.text_weak),
            );
        });
    });
    ui.add_space(4.0);

    // Properties of the active layer.
    let active = app.doc.active;
    if let Some(mut props) = app.doc.props(active) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tr("layer.opacity")).size(12.5));
            let mut op = props.opacity * 100.0;
            ui.spacing_mut().slider_width = (ui.available_width() - 60.0).max(60.0);
            let r = ui.add(
                egui::Slider::new(&mut op, 0.0..=100.0)
                    .suffix("%")
                    .max_decimals(0),
            );
            if r.drag_started() {
                app.props_drag = app.doc.props(active).map(|p| (active, p));
            }
            if r.changed() {
                props.opacity = op / 100.0;
                if r.dragged() {
                    app.doc.set_props(active, &props);
                } else {
                    app.finish_stroke();
                    ops::set_props(
                        &mut app.doc,
                        &mut app.hist,
                        active,
                        props.clone(),
                        "hist.opacity",
                    );
                }
            }
            if r.drag_stopped() {
                if let Some((id, old)) = app.props_drag.take() {
                    if let Some(new) = app.doc.props(id) {
                        if new != old {
                            app.doc.set_props(id, &old);
                            ops::set_props(&mut app.doc, &mut app.hist, id, new, "hist.opacity");
                        }
                    }
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tr("layer.blend")).size(12.5));
            let mut blend = props.blend;
            egui::ComboBox::from_id_salt("blend")
                .width(ui.available_width() - 8.0)
                .selected_text(tr(&blend_key(blend)))
                .show_ui(ui, |ui| {
                    for b in BlendMode::ALL {
                        ui.selectable_value(&mut blend, b, tr(&blend_key(b)));
                    }
                });
            if blend != props.blend {
                props.blend = blend;
                app.finish_stroke();
                ops::set_props(
                    &mut app.doc,
                    &mut app.hist,
                    active,
                    props.clone(),
                    "hist.blend",
                );
            }
        });
    }
    ui.add_space(6.0);

    // Layer rows, top-most first.
    let n = app.doc.layers.len();
    let mut rows: Vec<Rect> = Vec::with_capacity(n);
    let mut select = None;
    let mut toggle_vis = None;
    let mut toggle_lock = None;
    let mut rename = None;
    let mut drag_start = None;
    for vis_i in 0..n {
        let idx = n - 1 - vis_i;
        let (id, name, visible, locked, opacity, blend) = {
            let l = &app.doc.layers[idx];
            (
                l.id,
                l.name.clone(),
                l.visible,
                l.alpha_locked,
                l.opacity,
                l.blend,
            )
        };
        let (rect, resp) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), ROW_H),
            Sense::click_and_drag(),
        );
        rows.push(rect);
        let is_active = id == app.doc.active;
        let dragging_this = app.dragged_layer == Some(idx);
        let hover = ui.ctx().animate_bool(resp.id, resp.hovered());
        let p = ui.painter();
        if is_active {
            theme::gradient_pill(
                p,
                rect,
                &[
                    with_alpha(brand::ROYAL, if pal.dark { 0.55 } else { 0.22 }),
                    with_alpha(brand::AZURE, if pal.dark { 0.25 } else { 0.10 }),
                ],
                9.0,
            );
            p.rect_stroke(
                rect,
                9.0,
                Stroke::new(1.2, with_alpha(pal.neon, 0.9)),
                egui::StrokeKind::Inside,
            );
        } else if hover > 0.0 {
            p.rect_filled(rect, 9.0, with_alpha(pal.raised, hover));
        }
        if dragging_this {
            p.rect_stroke(
                rect,
                9.0,
                Stroke::new(1.5, pal.neon),
                egui::StrokeKind::Inside,
            );
        }
        // Thumbnail.
        let th = Rect::from_min_size(rect.min + Vec2::new(6.0, 6.0), Vec2::new(56.0, 28.0));
        checker(p, th, pal.dark);
        if let Some(t) = app.thumbs.get(&id) {
            p.image(
                t.id(),
                th,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        p.rect_stroke(
            th,
            3.0,
            Stroke::new(1.0, with_alpha(pal.stroke, 0.9)),
            egui::StrokeKind::Outside,
        );
        // Name + meta.
        let text_c = if visible { pal.text } else { pal.text_weak };
        let name_pos = Pos2::new(th.right() + 8.0, rect.top() + 6.0);
        let max_w = rect.right() - 64.0 - name_pos.x;
        let galley = p.layout(
            name.clone(),
            egui::FontId::proportional(13.0),
            text_c,
            max_w.max(20.0),
        );
        p.galley(name_pos, galley, text_c);
        let meta = format!("{:.0}% · {}", opacity * 100.0, tr(&blend_key(blend)));
        p.text(
            Pos2::new(name_pos.x, rect.bottom() - 6.0),
            egui::Align2::LEFT_BOTTOM,
            meta,
            egui::FontId::proportional(11.0),
            pal.text_weak,
        );
        // Eye and lock toggles (drawn on top, own interaction).
        let eye = Rect::from_center_size(
            Pos2::new(rect.right() - 18.0, rect.center().y),
            Vec2::splat(24.0),
        );
        let lock = Rect::from_center_size(
            Pos2::new(rect.right() - 44.0, rect.center().y),
            Vec2::splat(24.0),
        );
        let er = ui
            .interact(eye, resp.id.with("eye"), Sense::click())
            .on_hover_text(tr("layer.visibility"));
        let lr = ui
            .interact(lock, resp.id.with("lock"), Sense::click())
            .on_hover_text(tr("layer.alpha_lock"));
        let p = ui.painter();
        let ec = if er.hovered() {
            pal.neon
        } else if visible {
            pal.text
        } else {
            pal.text_weak
        };
        icons::draw(
            p,
            eye.shrink(4.0),
            if visible { Glyph::Eye } else { Glyph::EyeOff },
            ec,
        );
        let lc = if lr.hovered() {
            pal.neon
        } else if locked {
            pal.accent
        } else {
            with_alpha(pal.text_weak, 0.7)
        };
        icons::draw(
            p,
            lock.shrink(5.0),
            if locked { Glyph::Lock } else { Glyph::Unlock },
            lc,
        );
        if er.clicked() {
            toggle_vis = Some(id);
        } else if lr.clicked() {
            toggle_lock = Some(id);
        } else if resp.double_clicked() {
            rename = Some((id, name.clone()));
        } else if resp.clicked() {
            select = Some(id);
        }
        if resp.drag_started() {
            drag_start = Some(idx);
        }
        resp.context_menu(|ui| {
            if ui.button(tr("menu.layer.rename")).clicked() {
                rename = Some((id, name.clone()));
            }
            if ui.button(tr("menu.layer.duplicate")).clicked() {
                select = Some(id);
                app.doc.active = id;
                app.duplicate_layer(&ctx);
            }
            if ui.button(tr("menu.edit.clear")).clicked() {
                app.doc.active = id;
                app.clear_layer();
            }
            if ui
                .add_enabled(n > 1, egui::Button::new(tr("menu.layer.delete")))
                .clicked()
            {
                app.doc.active = id;
                app.delete_layer(&ctx);
            }
        });
        ui.add_space(2.0);
    }
    if let Some(i) = drag_start {
        app.dragged_layer = Some(i);
    }
    // Drag-to-reorder: compute insertion slot from pointer y.
    if let Some(from) = app.dragged_layer {
        let pointer = ctx.pointer_interact_pos();
        let released = ctx.input(|i| i.pointer.any_released() || !i.pointer.any_down());
        if let (Some(pos), false) = (pointer, rows.is_empty()) {
            // Visual slot index (0 = top).
            let mut slot = rows.len();
            for (vi, r) in rows.iter().enumerate() {
                if pos.y < r.center().y {
                    slot = vi;
                    break;
                }
            }
            let y = if slot < rows.len() {
                rows[slot].top() - 1.0
            } else {
                rows[rows.len() - 1].bottom() + 1.0
            };
            let x = rows[0].x_range();
            let p = ui.painter();
            theme::glow_rect(
                p,
                Rect::from_min_max(Pos2::new(x.min, y - 1.0), Pos2::new(x.max, y + 1.0)),
                1.0,
                pal.neon,
                0.6,
            );
            p.hline(x, y, Stroke::new(2.5, pal.neon));
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            if released {
                // Convert visual slot to document index (bottom-up).
                let to_vis_before = slot; // insert before this visual row
                let from_vis = n - 1 - from;
                let mut target_vis = if to_vis_before > from_vis {
                    to_vis_before - 1
                } else {
                    to_vis_before
                };
                target_vis = target_vis.min(n - 1);
                let to = n - 1 - target_vis;
                if to != from {
                    app.move_layer(from, to);
                }
                app.dragged_layer = None;
            }
        } else if released {
            app.dragged_layer = None;
        }
    }
    if let Some(id) = select {
        if app.doc.active != id {
            app.finish_stroke();
            app.doc.active = id;
        }
    }
    if let Some(id) = toggle_vis.or(toggle_lock) {
        if let Some(mut p) = app.doc.props(id) {
            let label = if toggle_vis.is_some() {
                p.visible = !p.visible;
                "hist.visibility"
            } else {
                p.alpha_locked = !p.alpha_locked;
                "hist.alpha_lock"
            };
            app.finish_stroke();
            ops::set_props(&mut app.doc, &mut app.hist, id, p, label);
        }
    }
    if let Some(r) = rename {
        app.dlg.rename = Some(r);
    }
}

pub fn blend_key(b: BlendMode) -> String {
    format!("blend.{}", b.id())
}
