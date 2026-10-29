//! Modal dialogs and the settings window.

use super::widgets::{primary_button, slider_row, toggle};
use crate::app::{PanoApp, Pending};
use crate::settings::{AnimChoice, Settings, ThemeChoice};
use crate::theme::Glow;
use eframe::egui::{self, Align, Id, Layout, RichText};
use panoraminal_core::document::Background;
use panoraminal_core::history::ops;
use panoraminal_i18n::{tr, trf, Lang};

pub fn show_all(app: &mut PanoApp, ctx: &egui::Context) {
    super::help::help_window(app, ctx);
    super::help::about_window(app, ctx);
    settings_window(app, ctx);
    new_dialog(app, ctx);
    export_dialog(app, ctx);
    offset_dialog(app, ctx);
    rename_dialog(app, ctx);
    recovery_dialog(app, ctx);
    unsaved_dialog(app, ctx);
    error_dialog(app, ctx);
}

fn title(ui: &mut egui::Ui, app: &PanoApp, text: &str) {
    let pal = app.pal;
    ui.horizontal(|ui| {
        ui.add(egui::Image::new(&app.icons.favicon).fit_to_exact_size(egui::Vec2::splat(22.0)));
        ui.label(RichText::new(text).size(18.0).strong().color(pal.text));
    });
    ui.add_space(2.0);
    let (r, _) = ui.allocate_exact_size(
        egui::Vec2::new(ui.available_width().max(260.0), 2.0),
        egui::Sense::hover(),
    );
    super::neon_line(ui.painter(), r, 0.0, app.s.glow.factor() * 0.6);
    ui.add_space(8.0);
}

fn buttons_right(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(10.0);
    ui.with_layout(Layout::right_to_left(Align::Center), add);
}

fn new_dialog(app: &mut PanoApp, ctx: &egui::Context) {
    let Some(mut d) = app.dlg.new.clone() else {
        return;
    };
    let mut close = false;
    let mut create = false;
    let max = app.max_dim;
    let r = egui::Modal::new(Id::unique("dlg_new")).show(ctx, |ui| {
        ui.set_width(420.0);
        title(ui, app, tr("new.title"));
        ui.label(RichText::new(tr("new.presets")).color(app.pal.text_weak));
        ui.horizontal_wrapped(|ui| {
            for (w, h, k) in [
                (2048u32, 1024u32, "new.preset.small"),
                (4096, 2048, "new.preset.medium"),
                (6144, 3072, "new.preset.large"),
                (8192, 4096, "new.preset.huge"),
            ] {
                ui.add_enabled_ui(w <= max, |ui| {
                    if ui
                        .selectable_label(
                            d.width == w && d.height == h,
                            format!("{} · {w}×{h}", tr(k)),
                        )
                        .clicked()
                    {
                        d.width = w;
                        d.height = h;
                    }
                });
            }
        });
        ui.add_space(6.0);
        egui::Grid::new("new_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label(tr("new.width"));
                let rw = ui.add(
                    egui::DragValue::new(&mut d.width)
                        .range(256..=max)
                        .suffix(" px"),
                );
                ui.end_row();
                ui.label(tr("new.height"));
                let rh = ui.add_enabled(
                    !d.lock_ratio,
                    egui::DragValue::new(&mut d.height)
                        .range(128..=max)
                        .suffix(" px"),
                );
                ui.end_row();
                if d.lock_ratio && rw.changed() {
                    d.height = (d.width / 2).max(128);
                }
                let _ = rh;
                ui.label("");
                toggle(ui, &app.pal, &mut d.lock_ratio, tr("new.lock_ratio"));
                ui.end_row();
                ui.label(tr("new.background"));
                egui::ComboBox::from_id_salt("new_bg")
                    .selected_text(tr(bg_key(d.background)))
                    .show_ui(ui, |ui| {
                        for b in [
                            Background::Sky,
                            Background::Night,
                            Background::White,
                            Background::Black,
                            Background::Transparent,
                        ] {
                            ui.selectable_value(&mut d.background, b, tr(bg_key(b)));
                        }
                    });
                ui.end_row();
            });
        if d.lock_ratio {
            d.height = (d.width / 2).max(128);
        }
        let mb = d.width as f64 * d.height as f64 * 4.0 / 1048576.0;
        ui.add_space(4.0);
        ui.label(
            RichText::new(trf("new.memory", &[("mb", &format!("{mb:.0}"))]))
                .size(12.0)
                .color(app.pal.text_weak),
        );
        if d.width != d.height * 2 {
            ui.label(
                RichText::new(tr("new.ratio_warning"))
                    .size(12.0)
                    .color(ui.visuals().warn_fg_color),
            );
        }
        buttons_right(ui, |ui| {
            if primary_button(ui, &app.pal, tr("new.create"), app.s.glow.factor()).clicked() {
                create = true;
            }
            if ui.button(tr("common.cancel")).clicked() {
                close = true;
            }
        });
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            create = true;
        }
    });
    if r.should_close() {
        close = true;
    }
    if create {
        app.create_new(&d);
        app.dlg.new = None;
    } else if close {
        app.dlg.new = None;
    } else {
        app.dlg.new = Some(d);
    }
}

