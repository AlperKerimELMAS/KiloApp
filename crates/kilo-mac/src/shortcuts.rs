//! Keyboard shortcuts: one table, which the menus show and every key press
//! is looked up in (`keys.rs` is AppKit's side of it).
//!
//! While the search field has the focus, only ⌘ shortcuts on letters,
//! digits and symbols act (⌘F, ⌘1, …): the arrows and plain keys (Space,
//! M, …) edit the text, so a space still types a space.
//!
//! They follow what people already know: Space, ⌘ with the arrows, and M
//! from Spotify, YouTube and Apple Music; ⌘[ or ⌥← to go back, as in
//! browsers; ⌘F, ⌘L or ⌘K, or /, to search; ⌃⌘S for the sidebar, as in
//! Finder and Mail.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    PlayPause,
    Next,
    Previous,
    SeekForward,
    SeekBackward,
    VolumeUp,
    VolumeDown,
    Mute,
    Search,
    Back,
    Home,
    Explore,
    Library,
    Reload,
    ScrollUp,
    ScrollDown,
    PageUp,
    PageDown,
    Top,
    Bottom,
    /// Collapses the sidebar to its icons, or expands it.
    ToggleSidebar,
}

impl Action {
    pub const ALL: [Action; 21] = [
        Action::PlayPause,
        Action::Next,
        Action::Previous,
        Action::SeekForward,
        Action::SeekBackward,
        Action::VolumeUp,
        Action::VolumeDown,
        Action::Mute,
        Action::Search,
        Action::Back,
        Action::Home,
        Action::Explore,
        Action::Library,
        Action::Reload,
        Action::ScrollUp,
        Action::ScrollDown,
        Action::PageUp,
        Action::PageDown,
        Action::Top,
        Action::Bottom,
        Action::ToggleSidebar,
    ];
}

/// A key, independent of the keyboard layout for letters and symbols.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// A letter (lowercase), digit or symbol.
    Char(char),
    Space,
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub command: bool,
    pub shift: bool,
    pub option: bool,
    pub control: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Shortcut {
    pub action: Action,
    pub key: Key,
    pub mods: Mods,
}

impl Shortcut {
    /// Whether it acts while a text field has the focus.
    pub fn while_typing(&self) -> bool {
        self.mods.command && matches!(self.key, Key::Char(_))
    }
}

/// How far the seek shortcuts move playback, in seconds.
pub const SEEK_STEP: f64 = 10.0;
/// How much the volume shortcuts change the volume, in percent.
pub const VOLUME_STEP: u8 = 10;

const PLAIN: Mods = Mods { command: false, shift: false, option: false, control: false };
const COMMAND: Mods = Mods { command: true, ..PLAIN };
const COMMAND_SHIFT: Mods = Mods { command: true, shift: true, ..PLAIN };
const CONTROL_COMMAND: Mods = Mods { command: true, control: true, ..PLAIN };
const OPTION: Mods = Mods { option: true, ..PLAIN };

const fn s(action: Action, key: Key, mods: Mods) -> Shortcut {
    Shortcut { action, key, mods }
}

/// Every shortcut. For each action, the first one listed is the one menus
/// show.
pub const SHORTCUTS: &[Shortcut] = &[
    s(Action::PlayPause, Key::Space, PLAIN),
    s(Action::Next, Key::Right, COMMAND),
    s(Action::Previous, Key::Left, COMMAND),
    s(Action::SeekForward, Key::Right, COMMAND_SHIFT),
    s(Action::SeekForward, Key::Right, PLAIN),
    s(Action::SeekBackward, Key::Left, COMMAND_SHIFT),
    s(Action::SeekBackward, Key::Left, PLAIN),
    s(Action::VolumeUp, Key::Up, COMMAND),
    s(Action::VolumeDown, Key::Down, COMMAND),
    s(Action::Mute, Key::Char('m'), PLAIN),
    s(Action::Search, Key::Char('f'), COMMAND),
    s(Action::Search, Key::Char('l'), COMMAND),
    s(Action::Search, Key::Char('k'), COMMAND),
    s(Action::Search, Key::Char('/'), PLAIN),
    s(Action::Back, Key::Char('['), COMMAND),
    s(Action::Back, Key::Left, OPTION),
    s(Action::Home, Key::Char('1'), COMMAND),
    s(Action::Explore, Key::Char('2'), COMMAND),
    s(Action::Library, Key::Char('3'), COMMAND),
    s(Action::Reload, Key::Char('r'), COMMAND),
    s(Action::ScrollUp, Key::Up, PLAIN),
    s(Action::ScrollDown, Key::Down, PLAIN),
    s(Action::PageUp, Key::PageUp, PLAIN),
    s(Action::PageDown, Key::PageDown, PLAIN),
    s(Action::Top, Key::Home, PLAIN),
    s(Action::Bottom, Key::End, PLAIN),
    s(Action::ToggleSidebar, Key::Char('s'), CONTROL_COMMAND),
];

/// The action for `key` pressed with `mods`, while a text field has the
/// focus (`typing`) or not. Letters and symbols ignore Shift: the layout
/// may need it to type them.
pub fn lookup(key: Key, mods: Mods, typing: bool) -> Option<Action> {
    let key = match key {
        Key::Char(c) => Key::Char(c.to_ascii_lowercase()),
        other => other,
    };
    SHORTCUTS
        .iter()
        .find(|s| {
            let shift_ok = matches!(key, Key::Char(_)) || s.mods.shift == mods.shift;
            let same = s.mods.command == mods.command && s.mods.option == mods.option && s.mods.control == mods.control;
            s.key == key && same && shift_ok
        })
        .filter(|s| !typing || s.while_typing())
        .map(|s| s.action)
}

/// The shortcut menus show for `action`.
pub fn shown(action: Action) -> Option<&'static Shortcut> {
    SHORTCUTS.iter().find(|s| s.action == action)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_key_does_two_things() {
        for (i, a) in SHORTCUTS.iter().enumerate() {
            for b in &SHORTCUTS[i + 1..] {
                assert!(a.key != b.key || a.mods != b.mods, "{a:?} and {b:?} share a key");
            }
        }
    }

    #[test]
    fn every_action_has_a_key() {
        for action in Action::ALL {
            assert!(shown(action).is_some(), "{action:?}");
        }
    }

    #[test]
    fn the_sidebar_is_control_command_s() {
        assert_eq!(lookup(Key::Char('s'), CONTROL_COMMAND, true), Some(Action::ToggleSidebar));
        assert_eq!(lookup(Key::Char('s'), COMMAND, false), None);
    }

    #[test]
    fn plain_keys_wait_for_the_text_field_to_let_go() {
        assert_eq!(lookup(Key::Space, PLAIN, false), Some(Action::PlayPause));
        assert_eq!(lookup(Key::Space, PLAIN, true), None);
        assert_eq!(lookup(Key::Char('M'), Mods { shift: true, ..PLAIN }, false), Some(Action::Mute));
        assert_eq!(lookup(Key::Right, COMMAND, false), Some(Action::Next));
        assert_eq!(lookup(Key::Right, COMMAND, true), None);
        assert_eq!(lookup(Key::Char('f'), COMMAND, true), Some(Action::Search));
        assert_eq!(lookup(Key::Right, COMMAND_SHIFT, false), Some(Action::SeekForward));
        assert_eq!(lookup(Key::Left, OPTION, false), Some(Action::Back));
        assert_eq!(lookup(Key::Char('x'), COMMAND, false), None);
        assert_eq!(shown(Action::Search).map(|s| s.key), Some(Key::Char('f')));
    }
}
