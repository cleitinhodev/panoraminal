//! The user interface: panels, canvas, dialogs, help and about.

pub mod brushlib;
pub mod canvas;
pub mod color;
pub mod dialogs;
pub mod help;
pub mod layers;
pub mod place;
pub mod widgets;

use crate::app::{PanoApp, Pending, ToastKind, Tool};
use crate::icons::Glyph;
use crate::settings::ThemeChoice;
use crate::shortcuts::{first_text, Action};
use crate::theme::{self, brand, with_alpha};
use eframe::egui::{self, Align, Align2, Color32, FontId, Layout, Pos2, Rect, Sense, Stroke, Vec2};
use panoraminal_core::ToolKind;
use panoraminal_i18n::{tr, trf};
use panoraminal_render::ViewMode;
use widgets::{icon_button, section_title};

pub fn layout(app: &mut PanoApp, ui: &mut egui::Ui, now: f64) {
    let mut show = !app.ui_hidden;
    let pal = app.pal;
    egui::Panel::top("top_bar")
        .exact_size(46.0)
        .frame(egui::Frame::NONE)
        .show_collapsible(ui, &mut show, |ui| top_bar(app, ui, now));
    egui::Panel::bottom("status")
        .exact_size(26.0)
        .frame(
            egui::Frame::NONE
                .fill(pal.panel)
                .inner_margin(egui::Margin::symmetric(10, 3)),
        )
        .show_collapsible(ui, &mut show, |ui| status_bar(app, ui));
    egui::Panel::left("tools")
        .exact_size(58.0)
        .resizable(false)
        .frame(
            egui::Frame::NONE
                .fill(pal.panel)
                .inner_margin(egui::Margin::symmetric(6, 10)),
        )
        .show_collapsible(ui, &mut show, |ui| tool_bar(app, ui, now));
    egui::Panel::right("side")
        .default_size(312.0)
        .size_range(260.0..=460.0)
        .frame(
            egui::Frame::NONE
                .fill(pal.panel)
                .inner_margin(egui::Margin::same(10)),
        )
        .show_collapsible(ui, &mut show, |ui| side_panel(app, ui, now));
    egui::CentralPanel::no_frame().show(ui, |ui| canvas::show(app, ui, now));
}