pub fn bg_key(b: Background) -> &'static str {
    match b {
        Background::White => "bg.white",
        Background::Black => "bg.black",
        Background::Transparent => "bg.transparent",
        Background::Sky => "bg.sky",
        Background::Night => "bg.night",
    }
}

fn export_dialog(app: &mut PanoApp, ctx: &egui::Context) {
    if !app.dlg.export_jpg {
        return;
    }
    let mut close = false;
    let mut go = false;
    let r = egui::Modal::new(Id::unique("dlg_export")).show(ctx, |ui| {
        ui.set_width(400.0);
        title(ui, app, tr("export.title"));
        ui.label(RichText::new(tr("export.desc")).color(app.pal.text_weak));
        ui.add_space(6.0);
        slider_row(
            ui,
            tr("export.quality"),
            &mut app.s.jpeg_quality,
            50..=100,
            "",
            false,
        );
        toggle(ui, &app.pal, &mut app.s.jpeg_gpano, tr("export.gpano"))
            .on_hover_text(tr("export.gpano_tip"));
        ui.add_space(4.0);
        ui.label(
            RichText::new(trf(
                "export.size",
                &[("w", &app.doc.width), ("h", &app.doc.height)],
            ))
            .size(12.0)
            .color(app.pal.text_weak),
        );
        buttons_right(ui, |ui| {
            if primary_button(ui, &app.pal, tr("export.button"), app.s.glow.factor()).clicked() {
                go = true;
            }
            if ui.button(tr("common.cancel")).clicked() {
                close = true;
            }
        });
    });
    if r.should_close() || close {
        app.dlg.export_jpg = false;
    }
    if go {
        app.dlg.export_jpg = false;
        app.export(ctx, true);
    }
}

fn offset_dialog(app: &mut PanoApp, ctx: &egui::Context) {
    let Some(mut deg) = app.dlg.offset else {
        return;
    };
    let mut close = false;
    let mut apply = false;
    let r = egui::Modal::new(Id::unique("dlg_offset")).show(ctx, |ui| {
        ui.set_width(380.0);
        title(ui, app, tr("offset.title"));
        ui.label(RichText::new(tr("offset.desc")).color(app.pal.text_weak));
        ui.add_space(6.0);
        slider_row(ui, tr("offset.angle"), &mut deg, -180.0..=180.0, "°", false);
        ui.horizontal(|ui| {
            for v in [-90.0f32, -45.0, 45.0, 90.0, 180.0] {
                if ui.small_button(format!("{v:+.0}°")).clicked() {
                    deg = v;
                }
            }
        });
        buttons_right(ui, |ui| {
            if primary_button(ui, &app.pal, tr("common.apply"), app.s.glow.factor()).clicked() {
                apply = true;
            }
            if ui.button(tr("common.cancel")).clicked() {
                close = true;
            }
        });
    });
    app.dlg.offset = Some(deg);
    if r.should_close() || close {
        app.dlg.offset = None;
    }
    if apply {
        app.dlg.offset = None;
        if deg.abs() > 0.01 {
            app.offset_heading(deg);
        }
    }
}

fn rename_dialog(app: &mut PanoApp, ctx: &egui::Context) {
    let Some((id, mut name)) = app.dlg.rename.clone() else {
        return;
    };
    let mut close = false;
    let mut apply = false;
    let r = egui::Modal::new(Id::unique("dlg_rename")).show(ctx, |ui| {
        ui.set_width(340.0);
        title(ui, app, tr("menu.layer.rename"));
        let te = ui.add(
            egui::TextEdit::singleline(&mut name)
                .desired_width(f32::INFINITY)
                .char_limit(64),
        );
        if !te.has_focus() && !te.lost_focus() {
            te.request_focus();
        }
        if te.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            apply = true;
        }
        buttons_right(ui, |ui| {
            if primary_button(ui, &app.pal, tr("common.ok"), app.s.glow.factor()).clicked() {
                apply = true;
            }
            if ui.button(tr("common.cancel")).clicked() {
                close = true;
            }
        });
    });
    if apply {
        let n = name.trim();
        if let Some(mut p) = app.doc.props(id) {
            if !n.is_empty() && n != p.name {
                p.name = n.to_string();
                app.finish_stroke();
                ops::set_props(&mut app.doc, &mut app.hist, id, p, "hist.rename");
            }
        }
        app.dlg.rename = None;
    } else if r.should_close() || close {
        app.dlg.rename = None;
    } else {
        app.dlg.rename = Some((id, name));
    }
}

