//! The window, laid out like Spotify: one dark frame (light, in light mode)
//! holding the sidebar and the player bar, and pages on a rounded panel
//! with the back button, search and the account button on top. Plus the
//! menu bar. The frame used to be the desktop blurred behind the window,
//! which tinted the sidebar with the wallpaper's colors so it looked like a
//! separate part; a plain color also costs nothing (the blur was 0.2 MB).

use kilo_core::shortcuts::{self, Action};
use std::cell::Cell;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSButton, NSCellImagePosition, NSControlStateValueOff, NSControlStateValueOn, NSEventModifierFlags,
    NSImage, NSLayoutAttribute, NSLayoutConstraint, NSLineBreakMode, NSMenu, NSMenuItem, NSTextAlignment, NSTextField, NSView, NSWindow,
    NSWindowStyleMask, NSWindowTitleVisibility,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use super::bar::{Bar, Kind};
use super::{
    FocusSink, RootView, SEARCH_HEIGHT, SearchField, Weight, actions, circle_button, fill, font, image_view, label, search_field, size,
    stack, symbol_button, theme,
};
use crate::settings::{self, Appearance, Language};
use crate::strings::{S, t};

const SIDEBAR: f64 = 224.0;
/// Collapsed: only the icons, clear of the window's buttons (which end 69
/// points from the left).
const SIDEBAR_COLLAPSED: f64 = 76.0;
/// The sidebar's items start this far in, and are this wide collapsed (the
/// icons stay put when it collapses).
const NAV_INSET: f64 = 16.0;
const ITEM: f64 = 44.0;
const TOP_BAR: f64 = 64.0;
const PLAYER_BAR: f64 = 88.0;
/// Gap between the panel and the window's edges.
const GAP: f64 = 8.0;
const PANEL_RADIUS: f64 = 12.0;
/// The back and account buttons.
const ROUND: f64 = 36.0;

/// A sidebar section: its pill (lit when selected) and its button.
pub type NavPill = (Retained<NSView>, Retained<NSButton>);

/// The sidebar: what changes when it collapses, and the sections.
pub struct Sidebar {
    width: Retained<NSLayoutConstraint>,
    trailing: Retained<NSLayoutConstraint>,
    brand: Retained<NSTextField>,
    toggle: Retained<NSButton>,
    pub nav: Vec<NavPill>,
    /// 0 expanded, 1 collapsed, in between while it animates.
    progress: Cell<f64>,
}

pub struct Shell {
    pub window: Retained<NSWindow>,
    pub focus_sink: Retained<FocusSink>,
    /// Where pages go.
    pub content: Retained<NSView>,
    pub back: Retained<NSButton>,
    pub search: Retained<SearchField>,
    /// The account button's picture, and the button over it (showing an
    /// icon until the picture arrives).
    pub avatar: Retained<NSView>,
    pub avatar_button: Retained<NSButton>,
    pub sidebar: Sidebar,
    pub bar: PlayerBar,
}

pub struct PlayerBar {
    pub art: Retained<NSView>,
    pub title: Retained<NSTextField>,
    pub artist: Retained<NSTextField>,
    pub play: Retained<NSButton>,
    pub elapsed: Retained<NSTextField>,
    pub total: Retained<NSTextField>,
    pub progress: Retained<Bar>,
    /// Mutes; its icon shows the volume.
    pub speaker: Retained<NSButton>,
    pub volume: Retained<Bar>,
}