fn top_bar(app: &mut PanoApp, ui: &mut egui::Ui, now: f64) {
    let pal = app.pal;
    let rect = ui.max_rect();
    let painter = ui.painter().clone();
    theme::gradient_rect(
        &painter,
        rect,
        &[pal.header_a, pal.header_b, pal.header_a],
        true,
    );
    // Animated neon line along the bottom edge.
    let line = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - 2.0), rect.max);
    let phase = if app.anim {
        (now * 0.12).fract() as f32
    } else {
        0.0
    };
    neon_line(&painter, line, phase, app.s.glow.factor());
    if app.anim && ui.ctx().input(|i| i.viewport().focused.unwrap_or(true)) {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
    }

    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(10.0, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    let ui = &mut child;
    let logo = egui::Image::new(&app.icons.logo_small).fit_to_exact_size(Vec2::splat(30.0));
    let r = ui
        .add(logo.sense(Sense::click()))
        .on_hover_text(tr("menu.help.about"));
    if r.clicked() {
        app.dlg.about = true;
    }
    if pal.dark {
        theme::glow_circle(
            ui.painter(),
            r.rect.center(),
            14.0,
            brand::AZURE,
            0.35 * app.s.glow.factor(),
        );
    }
    let stops = if pal.dark {
        [brand::SNOW, brand::ICE, brand::CYAN, brand::AZURE]
    } else {
        [brand::DEEP, brand::ROYAL, brand::AZURE, brand::ROYAL]
    };
    ui.label(theme::gradient_text(
        crate::APP_NAME,
        FontId::proportional(19.0),
        &stops,
        phase * 2.0,
    ));
    ui.add_space(10.0);
    menus(app, ui);

    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if icon_button(
            ui,
            &pal,
            Glyph::Help,
            false,
            &format!("{} (F1)", tr("menu.help.help")),
            30.0,
        )
        .clicked()
        {
            app.dlg.help = !app.dlg.help;
        }
        let dark = pal.dark;
        let tg = if dark { Glyph::Sun } else { Glyph::Moon };
        if icon_button(ui, &pal, tg, false, tr("action.toggle_theme"), 30.0).clicked() {
            app.s.theme = if dark {
                ThemeChoice::Light
            } else {
                ThemeChoice::Dark
            };
            app.cli.theme = None;
        }
        ui.separator();
        if icon_button(
            ui,
            &pal,
            Glyph::Grid,
            app.s.show_grid,
            &format!("{} (G)", tr("menu.view.grid")),
            30.0,
        )
        .clicked()
        {
            app.s.show_grid = !app.s.show_grid;
        }
        let flat = app.mode == ViewMode::Flat;
        if icon_button(
            ui,
            &pal,
            Glyph::Flat,
            flat,
            &format!("{} (V)", tr("view.flat")),
            30.0,
        )
        .clicked()
            && !flat
        {
            app.toggle_view();
        }
        if icon_button(
            ui,
            &pal,
            Glyph::Sphere,
            !flat,
            &format!("{} (V)", tr("view.sphere")),
            30.0,
        )
        .clicked()
            && flat
        {
            app.toggle_view();
        }
        ui.separator();
        let redo = app.hist.can_redo();
        let undo = app.hist.can_undo();
        let ctx = ui.ctx().clone();
        if ui
            .add_enabled_ui(redo, |ui| {
                icon_button(
                    ui,
                    &pal,
                    Glyph::Redo,
                    false,
                    &format!(
                        "{} ({})",
                        tr("menu.edit.redo"),
                        first_text(&ctx, Action::Redo)
                    ),
                    30.0,
                )
            })
            .inner
            .clicked()
        {
            app.redo(&ctx);
        }
        if ui
            .add_enabled_ui(undo, |ui| {
                icon_button(
                    ui,
                    &pal,
                    Glyph::Undo,
                    false,
                    &format!(
                        "{} ({})",
                        tr("menu.edit.undo"),
                        first_text(&ctx, Action::Undo)
                    ),
                    30.0,
                )
            })
            .inner
            .clicked()
        {
            app.undo(&ctx);
        }
    });
}

pub fn neon_line(painter: &egui::Painter, rect: Rect, phase: f32, glow: f32) {
    let stops: Vec<Color32> = (0..9)
        .map(|i| {
            let t = (i as f32 / 8.0 + phase).fract();
            theme::gradient_at(
                &[
                    brand::ROYAL,
                    brand::AZURE,
                    brand::CYAN,
                    brand::ICE,
                    brand::CYAN,
                    brand::AZURE,
                    brand::ROYAL,
                ],
                t,
            )
        })
        .collect();
    if glow > 0.0 {
        theme::gradient_rect(
            painter,
            rect.expand2(Vec2::new(0.0, 2.0))
                .translate(Vec2::new(0.0, -1.0)),
            &stops
                .iter()
                .map(|c| with_alpha(*c, 0.25 * glow))
                .collect::<Vec<_>>(),
            true,
        );
    }
    theme::gradient_rect(painter, rect, &stops, true);
}

fn menu_item(ui: &mut egui::Ui, label: &str, shortcut: Option<Action>) -> bool {
    let ctx = ui.ctx().clone();
    let mut b = egui::Button::new(label);
    if let Some(a) = shortcut {
        b = b.shortcut_text(first_text(&ctx, a));
    }
    ui.add(b).clicked()
}