fn recovery_dialog(app: &mut PanoApp, ctx: &egui::Context) {
    let Some((path, when)) = app.dlg.recovery.clone() else {
        return;
    };
    let mut choice = None;
    egui::Modal::new(Id::unique("dlg_recovery")).show(ctx, |ui| {
        ui.set_width(400.0);
        title(ui, app, tr("recovery.title"));
        ui.label(trf("recovery.desc", &[("when", &when)]));
        buttons_right(ui, |ui| {
            if primary_button(ui, &app.pal, tr("recovery.restore"), app.s.glow.factor()).clicked() {
                choice = Some(true);
            }
            if ui.button(tr("recovery.discard")).clicked() {
                choice = Some(false);
            }
        });
    });
    match choice {
        Some(true) => {
            app.dlg.recovery = None;
            app.busy_label = Some("busy.opening");
            app.jobs.recover(ctx, path);
        }
        Some(false) => {
            app.dlg.recovery = None;
            app.discard_recovery();
        }
        None => {}
    }
}

fn unsaved_dialog(app: &mut PanoApp, ctx: &egui::Context) {
    let Some(p) = app.dlg.unsaved.clone() else {
        return;
    };
    let mut choice = 0;
    let r = egui::Modal::new(Id::unique("dlg_unsaved")).show(ctx, |ui| {
        ui.set_width(420.0);
        title(ui, app, tr("unsaved.title"));
        ui.label(trf("unsaved.desc", &[("file", &app.doc_name())]));
        buttons_right(ui, |ui| {
            if primary_button(ui, &app.pal, tr("unsaved.save"), app.s.glow.factor()).clicked() {
                choice = 1;
            }
            if ui.button(tr("unsaved.discard")).clicked() {
                choice = 2;
            }
            if ui.button(tr("common.cancel")).clicked() {
                choice = 3;
            }
        });
    });
    if r.should_close() && choice == 0 {
        choice = 3;
    }
    match choice {
        1 => {
            app.dlg.unsaved = None;
            app.save(ctx, Some(p));
        }
        2 => {
            app.dlg.unsaved = None;
            if matches!(p, Pending::Quit) {
                app.allow_close = true;
            }
            app.run_pending(ctx, p);
        }
        3 => app.dlg.unsaved = None,
        _ => {}
    }
}

fn error_dialog(app: &mut PanoApp, ctx: &egui::Context) {
    let Some(msg) = app.dlg.error.clone() else {
        return;
    };
    let mut close = false;
    let r = egui::Modal::new(Id::unique("dlg_error")).show(ctx, |ui| {
        ui.set_width(420.0);
        title(ui, app, tr("error.title"));
        ui.label(RichText::new(msg).color(app.pal.text));
        buttons_right(ui, |ui| {
            if primary_button(ui, &app.pal, tr("common.ok"), app.s.glow.factor()).clicked() {
                close = true;
            }
        });
    });
    if close || r.should_close() {
        app.dlg.error = None;
    }
}