/// Builds the window's views, in `window` if given (rebuilding it for a new
/// look or language) or in a new window.
pub fn build(window: Option<Retained<NSWindow>>, mtm: MainThreadMarker) -> Shell {
    let p = theme::palette();
    let window = window.unwrap_or_else(|| new_window(mtm));
    // The frame is the window's own background: no layer to draw.
    window.setBackgroundColor(Some(&p.frame));

    let root = RootView::new(mtm);
    window.setContentView(Some(&root));
    let focus_sink = FocusSink::new(mtm);
    root.addSubview(&focus_sink);
    window.setInitialFirstResponder(Some(&focus_sink));

    let (sidebar_view, sidebar) = sidebar(mtm);
    let panel = NSView::new(mtm);
    fill(&panel, Some(&p.panel), PANEL_RADIUS);
    let (bar_view, bar) = player_bar(mtm);
    for v in [&*sidebar_view, &*panel, &*bar_view] {
        root.addSubview(v);
        v.setTranslatesAutoresizingMaskIntoConstraints(false);
    }
    let c = |a: Retained<NSLayoutConstraint>| a.setActive(true);
    c(bar_view.leadingAnchor().constraintEqualToAnchor(&root.leadingAnchor()));
    c(bar_view.trailingAnchor().constraintEqualToAnchor(&root.trailingAnchor()));
    c(bar_view.bottomAnchor().constraintEqualToAnchor(&root.bottomAnchor()));
    c(bar_view.heightAnchor().constraintEqualToConstant(PLAYER_BAR));
    c(sidebar_view.topAnchor().constraintEqualToAnchor(&root.topAnchor()));
    c(sidebar_view.leadingAnchor().constraintEqualToAnchor(&root.leadingAnchor()));
    c(sidebar_view.bottomAnchor().constraintEqualToAnchor(&bar_view.topAnchor()));
    c(panel.topAnchor().constraintEqualToAnchor_constant(&root.topAnchor(), GAP));
    c(panel.leadingAnchor().constraintEqualToAnchor(&sidebar_view.trailingAnchor()));
    c(panel.trailingAnchor().constraintEqualToAnchor_constant(&root.trailingAnchor(), -GAP));
    c(panel.bottomAnchor().constraintEqualToAnchor(&bar_view.topAnchor()));

    // Top of the panel: back, search, account.
    let back = circle_button("chevron.left", sel!(back:), ROUND, 14.0, false, mtm);
    size(&back, Some(ROUND), Some(ROUND));
    back.setToolTip(Some(&NSString::from_str(t(S::Back))));
    let (search_pill, search) = search_field(mtm);
    size(&search_pill, Some(400.0), Some(SEARCH_HEIGHT));
    let avatar = image_view(ROUND, ROUND, ROUND / 2.0, mtm);
    if let Some(layer) = avatar.layer() {
        layer.setBackgroundColor(Some(&p.raised.CGColor()));
    }
    let avatar_icon = symbol_button("person.fill", sel!(account:), 15.0, mtm);
    avatar_icon.setContentTintColor(Some(&p.text_dim));
    avatar_icon.setToolTip(Some(&NSString::from_str(t(S::Account))));
    // The whole circle is the button.
    avatar.addSubview(&avatar_icon);
    super::pin(&avatar_icon, &avatar, 0.0, 0.0, 0.0, 0.0);
    let content = NSView::new(mtm);
    for v in [&*back as &NSView, &search_pill, &avatar, &content] {
        panel.addSubview(v);
        v.setTranslatesAutoresizingMaskIntoConstraints(false);
    }
    let middle = TOP_BAR / 2.0;
    c(back.leadingAnchor().constraintEqualToAnchor_constant(&panel.leadingAnchor(), 16.0));
    c(back.centerYAnchor().constraintEqualToAnchor_constant(&panel.topAnchor(), middle));
    c(search_pill.leadingAnchor().constraintEqualToAnchor_constant(&back.trailingAnchor(), 12.0));
    c(search_pill.centerYAnchor().constraintEqualToAnchor_constant(&panel.topAnchor(), middle));
    c(avatar.trailingAnchor().constraintEqualToAnchor_constant(&panel.trailingAnchor(), -16.0));
    c(avatar.centerYAnchor().constraintEqualToAnchor_constant(&panel.topAnchor(), middle));
    // The page sits clear of the panel's rounded corners: nothing needs
    // clipping (a clipped panel measured +0.4 MB).
    c(content.topAnchor().constraintEqualToAnchor_constant(&panel.topAnchor(), TOP_BAR));
    c(content.leadingAnchor().constraintEqualToAnchor(&panel.leadingAnchor()));
    c(content.trailingAnchor().constraintEqualToAnchor(&panel.trailingAnchor()));
    c(content.bottomAnchor().constraintEqualToAnchor_constant(&panel.bottomAnchor(), -PANEL_RADIUS));

    let shell = Shell { window, focus_sink, content, back, search, avatar, avatar_button: avatar_icon, sidebar, bar };
    set_collapsed(&shell, settings::sidebar_collapsed());
    shell
}