fn menus(app: &mut PanoApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button(tr("menu.file"), |ui| {
            if menu_item(ui, tr("menu.file.new"), Some(Action::New)) {
                app.request(&ctx, Pending::New);
            }
            if menu_item(ui, tr("menu.file.open"), Some(Action::Open)) {
                app.request(&ctx, Pending::Open(None));
            }
            let recent = app.s.recent.clone();
            ui.add_enabled_ui(!recent.is_empty(), |ui| {
                ui.menu_button(tr("menu.file.recent"), |ui| {
                    for p in recent {
                        if ui
                            .button(crate::app::file_name(&p))
                            .on_hover_text(p.display().to_string())
                            .clicked()
                        {
                            app.request(&ctx, Pending::Open(Some(p)));
                        }
                    }
                    ui.separator();
                    if ui.button(tr("menu.file.clear_recent")).clicked() {
                        app.s.recent.clear();
                    }
                });
            });
            ui.separator();
            if menu_item(ui, tr("menu.file.save"), Some(Action::Save)) {
                app.save(&ctx, None);
            }
            if menu_item(ui, tr("menu.file.save_as"), Some(Action::SaveAs)) {
                app.save_as(&ctx, None);
            }
            ui.separator();
            if menu_item(ui, tr("menu.file.insert_image"), Some(Action::InsertImage)) {
                app.insert_image_dialog(&ctx);
            }
            if menu_item(ui, tr("menu.file.import"), Some(Action::ImportLayer)) {
                app.import_layer_dialog(&ctx);
            }
            if menu_item(
                ui,
                tr("menu.file.import_brushes"),
                Some(Action::ImportBrushes),
            ) {
                app.import_brushes_dialog(&ctx);
            }
            if menu_item(ui, tr("menu.file.export_png"), Some(Action::ExportPng)) {
                app.export(&ctx, false);
            }
            if menu_item(ui, tr("menu.file.export_jpg"), Some(Action::ExportJpg)) {
                app.dlg.export_jpg = true;
            }
            ui.separator();
            if menu_item(ui, tr("menu.file.settings"), Some(Action::Settings)) {
                app.dlg.settings = true;
            }
            ui.separator();
            if menu_item(ui, tr("menu.file.quit"), Some(Action::Quit)) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
        ui.menu_button(tr("menu.edit"), |ui| {
            let ul = app
                .hist
                .undo_label()
                .map(|l| format!("{} — {}", tr("menu.edit.undo"), tr(l)))
                .unwrap_or_else(|| tr("menu.edit.undo").to_string());
            let rl = app
                .hist
                .redo_label()
                .map(|l| format!("{} — {}", tr("menu.edit.redo"), tr(l)))
                .unwrap_or_else(|| tr("menu.edit.redo").to_string());
            if ui
                .add_enabled_ui(app.hist.can_undo(), |ui| {
                    menu_item(ui, &ul, Some(Action::Undo))
                })
                .inner
            {
                app.undo(&ctx);
            }
            if ui
                .add_enabled_ui(app.hist.can_redo(), |ui| {
                    menu_item(ui, &rl, Some(Action::Redo))
                })
                .inner
            {
                app.redo(&ctx);
            }
            ui.separator();
            if menu_item(ui, tr("menu.edit.paste_image"), Some(Action::PasteImage)) {
                app.paste_image_from_clipboard(&ctx);
            }
            ui.separator();
            if menu_item(ui, tr("menu.edit.clear"), Some(Action::ClearLayer)) {
                app.clear_layer();
            }
            if menu_item(ui, tr("menu.edit.fill_layer"), None) {
                app.fill_layer();
            }
            ui.separator();
            if menu_item(ui, tr("menu.edit.offset"), None) {
                app.dlg.offset = Some(90.0);
            }
            if menu_item(ui, tr("menu.edit.flip"), None) {
                app.flip_horizontal();
            }
            ui.separator();
            if menu_item(ui, tr("action.swap_colors"), Some(Action::SwapColors)) {
                app.swap_colors();
            }
        });
        ui.menu_button(tr("menu.layer"), |ui| {
            if menu_item(ui, tr("menu.layer.new"), Some(Action::NewLayer)) {
                app.new_layer(&ctx);
            }
            if menu_item(ui, tr("menu.layer.duplicate"), Some(Action::DuplicateLayer)) {
                app.duplicate_layer(&ctx);
            }
            if menu_item(ui, tr("menu.layer.merge_down"), Some(Action::MergeDown)) {
                app.merge_down(&ctx);
            }
            if menu_item(ui, tr("menu.layer.rename"), None) {
                if let Some(l) = app.doc.active_layer() {
                    app.dlg.rename = Some((l.id, l.name.clone()));
                }
            }
            ui.separator();
            if menu_item(ui, tr("menu.layer.delete"), None) {
                app.delete_layer(&ctx);
            }
        });
        ui.menu_button(tr("menu.view"), |ui| {
            let label = match app.mode {
                ViewMode::Sphere => tr("view.flat"),
                ViewMode::Flat => tr("view.sphere"),
            };
            if menu_item(ui, label, Some(Action::ToggleView)) {
                app.toggle_view();
            }
            let mut g = app.s.show_grid;
            if ui.checkbox(&mut g, tr("menu.view.grid")).changed() {
                app.s.show_grid = g;
            }
            ui.separator();
            if menu_item(ui, tr("menu.view.zoom_in"), Some(Action::ZoomIn)) {
                app.zoom(1.0 / 1.2);
            }
            if menu_item(ui, tr("menu.view.zoom_out"), Some(Action::ZoomOut)) {
                app.zoom(1.2);
            }
            if menu_item(ui, tr("menu.view.reset"), Some(Action::ResetView)) {
                let now = ctx.input(|i| i.time);
                app.reset_view(now);
            }
            ui.separator();
            if menu_item(ui, tr("menu.view.hide_ui"), Some(Action::HideUi)) {
                app.ui_hidden = true;
            }
            if menu_item(ui, tr("menu.view.fullscreen"), Some(Action::Fullscreen)) {
                app.run_action(&ctx, Action::Fullscreen);
            }
            ui.separator();
            ui.menu_button(tr("settings.theme"), |ui| {
                for (c, k) in [
                    (ThemeChoice::System, "settings.theme.system"),
                    (ThemeChoice::Dark, "settings.theme.dark"),
                    (ThemeChoice::Light, "settings.theme.light"),
                ] {
                    if ui
                        .radio(app.s.theme == c && app.cli.theme.is_none(), tr(k))
                        .clicked()
                    {
                        app.s.theme = c;
                        app.cli.theme = None;
                    }
                }
            });
            ui.menu_button(tr("settings.language"), |ui| {
                for l in panoraminal_i18n::Lang::ALL {
                    let cur = panoraminal_i18n::lang() == l;
                    if ui.radio(cur, l.native_name()).clicked() {
                        app.s.language = Some(l.code().to_string());
                        app.cli.lang = None;
                    }
                }
            });
        });
        ui.menu_button(tr("menu.help"), |ui| {
            if menu_item(ui, tr("menu.help.help"), Some(Action::Help)) {
                app.dlg.help = true;
                app.dlg.help_tab = 0;
            }
            if menu_item(ui, tr("menu.help.tools"), None) {
                app.dlg.help = true;
                app.dlg.help_tab = 1;
            }
            ui.separator();
            if ui.button(tr("about.repo")).clicked() {
                widgets::open_url(&ctx, crate::REPO_URL);
            }
            if ui.button(tr("about.donate")).clicked() {
                widgets::open_url(&ctx, crate::DONATE_URL);
            }
            ui.separator();
            if menu_item(ui, tr("menu.help.about"), None) {
                app.dlg.about = true;
            }
        });
    });
}

