//! The menu bar and the account button's menu. Their keyboard shortcuts come
//! from `shortcuts`.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{NSApplication, NSControlStateValueOff, NSControlStateValueOn, NSEventModifierFlags, NSMenu, NSMenuItem};
use objc2_foundation::NSString;

use super::actions;
use crate::settings::{self, Appearance, Language};
use crate::shortcuts::{self, Action};
use crate::strings::{S, t};

/// The menu bar: app (with sign in or out), edit (so copy and paste work in
/// the search field), view (sections, appearance, language), playback and
/// window. Shortcuts come from `shortcuts`.
pub fn install_menu(signed_in: bool, mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    let bar = NSMenu::new(mtm);
    let command_option = Some(NSEventModifierFlags::Command | NSEventModifierFlags::Option);

    let app_menu = menu("Kilo", mtm);
    add(&app_menu, t(S::AboutKilo), Some(sel!(orderFrontStandardAboutPanel:)), "", None, false, mtm);
    app_menu.addItem(&NSMenuItem::separatorItem(mtm));
    let (account, action) = if signed_in { (S::SignOut, sel!(signOut:)) } else { (S::SignIn, sel!(signIn:)) };
    add(&app_menu, t(account), Some(action), "", None, true, mtm);
    app_menu.addItem(&NSMenuItem::separatorItem(mtm));
    add(&app_menu, t(S::HideKilo), Some(sel!(hide:)), "h", None, false, mtm);
    add(&app_menu, t(S::HideOthers), Some(sel!(hideOtherApplications:)), "h", command_option, false, mtm);
    add(&app_menu, t(S::ShowAll), Some(sel!(unhideAllApplications:)), "", None, false, mtm);
    app_menu.addItem(&NSMenuItem::separatorItem(mtm));
    add(&app_menu, t(S::QuitKilo), Some(sel!(terminate:)), "q", None, false, mtm);

    let edit = menu(t(S::Edit), mtm);
    for (title, action, key) in [
        (S::Undo, sel!(undo:), "z"),
        (S::Redo, sel!(redo:), "Z"),
        (S::Cut, sel!(cut:), "x"),
        (S::Copy, sel!(copy:), "c"),
        (S::Paste, sel!(paste:), "v"),
        (S::SelectAll, sel!(selectAll:), "a"),
    ] {
        add(&edit, t(title), Some(action), key, None, false, mtm);
    }
    edit.addItem(&NSMenuItem::separatorItem(mtm));
    shortcut(&edit, S::Search, Action::Search, mtm);

    let view = menu(t(S::View), mtm);
    // Its title says which way it goes (`validateMenuItem:`).
    shortcut(&view, S::CollapseSidebar, Action::ToggleSidebar, mtm);
    view.addItem(&NSMenuItem::separatorItem(mtm));
    shortcut(&view, S::Home, Action::Home, mtm);
    shortcut(&view, S::Explore, Action::Explore, mtm);
    shortcut(&view, S::Library, Action::Library, mtm);
    view.addItem(&NSMenuItem::separatorItem(mtm));
    shortcut(&view, S::Back, Action::Back, mtm);
    shortcut(&view, S::Reload, Action::Reload, mtm);
    view.addItem(&NSMenuItem::separatorItem(mtm));
    add_settings(&view, mtm);

    let playback = menu(t(S::Playback), mtm);
    for (i, (title, action)) in [
        (S::PlayPause, Action::PlayPause),
        (S::Next, Action::Next),
        (S::Previous, Action::Previous),
        (S::SeekForward, Action::SeekForward),
        (S::SeekBackward, Action::SeekBackward),
        (S::VolumeUp, Action::VolumeUp),
        (S::VolumeDown, Action::VolumeDown),
        (S::Mute, Action::Mute),
    ]
    .into_iter()
    .enumerate()
    {
        if matches!(i, 3 | 5) {
            playback.addItem(&NSMenuItem::separatorItem(mtm));
        }
        shortcut(&playback, title, action, mtm);
    }

    let window = menu(t(S::Window), mtm);
    add(&window, t(S::Minimize), Some(sel!(performMiniaturize:)), "m", None, false, mtm);
    add(&window, t(S::Close), Some(sel!(performClose:)), "w", None, false, mtm);

    for m in [&app_menu, &edit, &view, &playback, &window] {
        let holder = NSMenuItem::new(mtm);
        holder.setSubmenu(Some(m));
        bar.addItem(&holder);
    }
    app.setMainMenu(Some(&bar));
    app.setWindowsMenu(Some(&window));
}