fn new_window(mtm: MainThreadMarker) -> Retained<NSWindow> {
    // SAFETY: standard NSWindow initializer with valid arguments.
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1240.0, 820.0)),
            NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable
                | NSWindowStyleMask::Resizable
                | NSWindowStyleMask::FullSizeContentView,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    // SAFETY: plain property setter.
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&NSString::from_str("Kilo"));
    window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
    window.setTitlebarAppearsTransparent(true);
    window.setMinSize(NSSize::new(900.0, 600.0));
    window.center();
    window.setFrameAutosaveName(&NSString::from_str("KiloMain"));
    window
}

/// The sidebar: the name, then the sections as pills.
fn sidebar(mtm: MainThreadMarker) -> (Retained<NSView>, Sidebar) {
    let sidebar = NSView::new(mtm);
    let toggle = symbol_button("line.3.horizontal", sel!(toggleSidebar:), 17.0, mtm);
    size(&toggle, Some(ITEM), Some(40.0));
    let brand = label("Kilo", 24.0, Weight::Bold, false, mtm);
    let head = stack(&[&toggle, &brand], false, 4.0, mtm);
    head.setAlignment(NSLayoutAttribute::CenterY);
    let nav: Vec<NavPill> = [
        (S::Home, "house.fill", sel!(home:)),
        (S::Explore, "safari.fill", sel!(explore:)),
        (S::Library, "books.vertical.fill", sel!(library:)),
    ]
    .into_iter()
    .map(|(title, symbol, action)| nav_pill(t(title), symbol, action, mtm))
    .collect();
    let mut items: Vec<&NSView> = vec![&head];
    items.extend(nav.iter().map(|(pill, _)| &**pill));
    let column = stack(&items, true, 4.0, mtm);
    column.setAlignment(NSLayoutAttribute::Leading);
    column.setCustomSpacing_afterView(14.0, &head);
    sidebar.addSubview(&column);
    column.setTranslatesAutoresizingMaskIntoConstraints(false);
    column.topAnchor().constraintEqualToAnchor_constant(&sidebar.topAnchor(), 40.0).setActive(true);
    column.leadingAnchor().constraintEqualToAnchor_constant(&sidebar.leadingAnchor(), NAV_INSET).setActive(true);
    let trailing = column.trailingAnchor().constraintEqualToAnchor_constant(&sidebar.trailingAnchor(), -12.0);
    trailing.setActive(true);
    for (pill, _) in &nav {
        pill.widthAnchor().constraintEqualToAnchor(&column.widthAnchor()).setActive(true);
    }
    let width = sidebar.widthAnchor().constraintEqualToConstant(SIDEBAR);
    width.setActive(true);
    (sidebar, Sidebar { width, trailing, brand, toggle, nav, progress: Cell::new(0.0) })
}

/// Collapses the sidebar to its icons (the ☰ button and the sections, with
/// their names as tooltips), or expands it. The icons stay where they are.
pub fn set_collapsed(shell: &Shell, collapsed: bool) {
    let s = &shell.sidebar;
    set_width(s, if collapsed { 1.0 } else { 0.0 });
    s.brand.setHidden(collapsed);
    s.toggle.setToolTip(Some(&NSString::from_str(t(if collapsed { S::ExpandSidebar } else { S::CollapseSidebar }))));
    for (_, button) in &s.nav {
        button.setImagePosition(if collapsed { NSCellImagePosition::ImageOnly } else { NSCellImagePosition::ImageLeading });
        button.setToolTip(collapsed.then(|| button.title()).as_deref());
    }
}