fn tool_bar(app: &mut PanoApp, ui: &mut egui::Ui, now: f64) {
    let pal = app.pal;
    let ctx = ui.ctx().clone();
    ui.vertical_centered(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        for t in Tool::ALL {
            let sel = app.tool == t;
            let tip = format!("{} ({})", tr(t.label()), first_text(&ctx, t.action()));
            let r = icon_button(ui, &pal, t.glyph(), sel, &tip, 40.0);
            if sel && app.anim {
                // Subtle breathing glow on the active tool.
                let k = 0.6 + 0.4 * ((now * 2.4).sin() as f32 * 0.5 + 0.5);
                theme::glow_rect(
                    ui.painter(),
                    r.rect,
                    10.0,
                    pal.neon,
                    0.35 * k * app.s.glow.factor(),
                );
            }
            if r.clicked() {
                app.set_tool(t);
            }
            if t == Tool::Paint(ToolKind::Eraser) {
                ui.add_space(2.0);
                let (r, _) = ui.allocate_exact_size(Vec2::new(30.0, 1.0), Sense::hover());
                ui.painter()
                    .hline(r.x_range(), r.center().y, Stroke::new(1.0, pal.stroke));
                ui.add_space(2.0);
            }
        }
        ui.add_space(12.0);
        // Primary / secondary colour chips.
        let (rect, _) = ui.allocate_exact_size(Vec2::new(44.0, 44.0), Sense::hover());
        let sec = Rect::from_min_size(rect.min + Vec2::new(16.0, 16.0), Vec2::splat(26.0));
        let pri = Rect::from_min_size(rect.min + Vec2::new(2.0, 2.0), Vec2::splat(26.0));
        let c = |c: [u8; 3]| Color32::from_rgb(c[0], c[1], c[2]);
        let rs = ui
            .interact(sec, ui.scope_id().with("sec"), Sense::click())
            .on_hover_text(tr("color.secondary"));
        ui.painter().rect_filled(sec, 6.0, c(app.secondary));
        ui.painter().rect_stroke(
            sec,
            6.0,
            Stroke::new(1.5, pal.stroke),
            egui::StrokeKind::Outside,
        );
        theme::glow_rect(
            ui.painter(),
            pri,
            6.0,
            c(app.primary),
            0.5 * app.s.glow.factor(),
        );
        ui.painter().rect_filled(pri, 6.0, c(app.primary));
        ui.painter().rect_stroke(
            pri,
            6.0,
            Stroke::new(
                2.0,
                if pal.dark {
                    brand::SNOW
                } else {
                    Color32::WHITE
                },
            ),
            egui::StrokeKind::Outside,
        );
        ui.interact(pri, ui.scope_id().with("pri"), Sense::hover())
            .on_hover_text(tr("color.primary"));
        if rs.clicked() {
            app.swap_colors();
        }
        if icon_button(
            ui,
            &pal,
            Glyph::Swap,
            false,
            &format!("{} (X)", tr("action.swap_colors")),
            26.0,
        )
        .clicked()
        {
            app.swap_colors();
        }
    });
}

