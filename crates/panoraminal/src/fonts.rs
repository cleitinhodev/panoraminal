//! Font setup: egui's default fonts plus a bundled CJK subset (Noto Sans CJK,
//! SIL Open Font License) so Japanese and Chinese UI text renders everywhere.
//! If a full system CJK font is available it is added as a further fallback.

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};
use std::sync::Arc;

static CJK_SUBSET: &[u8] = include_bytes!("../assets/fonts/NotoSansCJK-Panoraminal-subset.otf");

fn system_cjk() -> Option<Vec<u8>> {
    let candidates: &[&str] = &[
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\YuGothM.ttc",
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
    ];
    candidates.iter().find_map(|p| std::fs::read(p).ok())
}

pub fn install(ctx: &egui::Context, with_system_fallback: bool) {
    let mut defs = FontDefinitions::default();
    defs.font_data.insert(
        "cjk-subset".into(),
        Arc::new(FontData::from_static(CJK_SUBSET)),
    );
    let mut names = vec!["cjk-subset".to_string()];
    if with_system_fallback {
        if let Some(b) = system_cjk() {
            defs.font_data
                .insert("cjk-system".into(), Arc::new(FontData::from_owned(b)));
            names.push("cjk-system".into());
        }
    }
    for fam in [FontFamily::Proportional, FontFamily::Monospace] {
        let list = defs.families.entry(fam).or_default();
        list.extend(names.iter().cloned());
    }
    ctx.set_fonts(defs);
}
