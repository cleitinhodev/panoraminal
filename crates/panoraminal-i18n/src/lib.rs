//! # panoraminal-i18n
//!
//! Flat `key → text` JSON catalogs embedded in the binary. Missing keys fall back to
//! English, then to the key itself. Arguments use `{name}` placeholders.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    PtBr,
    En,
    Es,
    Fr,
    De,
    It,
    Ja,
    ZhCn,
}

impl Lang {
    pub const ALL: [Lang; 8] = [
        Lang::PtBr,
        Lang::En,
        Lang::Es,
        Lang::Fr,
        Lang::De,
        Lang::It,
        Lang::Ja,
        Lang::ZhCn,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Lang::PtBr => "pt-BR",
            Lang::En => "en",
            Lang::Es => "es",
            Lang::Fr => "fr",
            Lang::De => "de",
            Lang::It => "it",
            Lang::Ja => "ja",
            Lang::ZhCn => "zh-CN",
        }
    }
    /// Name of the language in that language.
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::PtBr => "Português (Brasil)",
            Lang::En => "English",
            Lang::Es => "Español",
            Lang::Fr => "Français",
            Lang::De => "Deutsch",
            Lang::It => "Italiano",
            Lang::Ja => "日本語",
            Lang::ZhCn => "简体中文",
        }
    }
    pub fn needs_cjk(self) -> bool {
        matches!(self, Lang::Ja | Lang::ZhCn)
    }
    fn index(self) -> usize {
        Self::ALL.iter().position(|l| *l == self).unwrap_or(1)
    }
    fn source(self) -> &'static str {
        match self {
            Lang::PtBr => include_str!("../locales/pt-BR.json"),
            Lang::En => include_str!("../locales/en.json"),
            Lang::Es => include_str!("../locales/es.json"),
            Lang::Fr => include_str!("../locales/fr.json"),
            Lang::De => include_str!("../locales/de.json"),
            Lang::It => include_str!("../locales/it.json"),
            Lang::Ja => include_str!("../locales/ja.json"),
            Lang::ZhCn => include_str!("../locales/zh-CN.json"),
        }
    }
    /// Parses a BCP-47-ish tag such as `pt_BR.UTF-8`, `en-US`, `zh-Hans-CN`, `ja`.
    pub fn from_tag(tag: &str) -> Option<Lang> {
        let t = tag
            .split('.')
            .next()
            .unwrap_or("")
            .replace('_', "-")
            .to_ascii_lowercase();
        let primary = t.split('-').next().unwrap_or("");
        Some(match primary {
            "pt" => Lang::PtBr,
            "en" => Lang::En,
            "es" => Lang::Es,
            "fr" => Lang::Fr,
            "de" => Lang::De,
            "it" => Lang::It,
            "ja" => Lang::Ja,
            "zh" => Lang::ZhCn,
            _ => return None,
        })
    }
    /// Best match for the operating-system locale (English if unsupported).
    pub fn detect() -> Lang {
        sys_locale::get_locales()
            .find_map(|l| Lang::from_tag(&l))
            .unwrap_or(Lang::En)
    }
}

type Catalog = HashMap<String, String>;

fn catalogs() -> &'static Vec<Catalog> {
    static C: OnceLock<Vec<Catalog>> = OnceLock::new();
    C.get_or_init(|| Lang::ALL.iter().map(|l| parse(l.source())).collect())
}

fn parse(src: &str) -> Catalog {
    let v: serde_json::Value = serde_json::from_str(src).expect("locale JSON must be valid");
    let mut out = HashMap::new();
    if let serde_json::Value::Object(m) = v {
        for (k, v) in m {
            if let serde_json::Value::String(s) = v {
                out.insert(k, s);
            }
        }
    }
    out
}

static CURRENT: AtomicUsize = AtomicUsize::new(1);

pub fn set_lang(l: Lang) {
    CURRENT.store(l.index(), Ordering::Relaxed);
}
pub fn lang() -> Lang {
    Lang::ALL[CURRENT.load(Ordering::Relaxed)]
}

/// Translates `key` in the current language.
pub fn tr(key: &str) -> &'static str {
    tr_in(lang(), key)
}

pub fn tr_in(l: Lang, key: &str) -> &'static str {
    let c = catalogs();
    if let Some(s) = c[l.index()].get(key) {
        return s.as_str();
    }
    if let Some(s) = c[Lang::En.index()].get(key) {
        return s.as_str();
    }
    // Leak-free fallback: unknown keys are returned via a static cache.
    static MISSING: OnceLock<std::sync::Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let mut m = MISSING.get_or_init(Default::default).lock().unwrap();
    if let Some(s) = m.get(key) {
        return s;
    }
    let s: &'static str = Box::leak(key.to_string().into_boxed_str());
    m.insert(key.to_string(), s);
    s
}

/// Translates and substitutes `{name}` placeholders.
pub fn trf(key: &str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut s = tr(key).to_string();
    for (k, v) in args {
        s = s.replace(&format!("{{{k}}}"), &v.to_string());
    }
    s
}

/// All keys of the English (reference) catalog.
pub fn keys() -> Vec<&'static str> {
    let mut v: Vec<_> = catalogs()[Lang::En.index()]
        .keys()
        .map(|s| s.as_str())
        .collect();
    v.sort();
    v
}

/// Every distinct character used by every catalog (used to subset the CJK font).
pub fn all_chars() -> std::collections::BTreeSet<char> {
    catalogs()
        .iter()
        .flat_map(|c| c.values().flat_map(|s| s.chars()))
        .chain(Lang::ALL.iter().flat_map(|l| l.native_name().chars()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_catalogs_are_complete() {
        let en = &catalogs()[Lang::En.index()];
        for l in Lang::ALL {
            let c = &catalogs()[l.index()];
            let missing: Vec<_> = en.keys().filter(|k| !c.contains_key(*k)).collect();
            assert!(missing.is_empty(), "{} is missing {:?}", l.code(), missing);
            let extra: Vec<_> = c.keys().filter(|k| !en.contains_key(*k)).collect();
            assert!(
                extra.is_empty(),
                "{} has unknown keys {:?}",
                l.code(),
                extra
            );
        }
    }

    #[test]
    fn placeholders_match() {
        let en = &catalogs()[Lang::En.index()];
        let ph = |s: &str| {
            let mut v: Vec<String> = s
                .split('{')
                .skip(1)
                .filter_map(|p| p.split_once('}').map(|x| x.0.to_string()))
                .collect();
            v.sort();
            v
        };
        for l in Lang::ALL {
            for (k, v) in &catalogs()[l.index()] {
                assert_eq!(ph(v), ph(&en[k]), "{} key {k}", l.code());
            }
        }
    }

    #[test]
    fn tags() {
        assert_eq!(Lang::from_tag("pt_BR.UTF-8"), Some(Lang::PtBr));
        assert_eq!(Lang::from_tag("zh-Hans-CN"), Some(Lang::ZhCn));
        assert_eq!(Lang::from_tag("en-US"), Some(Lang::En));
        assert_eq!(Lang::from_tag("ko-KR"), None);
    }

    #[test]
    fn fallback_and_args() {
        set_lang(Lang::En);
        assert_eq!(tr("no.such.key"), "no.such.key");
        assert!(trf("status.zoom", &[("fov", &75)]).contains("75"));
    }
}
