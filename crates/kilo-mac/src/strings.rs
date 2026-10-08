//! Picks the language for Kilo's words (`kilo_core::strings`): the user's
//! choice (`settings::language`) or the Mac's.

pub use kilo_core::strings::{S, t};

use crate::settings::{self, Language};

/// Picks the language from the user's preference (call at launch, and when
/// the preference changes).
pub fn init() {
    let turkish = match settings::language() {
        Language::English => false,
        Language::Turkish => true,
        Language::System => settings::system_language().as_deref() == Some("tr"),
    };
    kilo_core::strings::set_language(if turkish { kilo_core::strings::Language::Turkish } else { kilo_core::strings::Language::English });
}
