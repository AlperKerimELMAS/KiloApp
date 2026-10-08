//! The user's preferences, kept in the system's defaults store
//! (`~/Library/Preferences/<bundle id>.plist`), which the helpers share.

use objc2_foundation::{NSLocale, NSString, NSUserDefaults};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    System,
    English,
    Turkish,
}

pub fn appearance() -> Appearance {
    match get("appearance").as_deref() {
        Some("light") => Appearance::Light,
        Some("dark") => Appearance::Dark,
        _ => Appearance::System,
    }
}

pub fn set_appearance(a: Appearance) {
    set("appearance", ["system", "light", "dark"][a as usize]);
}

pub fn language() -> Language {
    match get("language").as_deref() {
        Some("en") => Language::English,
        Some("tr") => Language::Turkish,
        _ => Language::System,
    }
}

pub fn set_language(l: Language) {
    set("language", ["system", "en", "tr"][l as usize]);
}

/// The sidebar shows only its icons.
pub fn sidebar_collapsed() -> bool {
    get("sidebar").as_deref() == Some("collapsed")
}

pub fn set_sidebar_collapsed(collapsed: bool) {
    set("sidebar", if collapsed { "collapsed" } else { "expanded" });
}

/// The language YouTube's content comes in (`hl`): the chosen one, or the
/// Mac's first preferred language (`"en"`, `"tr"`, `"de"`…).
pub fn content_language() -> String {
    match language() {
        Language::English => "en".into(),
        Language::Turkish => "tr".into(),
        Language::System => system_language().unwrap_or_else(|| "en".into()),
    }
}

/// The primary subtag of the Mac's first preferred language, lowercased.
pub fn system_language() -> Option<String> {
    let first = NSLocale::preferredLanguages().firstObject()?.to_string();
    let code = first.split(['-', '_']).next()?.to_ascii_lowercase();
    (2..=3).contains(&code.len()).then_some(code)
}

fn get(key: &str) -> Option<String> {
    NSUserDefaults::standardUserDefaults().stringForKey(&NSString::from_str(key)).map(|s| s.to_string())
}

fn set(key: &str, value: &str) {
    // SAFETY: an NSString is a valid property-list value.
    unsafe { NSUserDefaults::standardUserDefaults().setObject_forKey(Some(&NSString::from_str(value)), &NSString::from_str(key)) };
}
