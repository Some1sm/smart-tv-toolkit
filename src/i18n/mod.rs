//! Interface language. English is written in the code; other languages are
//! a dictionary from that English to the translation. The design helpers
//! (`ds::label`, buttons, cards…) translate fixed text on their own, so views
//! just pass English; values read from TVs pass through unchanged.
//!
//! Log lines stay in English on purpose: they match what people search for.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

mod es;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Es,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::En, Lang::Es];

    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Es => "es",
        }
    }

    pub fn from_code(code: &str) -> Option<Lang> {
        let code = code.trim().to_lowercase();
        Lang::ALL.into_iter().find(|l| code == l.code() || code.starts_with(&format!("{}-", l.code())) || code.starts_with(&format!("{}_", l.code())))
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

pub fn current() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Lang::Es,
        _ => Lang::En,
    }
}

/// The saved choice, else the computer's language, else English.
pub fn pick(saved: Option<&str>) -> Lang {
    saved
        .and_then(Lang::from_code)
        .or_else(|| sys_locale::get_locale().as_deref().and_then(Lang::from_code))
        .unwrap_or(Lang::En)
}

fn spanish() -> &'static HashMap<&'static str, &'static str> {
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| es::WORDS.iter().copied().collect())
}

/// `english` in the current language (unchanged if there's no translation).
pub fn tr(english: &str) -> &str {
    match current() {
        Lang::En => english,
        Lang::Es => spanish().get(english).copied().unwrap_or(english),
    }
}

/// [`tr`] for text built at run time: only exact matches translate.
pub fn tr_owned(english: String) -> String {
    match current() {
        Lang::En => english,
        Lang::Es => spanish().get(english.as_str()).map_or(english, |s| (*s).to_owned()),
    }
}

/// Translate `template`, then fill its `{}` in order.
pub fn trf(template: &'static str, args: &[&dyn std::fmt::Display]) -> String {
    fill(tr(template), args)
}

fn fill(template: &str, args: &[&dyn std::fmt::Display]) -> String {
    let mut out = String::new();
    let mut args = args.iter();
    let mut parts = template.split("{}").peekable();
    while let Some(part) = parts.next() {
        out.push_str(part);
        if parts.peek().is_some()
            && let Some(a) = args.next()
        {
            out.push_str(&a.to_string());
        }
    }
    out
}

/// Text a design helper shows: fixed English (`&str`) gets translated,
/// run-time text (`String`, `&String`) is shown as it is.
pub trait Text<'a> {
    fn localized(self) -> Cow<'a, str>;
}

impl<'a> Text<'a> for &'a str {
    fn localized(self) -> Cow<'a, str> {
        Cow::Borrowed(tr(self))
    }
}

impl<'a> Text<'a> for String {
    fn localized(self) -> Cow<'a, str> {
        Cow::Owned(self)
    }
}

impl<'a> Text<'a> for &'a String {
    fn localized(self) -> Cow<'a, str> {
        Cow::Borrowed(self.as_str())
    }
}

impl<'a> Text<'a> for Cow<'a, str> {
    fn localized(self) -> Cow<'a, str> {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // (The language is global, so tests don't switch it: other tests read it.)
    #[test]
    fn translates_and_fills() {
        assert_eq!(spanish().get("Overview"), Some(&"Resumen"));
        assert_eq!(spanish().get("com.netflix.ninja"), None);
        let template = spanish()["Version {} is available."];
        assert_eq!(fill(template, &[&"0.2.0"]), "La versión 0.2.0 está disponible.");
        assert_eq!(tr("Overview"), "Overview");
        assert_eq!(Lang::from_code("es-ES"), Some(Lang::Es));
        assert_eq!(Lang::from_code("en_US"), Some(Lang::En));
        assert_eq!(Lang::from_code("fr-FR"), None);
    }

    #[test]
    fn bloatware_reasons_are_translated() {
        for reason in crate::android::bloat::all_reasons() {
            assert!(spanish().contains_key(reason), "missing Spanish for: {reason}");
        }
    }

    #[test]
    fn dictionary_is_clean() {
        let mut seen = std::collections::HashSet::new();
        for (en, es) in es::WORDS {
            assert!(seen.insert(*en), "duplicate entry: {en}");
            assert_eq!(en.matches("{}").count(), es.matches("{}").count(), "placeholders differ: {en}");
            assert!(!es.is_empty(), "empty translation: {en}");
        }
    }
}