fn side_panel(app: &mut PanoApp, ui: &mut egui::Ui, now: f64) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let pal = app.pal;
            theme::card(&pal).show(ui, |ui| {
                ui.set_width(ui.available_width());
                section_title(ui, &pal, tr("panel.color"));
                color::color_card(app, ui);
            });
            ui.add_space(8.0);
            theme::card(&pal).show(ui, |ui| {
                ui.set_width(ui.available_width());
                section_title(ui, &pal, tr(app.tool.label()));
                color::brush_card(app, ui);
            });
            ui.add_space(8.0);
            theme::card(&pal).show(ui, |ui| {
                ui.set_width(ui.available_width());
                section_title(ui, &pal, tr("panel.layers"));
                layers::layers_card(app, ui, now);
            });
        });
}

fn status_bar(app: &mut PanoApp, ui: &mut egui::Ui) {
    let pal = app.pal;
    let rect = ui.max_rect().expand2(Vec2::new(10.0, 3.0));
    ui.painter().hline(
        rect.x_range(),
        rect.top(),
        Stroke::new(1.0, with_alpha(pal.accent, 0.35)),
    );
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let small = |s: String| egui::RichText::new(s).size(12.0);
        ui.label(small(tr(app.tool.label()).to_string()).color(pal.accent));
        if let Tool::Paint(_) = app.tool {
            ui.label(small(trf(
                "status.size",
                &[("px", &(app.brush().size.round()))],
            )));
        }
        if let Some(d) = app.cursor_dir {
            let (lon, lat) = panoraminal_core::math::lonlat_of(d);
            ui.label(small(trf(
                "status.cursor",
                &[
                    ("lon", &format!("{:.1}", lon.to_degrees())),
                    ("lat", &format!("{:.1}", lat.to_degrees())),
                ],
            )));
        }
        let view = match app.mode {
            ViewMode::Sphere => trf("status.zoom", &[("fov", &format!("{:.0}", app.cam.fov))]),
            ViewMode::Flat => trf(
                "status.flat_zoom",
                &[(
                    "pct",
                    &format!(
                        "{:.0}",
                        app.flat
                            .scale(app.canvas_rect.width() as f64, app.doc.width)
                            * 100.0
                    ),
                )],
            ),
        };
        ui.label(small(view));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(small(app.gpu_info.clone()).color(pal.text_weak))
                .on_hover_text(tr("status.gpu"));
            ui.label(
                small(trf(
                    "status.doc",
                    &[
                        ("w", &app.doc.width),
                        ("h", &app.doc.height),
                        ("n", &app.doc.layers.len()),
                    ],
                ))
                .color(pal.text_weak),
            );
            ui.label(
                small(trf(
                    "status.undo",
                    &[("mb", &format!("{:.0}", app.hist.bytes() as f64 / 1048576.0))],
                ))
                .color(pal.text_weak),
            );
            if let Some(b) = app.busy_label {
                ui.add(egui::Spinner::new().size(12.0).color(pal.neon));
                ui.label(small(tr(b).to_string()).color(pal.neon));
            } else if app.is_modified() {
                ui.label(small(format!("● {}", tr("status.modified"))).color(pal.accent));
            }
        });
    });
}