/// How collapsed the sidebar is: 0 expanded, 1 collapsed.
pub fn sidebar_progress(shell: &Shell) -> f64 {
    shell.sidebar.progress.get()
}

/// One frame of the sidebar collapsing or expanding (`progress` from 0,
/// expanded, to 1). The names are cut off as the pills narrow and the page
/// slides over them; the "Kilo" name fades. `set_collapsed` ends it.
pub fn set_sidebar_progress(shell: &Shell, progress: f64) {
    let s = &shell.sidebar;
    set_width(s, progress);
    s.brand.setHidden(false);
    s.brand.setAlphaValue((1.0 - 2.0 * progress).max(0.0));
    for (_, button) in &s.nav {
        if button.imagePosition() != NSCellImagePosition::ImageLeading {
            button.setImagePosition(NSCellImagePosition::ImageLeading);
            button.setToolTip(None);
        }
    }
}

fn set_width(s: &Sidebar, progress: f64) {
    let between = |expanded: f64, collapsed: f64| expanded + (collapsed - expanded) * progress;
    s.progress.set(progress);
    s.width.setConstant(between(SIDEBAR, SIDEBAR_COLLAPSED));
    s.trailing.setConstant(between(-12.0, NAV_INSET + ITEM - SIDEBAR_COLLAPSED));
    if progress == 0.0 {
        s.brand.setAlphaValue(1.0);
    }
}

/// A sidebar section: icon and title in a pill that lights up when selected.
fn nav_pill(title: &str, symbol: &str, action: Sel, mtm: MainThreadMarker) -> NavPill {
    let p = theme::palette();
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(symbol), None).unwrap_or_default();
    let target: &AnyObject = actions(mtm);
    // SAFETY: the action target lives for the whole app.
    let button =
        unsafe { NSButton::buttonWithTitle_image_target_action(&NSString::from_str(title), &image, Some(target), Some(action), mtm) };
    button.setBordered(false);
    button.setImagePosition(NSCellImagePosition::ImageLeading);
    button.setAlignment(NSTextAlignment::Left);
    button.setFont(Some(&font(14.0, Weight::Bold)));
    // While the sidebar narrows, the name is cut off, not shortened to "…".
    button.setLineBreakMode(NSLineBreakMode::ByClipping);
    button.setContentTintColor(Some(&p.text_dim));
    let pill = NSView::new(mtm);
    fill(&pill, None, 20.0);
    pill.addSubview(&button);
    super::pin(&button, &pill, 0.0, 12.0, 0.0, 12.0);
    size(&pill, None, Some(40.0));
    (pill, button)
}

/// Lights up the sidebar pill for the current section (or none); the
/// others' words dim, as in Spotify.
pub fn select_nav(shell: &Shell, index: Option<usize>) {
    let p = theme::palette();
    for (i, (pill, button)) in shell.sidebar.nav.iter().enumerate() {
        let on = Some(i) == index;
        if let Some(layer) = pill.layer() {
            layer.setBackgroundColor(on.then(|| p.selected.CGColor()).as_deref());
        }
        button.setContentTintColor(Some(if on { &p.text } else { &p.text_dim }));
    }
}

