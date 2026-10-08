//! Keyboard shortcuts: one table for every front end, so Kilo answers to the
//! same keys on macOS, Windows and Linux. `primary` is ⌘ on macOS and Ctrl
//! elsewhere; `alt` is ⌥ or Alt. A few follow each platform's own habit
//! instead (`Os`): the sidebar is ⌃⌘S on macOS, as in Finder and Mail, and
//! Ctrl+B elsewhere.
//!
//! While the search field has the focus, only `primary` shortcuts on
//! letters, digits and symbols act (⌘F, ⌘1, …): the arrows and plain keys
//! (Space, M, …) edit the text, so a space still types a space.
//!
//! They follow what people already know: Space, ⌘/Ctrl with the arrows, and
//! M from Spotify, YouTube and Apple Music; ⌘[ (macOS) or Alt+← (Windows,
//! Linux) to go back, as in browsers; ⌘F, ⌘L or ⌘K, or /, to search.

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
    pub primary: bool,
    pub shift: bool,
    pub alt: bool,
    /// The Control key on macOS (elsewhere, Ctrl is `primary`).
    pub control: bool,
}

/// Where a shortcut applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    All,
    Mac,
    /// Windows and Linux.
    Others,
}

impl Os {
    fn here(self) -> bool {
        match self {
            Os::All => true,
            Os::Mac => cfg!(target_os = "macos"),
            Os::Others => !cfg!(target_os = "macos"),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Shortcut {
    pub action: Action,
    pub key: Key,
    pub mods: Mods,
    pub os: Os,
}

impl Shortcut {
    /// Whether it acts while a text field has the focus.
    pub fn while_typing(&self) -> bool {
        self.mods.primary && matches!(self.key, Key::Char(_))
    }
}

/// How far the seek shortcuts move playback, in seconds.
pub const SEEK_STEP: f64 = 10.0;
/// How much the volume shortcuts change the volume, in percent.
pub const VOLUME_STEP: u8 = 10;

const PLAIN: Mods = Mods { primary: false, shift: false, alt: false, control: false };
const PRIMARY: Mods = Mods { primary: true, ..PLAIN };
const PRIMARY_SHIFT: Mods = Mods { primary: true, shift: true, ..PLAIN };
const PRIMARY_CONTROL: Mods = Mods { primary: true, control: true, ..PLAIN };
const ALT: Mods = Mods { alt: true, ..PLAIN };

const fn s(action: Action, key: Key, mods: Mods) -> Shortcut {
    Shortcut { action, key, mods, os: Os::All }
}

const fn only(os: Os, action: Action, key: Key, mods: Mods) -> Shortcut {
    Shortcut { action, key, mods, os }
}

/// Every shortcut. For each action, the first one listed is the one menus
/// show.
pub const SHORTCUTS: &[Shortcut] = &[
    s(Action::PlayPause, Key::Space, PLAIN),
    s(Action::Next, Key::Right, PRIMARY),
    s(Action::Previous, Key::Left, PRIMARY),
    s(Action::SeekForward, Key::Right, PRIMARY_SHIFT),
    s(Action::SeekForward, Key::Right, PLAIN),
    s(Action::SeekBackward, Key::Left, PRIMARY_SHIFT),
    s(Action::SeekBackward, Key::Left, PLAIN),
    s(Action::VolumeUp, Key::Up, PRIMARY),
    s(Action::VolumeDown, Key::Down, PRIMARY),
    s(Action::Mute, Key::Char('m'), PLAIN),
    s(Action::Search, Key::Char('f'), PRIMARY),
    s(Action::Search, Key::Char('l'), PRIMARY),
    s(Action::Search, Key::Char('k'), PRIMARY),
    s(Action::Search, Key::Char('/'), PLAIN),
    s(Action::Back, Key::Char('['), PRIMARY),
    s(Action::Back, Key::Left, ALT),
    s(Action::Home, Key::Char('1'), PRIMARY),
    s(Action::Explore, Key::Char('2'), PRIMARY),
    s(Action::Library, Key::Char('3'), PRIMARY),
    s(Action::Reload, Key::Char('r'), PRIMARY),
    s(Action::ScrollUp, Key::Up, PLAIN),
    s(Action::ScrollDown, Key::Down, PLAIN),
    s(Action::PageUp, Key::PageUp, PLAIN),
    s(Action::PageDown, Key::PageDown, PLAIN),
    s(Action::Top, Key::Home, PLAIN),
    s(Action::Bottom, Key::End, PLAIN),
    only(Os::Mac, Action::ToggleSidebar, Key::Char('s'), PRIMARY_CONTROL),
    only(Os::Others, Action::ToggleSidebar, Key::Char('b'), PRIMARY),
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
            let same = s.mods.primary == mods.primary && s.mods.alt == mods.alt && s.mods.control == mods.control;
            s.os.here() && s.key == key && same && shift_ok
        })
        .filter(|s| !typing || s.while_typing())
        .map(|s| s.action)
}

/// The shortcut menus show for `action` on this platform.
pub fn shown(action: Action) -> Option<&'static Shortcut> {
    SHORTCUTS.iter().find(|s| s.action == action && s.os.here())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_key_does_two_things() {
        for (i, a) in SHORTCUTS.iter().enumerate() {
            for b in &SHORTCUTS[i + 1..] {
                let same_os = a.os == Os::All || b.os == Os::All || a.os == b.os;
                assert!(!same_os || a.key != b.key || a.mods != b.mods, "{a:?} and {b:?} share a key");
            }
        }
    }

    #[test]
    fn every_action_has_a_key_everywhere() {
        for action in Action::ALL {
            for os in [Os::Mac, Os::Others] {
                assert!(SHORTCUTS.iter().any(|s| s.action == action && (s.os == Os::All || s.os == os)), "{action:?} on {os:?}");
            }
        }
    }

    #[test]
    fn the_sidebar_follows_each_platform() {
        let (key, mods) = if cfg!(target_os = "macos") { ('s', PRIMARY_CONTROL) } else { ('b', PRIMARY) };
        assert_eq!(lookup(Key::Char(key), mods, true), Some(Action::ToggleSidebar));
        assert_eq!(lookup(Key::Char('s'), PRIMARY, false), None);
    }

    #[test]
    fn plain_keys_wait_for_the_text_field_to_let_go() {
        assert_eq!(lookup(Key::Space, PLAIN, false), Some(Action::PlayPause));
        assert_eq!(lookup(Key::Space, PLAIN, true), None);
        assert_eq!(lookup(Key::Char('M'), Mods { shift: true, ..PLAIN }, false), Some(Action::Mute));
        assert_eq!(lookup(Key::Right, PRIMARY, false), Some(Action::Next));
        assert_eq!(lookup(Key::Right, PRIMARY, true), None);
        assert_eq!(lookup(Key::Char('f'), PRIMARY, true), Some(Action::Search));
        assert_eq!(lookup(Key::Right, PRIMARY_SHIFT, false), Some(Action::SeekForward));
        assert_eq!(lookup(Key::Left, ALT, false), Some(Action::Back));
        assert_eq!(lookup(Key::Char('x'), PRIMARY, false), None);
        assert_eq!(shown(Action::Search).map(|s| s.key), Some(Key::Char('f')));
    }
}