/// Toasts, splash and the "UI hidden" hint, drawn above everything.
pub fn overlay(app: &mut PanoApp, ctx: &egui::Context, now: f64) {
    let pal = app.pal;
    let screen = ctx.content_rect();
    // Toasts.
    let life = 3.2;
    app.toasts.retain(|t| now - t.born < life);
    if !app.toasts.is_empty() {
        ctx.request_repaint_after(std::time::Duration::from_millis(if app.anim {
            16
        } else {
            250
        }));
    }
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::unique("toasts"),
    ));
    // Toasts stack at the bottom centre of the canvas so they never cover panels.
    let area = if app.ui_hidden || app.canvas_rect.width() < 50.0 {
        screen
    } else {
        app.canvas_rect
    };
    let mut y = area.bottom() - 16.0;
    for t in app.toasts.iter().rev() {
        let age = (now - t.born) as f32;
        let a_in = if app.anim { (age / 0.22).min(1.0) } else { 1.0 };
        let a_out = ((life as f32 - age) / 0.4).clamp(0.0, 1.0);
        let alpha = a_in.min(a_out);
        let ease = 1.0 - (1.0 - a_in).powi(3);
        let galley = painter.layout(t.text.clone(), FontId::proportional(13.5), pal.text, 360.0);
        let size = galley.size() + Vec2::new(36.0, 18.0);
        let x = area.center().x - size.x * 0.5;
        let rect = Rect::from_min_size(Pos2::new(x, y - size.y + (1.0 - ease) * 24.0), size);
        let accent = match t.kind {
            ToastKind::Ok => pal.ok,
            ToastKind::Error => pal.danger,
            ToastKind::Info => pal.neon,
        };
        theme::glow_rect(
            &painter,
            rect,
            10.0,
            accent,
            0.5 * alpha * app.s.glow.factor(),
        );
        painter.rect_filled(rect, 10.0, with_alpha(pal.raised, 0.97 * alpha));
        painter.rect_stroke(
            rect,
            10.0,
            Stroke::new(1.0, with_alpha(accent, alpha)),
            egui::StrokeKind::Inside,
        );
        painter.circle_filled(
            Pos2::new(rect.left() + 14.0, rect.center().y),
            4.0,
            with_alpha(accent, alpha),
        );
        painter.galley_with_override_text_color(
            rect.min + Vec2::new(26.0, 9.0),
            galley,
            with_alpha(pal.text, alpha),
        );
        y -= size.y + 8.0;
    }

    if app.ui_hidden {
        let p = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::unique("hint"),
        ));
        p.text(
            Pos2::new(screen.center().x, screen.top() + 14.0),
            Align2::CENTER_TOP,
            tr("hint.show_ui"),
            FontId::proportional(12.0),
            with_alpha(pal.text_weak, 0.8),
        );
    }

    // Splash.
    if now < app.splash_until {
        splash(app, ctx, now, screen);
    }
}