fn player_bar(mtm: MainThreadMarker) -> (Retained<NSView>, PlayerBar) {
    let p = theme::palette();
    let root = NSView::new(mtm);

    // Left: what's playing.
    let art = image_view(56.0, 56.0, 6.0, mtm);
    let title = label(t(S::NothingPlaying), 14.0, Weight::Semibold, false, mtm);
    let artist = label("", 12.0, Weight::Regular, true, mtm);
    let text = stack(&[&title, &artist], true, 3.0, mtm);
    text.setAlignment(NSLayoutAttribute::Leading);
    let now = stack(&[&art, &text], false, 12.0, mtm);
    now.setAlignment(NSLayoutAttribute::CenterY);

    // Middle: the controls over the progress bar.
    let prev = symbol_button("backward.fill", sel!(previous:), 16.0, mtm);
    prev.setToolTip(Some(&NSString::from_str(t(S::Previous))));
    let play = circle_button("play.fill", sel!(playPause:), 36.0, 15.0, true, mtm);
    size(&play, Some(36.0), Some(36.0));
    play.setToolTip(Some(&NSString::from_str(t(S::PlayPause))));
    let next = symbol_button("forward.fill", sel!(next:), 16.0, mtm);
    next.setToolTip(Some(&NSString::from_str(t(S::Next))));
    let controls = stack(&[&prev, &play, &next], false, 22.0, mtm);
    controls.setAlignment(NSLayoutAttribute::CenterY);
    let time = |s: &str| {
        let l = label(s, 11.0, Weight::Regular, true, mtm);
        size(&l, Some(40.0), None);
        l
    };
    let elapsed = time("");
    elapsed.setAlignment(NSTextAlignment::Right);
    let total = time("");
    let progress = Bar::new(Kind::Progress, 4.0, &p.text, mtm);
    size(&progress, None, Some(12.0));
    let timeline = stack(&[&elapsed, &progress, &total], false, 8.0, mtm);
    timeline.setAlignment(NSLayoutAttribute::CenterY);

    // Right: volume.
    let speaker = symbol_button(speaker_symbol(100), sel!(mute:), 13.0, mtm);
    speaker.setContentTintColor(Some(&p.text_dim));
    speaker.setToolTip(Some(&NSString::from_str(t(S::Mute))));
    // The same width at every volume, so the slider doesn't move.
    size(&speaker, Some(22.0), None);
    let volume = Bar::new(Kind::Volume, 4.0, &p.text, mtm);
    size(&volume, Some(100.0), Some(12.0));
    let right = stack(&[&speaker, &volume], false, 6.0, mtm);
    right.setAlignment(NSLayoutAttribute::CenterY);

    for v in [&*now as &NSView, &controls, &timeline, &right] {
        root.addSubview(v);
        v.setTranslatesAutoresizingMaskIntoConstraints(false);
    }
    let c = |a: Retained<NSLayoutConstraint>| a.setActive(true);
    c(now.leadingAnchor().constraintEqualToAnchor_constant(&root.leadingAnchor(), 16.0));
    c(now.centerYAnchor().constraintEqualToAnchor(&root.centerYAnchor()));
    c(controls.centerXAnchor().constraintEqualToAnchor(&root.centerXAnchor()));
    c(controls.topAnchor().constraintEqualToAnchor_constant(&root.topAnchor(), 12.0));
    c(timeline.centerXAnchor().constraintEqualToAnchor(&root.centerXAnchor()));
    c(timeline.topAnchor().constraintEqualToAnchor_constant(&controls.bottomAnchor(), 6.0));
    c(timeline.widthAnchor().constraintEqualToAnchor_multiplier(&root.widthAnchor(), 0.4));
    c(right.trailingAnchor().constraintEqualToAnchor_constant(&root.trailingAnchor(), -20.0));
    c(right.centerYAnchor().constraintEqualToAnchor(&root.centerYAnchor()));
    // The title gives way to the controls.
    c(now.trailingAnchor().constraintLessThanOrEqualToAnchor_constant(&timeline.leadingAnchor(), -16.0));

    (root, PlayerBar { art, title, artist, play, elapsed, total, progress, speaker, volume })
}

/// The speaker icon for `volume` (0–100).
pub fn speaker_symbol(volume: u8) -> &'static str {
    match volume {
        0 => "speaker.slash.fill",
        1..=33 => "speaker.wave.1.fill",
        34..=66 => "speaker.wave.2.fill",
        _ => "speaker.wave.3.fill",
    }
}

/// The menu bar: app (with sign in or out), edit (so copy and paste work in
/// the search field), view (sections, appearance, language), playback and
/// window. Shortcuts come from `kilo_core::shortcuts`.
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
