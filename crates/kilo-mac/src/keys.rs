//! Keyboard shortcuts on macOS (`kilo_core::shortcuts`). The menus carry
//! and show them; the focus sink hears the ones a menu doesn't (when nothing
//! is being typed). While the search field has the focus, the menus let the
//! arrows and plain keys through to it (`Actions::validateMenuItem:`).

use kilo_core::shortcuts::{self, Action, Key, Mods, SEEK_STEP, Shortcut, VOLUME_STEP};
use objc2::{ClassType, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSText};
use objc2_foundation::NSObjectProtocol;

use crate::app::{self, Route};

/// The key and modifiers of a key-down `event`.
fn key_of(event: &NSEvent) -> Option<(Key, Mods)> {
    let flags = event.modifierFlags();
    let c = event.charactersIgnoringModifiers()?.to_string().chars().next()?;
    let key = match c {
        ' ' => Key::Space,
        '\u{F702}' => Key::Left,
        '\u{F703}' => Key::Right,
        '\u{F700}' => Key::Up,
        '\u{F701}' => Key::Down,
        '\u{F72C}' => Key::PageUp,
        '\u{F72D}' => Key::PageDown,
        '\u{F729}' => Key::Home,
        '\u{F72B}' => Key::End,
        // Other function keys (F1, Delete, …) and control characters.
        c if c.is_control() || ('\u{F700}'..='\u{F8FF}').contains(&c) => return None,
        c => Key::Char(c),
    };
    let mods = Mods {
        primary: flags.contains(NSEventModifierFlags::Command),
        shift: flags.contains(NSEventModifierFlags::Shift),
        alt: flags.contains(NSEventModifierFlags::Option),
        control: flags.contains(NSEventModifierFlags::Control),
    };
    Some((key, mods))
}

/// A key pressed while nothing is being typed. Returns whether it was a
/// shortcut.
pub fn handle(event: &NSEvent) -> bool {
    let Some(action) = key_of(event).and_then(|(key, mods)| shortcuts::lookup(key, mods, false)) else { return false };
    // Holding a key repeats seeking, volume and scrolling, nothing else.
    let repeats = matches!(
        action,
        Action::SeekForward
            | Action::SeekBackward
            | Action::VolumeUp
            | Action::VolumeDown
            | Action::ScrollUp
            | Action::ScrollDown
            | Action::PageUp
            | Action::PageDown
    );
    if !event.isARepeat() || repeats {
        perform(action);
    }
    true
}

pub fn perform(action: Action) {
    let step = i16::from(VOLUME_STEP);
    match action {
        Action::PlayPause => app::play_pause(),
        Action::Next => app::next(),
        Action::Previous => app::previous(),
        Action::SeekForward => app::seek_by(SEEK_STEP),
        Action::SeekBackward => app::seek_by(-SEEK_STEP),
        Action::VolumeUp => app::change_volume(step),
        Action::VolumeDown => app::change_volume(-step),
        Action::Mute => app::toggle_mute(),
        Action::Search => app::focus_search(),
        Action::Back => app::back(),
        Action::Home => app::go(Route::Home),
        Action::Explore => app::go(Route::Explore),
        Action::Library => app::go(Route::Library),
        Action::Reload => app::reload(),
        Action::ScrollUp => app::scroll_by(-40.0, 0.0),
        Action::ScrollDown => app::scroll_by(40.0, 0.0),
        Action::PageUp => app::scroll_by(0.0, -1.0),
        Action::PageDown => app::scroll_by(0.0, 1.0),
        Action::Top => app::scroll_to(Some(0.0)),
        Action::Bottom => app::scroll_to(None),
        Action::ToggleSidebar => app::toggle_sidebar(),
    }
}

/// The action a menu item's tag stands for.
pub fn action(tag: isize) -> Option<Action> {
    Action::ALL.get(usize::try_from(tag).ok()?).copied()
}

pub fn tag(action: Action) -> isize {
    Action::ALL.iter().position(|&a| a == action).map_or(-1, |i| i as isize)
}

/// `shortcut` as a menu item's key equivalent and modifiers.
pub fn key_equivalent(shortcut: &Shortcut) -> (String, NSEventModifierFlags) {
    let key = match shortcut.key {
        Key::Char(c) => c.to_string(),
        Key::Space => " ".into(),
        Key::Left => "\u{F702}".into(),
        Key::Right => "\u{F703}".into(),
        Key::Up => "\u{F700}".into(),
        Key::Down => "\u{F701}".into(),
        Key::PageUp => "\u{F72C}".into(),
        Key::PageDown => "\u{F72D}".into(),
        Key::Home => "\u{F729}".into(),
        Key::End => "\u{F72B}".into(),
    };
    let mut mods = NSEventModifierFlags::empty();
    for (on, flag) in [
        (shortcut.mods.primary, NSEventModifierFlags::Command),
        (shortcut.mods.shift, NSEventModifierFlags::Shift),
        (shortcut.mods.alt, NSEventModifierFlags::Option),
        (shortcut.mods.control, NSEventModifierFlags::Control),
    ] {
        if on {
            mods |= flag;
        }
    }
    (key, mods)
}

/// Whether a key just pressed goes to a text field (the search field):
/// then only shortcuts that act while typing should take it.
pub fn typing_key(mtm: MainThreadMarker) -> bool {
    let app = NSApplication::sharedApplication(mtm);
    let key_down = app.currentEvent().is_some_and(|e| e.r#type() == NSEventType::KeyDown);
    // The field editor (an NSText) has the focus while a field is edited.
    key_down && app.keyWindow().and_then(|w| w.firstResponder()).is_some_and(|r| r.isKindOfClass(NSText::class()))
}