fn splash(app: &mut PanoApp, ctx: &egui::Context, now: f64, screen: Rect) {
    let total = 1.9f32;
    let t = (total - (app.splash_until - now) as f32).max(0.0);
    let fade = if t > total - 0.5 {
        (total - t) / 0.5
    } else {
        1.0
    };
    let pal = app.pal;
    let id = egui::Id::unique("splash");
    let area = egui::Area::new(id)
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .interactable(true);
    area.show(ctx, |ui| {
        let (rect, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
        if resp.clicked() {
            app.splash_until = 0.0;
        }
        let p = ui.painter();
        p.rect_filled(rect, 0.0, with_alpha(pal.bg, 0.94 * fade));
        let c = rect.center() - Vec2::new(0.0, 30.0);
        let appear = (t / 0.6).min(1.0);
        let e = 1.0 - (1.0 - appear).powi(3);
        let size = 120.0 + 24.0 * e;
        // Orbiting neon ring.
        let ring_r = size * 0.72;
        let a = t * 3.2;
        theme::glow_circle(
            p,
            c,
            ring_r,
            with_alpha(brand::AZURE, fade),
            0.8 * fade * app.s.glow.factor().max(0.3),
        );
        p.circle_stroke(
            c,
            ring_r,
            Stroke::new(2.0, with_alpha(brand::CYAN, 0.7 * fade)),
        );
        for k in 0..3 {
            let ang = a + k as f32 * std::f32::consts::TAU / 3.0;
            let d = c + Vec2::angled(ang) * ring_r;
            theme::glow_circle(p, d, 4.0, with_alpha(brand::CYAN, fade), fade);
            p.circle_filled(d, 4.0, with_alpha(brand::ICE, fade));
        }
        let img_rect = Rect::from_center_size(c, Vec2::splat(size));
        egui::Image::new(&app.icons.logo)
            .tint(Color32::from_white_alpha((255.0 * fade * e) as u8))
            .paint_at(ui, img_rect);
        let job = theme::gradient_text(
            crate::APP_NAME,
            FontId::proportional(34.0),
            &[
                brand::SNOW,
                brand::ICE,
                brand::CYAN,
                brand::AZURE,
                brand::ROYAL,
            ],
            t * 0.4,
        );
        let galley = ui.painter().layout_job(job);
        let pos = Pos2::new(
            c.x - galley.size().x * 0.5,
            c.y + size * 0.5 + ring_r * 0.3 + 6.0,
        );
        ui.painter().galley(pos, galley, Color32::WHITE);
        ui.painter().text(
            Pos2::new(c.x, pos.y + 48.0),
            Align2::CENTER_TOP,
            format!("{} · v{}", tr("app.tagline"), crate::VERSION),
            FontId::proportional(14.0),
            with_alpha(if pal.dark { brand::ICE } else { brand::ROYAL }, fade),
        );
    });
    ctx.request_repaint();
}