/// A menu item for a keyboard shortcut, showing its key.
fn shortcut(m: &NSMenu, title: S, action: Action, mtm: MainThreadMarker) {
    let (key, mods) = shortcuts::shown(action).map_or_else(|| (String::new(), NSEventModifierFlags::empty()), crate::keys::key_equivalent);
    let item = add(m, t(title), Some(sel!(shortcut:)), &key, Some(mods), true, mtm);
    item.setTag(crate::keys::tag(action));
}

/// The account button's menu: who's signed in, the settings, and signing in
/// or out.
pub fn account_menu(signed_in: bool, account: Option<(&str, &str)>, mtm: MainThreadMarker) -> Retained<NSMenu> {
    let m = menu("", mtm);
    match (signed_in, account) {
        (true, Some((name, handle))) => {
            for line in [name, handle].into_iter().filter(|l| !l.is_empty()) {
                add(&m, line, None, "", None, false, mtm).setEnabled(false);
            }
        }
        // Signed in, but the name hasn't arrived yet: no header.
        (true, None) => {}
        (false, _) => {
            add(&m, t(S::NotSignedIn), None, "", None, false, mtm).setEnabled(false);
        }
    }
    m.addItem(&NSMenuItem::separatorItem(mtm));
    add_settings(&m, mtm);
    m.addItem(&NSMenuItem::separatorItem(mtm));
    let (title, action) = if signed_in { (S::SignOut, sel!(signOut:)) } else { (S::SignIn, sel!(signIn:)) };
    add(&m, t(title), Some(action), "", None, true, mtm);
    m
}

/// The Appearance and Language submenus, with the current choices ticked.
fn add_settings(m: &NSMenu, mtm: MainThreadMarker) {
    let appearance = settings::appearance();
    let sub = menu(t(S::Appearance), mtm);
    for (i, (title, value)) in
        [(S::System, Appearance::System), (S::Light, Appearance::Light), (S::Dark, Appearance::Dark)].into_iter().enumerate()
    {
        choice(&sub, t(title), sel!(setAppearance:), i, value == appearance, mtm);
    }
    submenu(m, &sub, mtm);
    let language = settings::language();
    let sub = menu(t(S::Language), mtm);
    for (i, (title, value)) in
        [(t(S::System), Language::System), ("English", Language::English), ("Türkçe", Language::Turkish)].into_iter().enumerate()
    {
        choice(&sub, title, sel!(setLanguage:), i, value == language, mtm);
    }
    submenu(m, &sub, mtm);
}

fn choice(m: &NSMenu, title: &str, action: Sel, tag: usize, on: bool, mtm: MainThreadMarker) {
    let item = add(m, title, Some(action), "", None, true, mtm);
    item.setTag(tag as isize);
    item.setState(if on { NSControlStateValueOn } else { NSControlStateValueOff });
}

fn submenu(m: &NSMenu, sub: &NSMenu, mtm: MainThreadMarker) {
    let holder = add(m, &sub.title().to_string(), None, "", None, false, mtm);
    holder.setSubmenu(Some(sub));
}

fn menu(title: &str, mtm: MainThreadMarker) -> Retained<NSMenu> {
    NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title))
}

/// Adds an item; `ours` sends its action to Kilo's action target (others go
/// up the responder chain).
fn add(
    menu: &NSMenu,
    title: &str,
    action: Option<Sel>,
    key: &str,
    modifiers: Option<NSEventModifierFlags>,
    ours: bool,
    mtm: MainThreadMarker,
) -> Retained<NSMenuItem> {
    // SAFETY: standard NSMenuItem initializer.
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &NSString::from_str(title), action, &NSString::from_str(key))
    };
    if let Some(m) = modifiers {
        item.setKeyEquivalentModifierMask(m);
    }
    if ours {
        let target: &AnyObject = actions(mtm);
        // SAFETY: the target lives for the whole app.
        unsafe { item.setTarget(Some(target)) };
    }
    menu.addItem(&item);
    item
}