fn settings_window(app: &mut PanoApp, ctx: &egui::Context) {
    if !app.dlg.settings {
        return;
    }
    let mut open = true;
    let pal = app.pal;
    egui::Window::new(tr("settings.title"))
        .id(Id::unique("win_settings"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size([460.0, 600.0])
        .max_height((ctx.content_rect().height() - 60.0).max(240.0))
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let s = &mut app.s;
                    super::widgets::section_title(ui, &pal, tr("settings.appearance"));
                    egui::Grid::new("set_app")
                        .num_columns(2)
                        .spacing([14.0, 8.0])
                        .show(ui, |ui| {
                            ui.label(tr("settings.language"));
                            let cur = panoraminal_i18n::lang();
                            egui::ComboBox::from_id_salt("lang")
                                .selected_text(cur.native_name())
                                .show_ui(ui, |ui| {
                                    if ui
                                        .selectable_label(
                                            s.language.is_none(),
                                            tr("settings.language.auto"),
                                        )
                                        .clicked()
                                    {
                                        s.language = None;
                                        app.cli.lang = None;
                                    }
                                    for l in Lang::ALL {
                                        if ui
                                            .selectable_label(
                                                s.language.as_deref() == Some(l.code()),
                                                l.native_name(),
                                            )
                                            .clicked()
                                        {
                                            s.language = Some(l.code().into());
                                            app.cli.lang = None;
                                        }
                                    }
                                });
                            ui.end_row();
                            ui.label(tr("settings.theme"));
                            ui.horizontal(|ui| {
                                for (c, k) in [
                                    (ThemeChoice::System, "settings.theme.system"),
                                    (ThemeChoice::Dark, "settings.theme.dark"),
                                    (ThemeChoice::Light, "settings.theme.light"),
                                ] {
                                    if ui.selectable_label(s.theme == c, tr(k)).clicked() {
                                        s.theme = c;
                                        app.cli.theme = None;
                                    }
                                }
                            });
                            ui.end_row();
                            ui.label(tr("settings.animations"));
                            ui.horizontal(|ui| {
                                for (c, k) in [
                                    (AnimChoice::Auto, "settings.anim.auto"),
                                    (AnimChoice::On, "settings.anim.on"),
                                    (AnimChoice::Off, "settings.anim.off"),
                                ] {
                                    ui.selectable_value(&mut s.animations, c, tr(k));
                                }
                            });
                            ui.end_row();
                            ui.label(tr("settings.glow"));
                            ui.horizontal(|ui| {
                                for (c, k) in [
                                    (Glow::Full, "settings.glow.full"),
                                    (Glow::Reduced, "settings.glow.reduced"),
                                    (Glow::Off, "settings.glow.off"),
                                ] {
                                    ui.selectable_value(&mut s.glow, c, tr(k));
                                }
                            });
                            ui.end_row();
                            ui.label(tr("settings.ui_scale"));
                            ui.add(
                                egui::Slider::new(&mut s.ui_scale, 0.75..=2.0)
                                    .step_by(0.05)
                                    .suffix("×"),
                            );
                            ui.end_row();
                            ui.label("");
                            toggle(ui, &pal, &mut s.show_splash, tr("settings.splash"));
                            ui.end_row();
                        });
                    ui.label(
                        RichText::new(tr("settings.anim_tip"))
                            .size(11.5)
                            .color(pal.text_weak),
                    );
                    ui.add_space(10.0);
                    super::widgets::section_title(ui, &pal, tr("settings.editor"));
                    egui::Grid::new("set_ed")
                        .num_columns(2)
                        .spacing([14.0, 8.0])
                        .show(ui, |ui| {
                            ui.label(tr("settings.autosave"));
                            ui.add(
                                egui::Slider::new(&mut s.autosave_minutes, 0..=30).suffix(" min"),
                            )
                            .on_hover_text(tr("settings.autosave_tip"));
                            ui.end_row();
                            ui.label(tr("settings.undo_memory"));
                            ui.add(
                                egui::Slider::new(&mut s.undo_memory_mb, 128..=8192)
                                    .logarithmic(true)
                                    .suffix(" MB"),
                            );
                            ui.end_row();
                            ui.label(tr("settings.zoom_speed"));
                            ui.add(
                                egui::Slider::new(&mut s.wheel_zoom_speed, 0.25..=3.0).suffix("×"),
                            );
                            ui.end_row();
                            ui.label("");
                            ui.vertical(|ui| {
                                toggle(ui, &pal, &mut s.pen_pressure, tr("settings.pen_pressure"))
                                    .on_hover_text(tr("settings.pen_pressure_tip"));
                                toggle(ui, &pal, &mut s.invert_drag, tr("settings.invert_drag"));
                                toggle(
                                    ui,
                                    &pal,
                                    &mut s.confirm_on_close,
                                    tr("settings.confirm_close"),
                                );
                            });
                            ui.end_row();
                        });
                    ui.add_space(10.0);
                    super::widgets::section_title(ui, &pal, tr("settings.export"));
                    egui::Grid::new("set_ex")
                        .num_columns(2)
                        .spacing([14.0, 8.0])
                        .show(ui, |ui| {
                            ui.label(tr("export.quality"));
                            ui.add(egui::Slider::new(&mut s.jpeg_quality, 50..=100));
                            ui.end_row();
                            ui.label("");
                            toggle(ui, &pal, &mut s.jpeg_gpano, tr("export.gpano"));
                            ui.end_row();
                        });
                    ui.add_space(10.0);
                    if let Some(p) = crate::settings::settings_path() {
                        ui.label(
                            RichText::new(trf("settings.path", &[("path", &p.display())]))
                                .size(11.0)
                                .color(pal.text_weak),
                        );
                    }
                    ui.horizontal(|ui| {
                        if ui.button(tr("settings.reset")).clicked() {
                            let keep_recent = s.recent.clone();
                            *s = Settings::default();
                            s.recent = keep_recent;
                        }
                    });
                });
            app.hist.max_bytes = app.s.undo_memory_mb as usize * 1024 * 1024;
        });
    if !open {
        app.dlg.settings = false;
        if app.screenshot.is_none() {
            app.s.save();
        }
    }
}
