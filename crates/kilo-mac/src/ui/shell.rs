//! The window frame: sidebar, search bar, page area and player bar, laid out
//! like YouTube Music, plus the menu bar.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameDarkAqua, NSApplication, NSBackingStoreType, NSButton, NSCellImagePosition,
    NSEventModifierFlags, NSImage, NSMenu, NSMenuItem, NSSearchField, NSTextAlignment, NSTextField, NSView, NSWindow, NSWindowStyleMask,
    NSWindowTitleVisibility,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use super::bar::{Bar, Kind};
use super::{Weight, accent, actions, bar_bg, bg, font, image_view, label, search_field, sidebar_bg, size, stack, symbol_button, text};

pub const SIDEBAR: f64 = 232.0;
const TOP_BAR: f64 = 60.0;
const PLAYER_BAR: f64 = 76.0;

pub struct Shell {
    pub window: Retained<NSWindow>,
    pub focus_sink: Retained<super::FocusSink>,
    pub content: Retained<NSView>,
    pub back: Retained<NSButton>,
    pub search: Retained<NSSearchField>,
    pub nav: Vec<Retained<NSButton>>,
    pub bar: PlayerBar,
}

pub struct PlayerBar {
    pub art: Retained<NSView>,
    pub title: Retained<NSTextField>,
    pub artist: Retained<NSTextField>,
    pub play: Retained<NSButton>,
    pub time: Retained<NSTextField>,
    pub progress: Retained<Bar>,
    pub volume: Retained<Bar>,
}

pub fn build(mtm: MainThreadMarker) -> Shell {
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
    // SAFETY: plain property setters.
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&NSString::from_str("Kilo"));
    window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
    window.setTitlebarAppearsTransparent(true);
    // SAFETY: NSAppearanceNameDarkAqua is a constant string.
    window.setAppearance(NSAppearance::appearanceNamed(unsafe { NSAppearanceNameDarkAqua }).as_deref());
    window.setBackgroundColor(Some(&bg()));
    window.setMinSize(NSSize::new(900.0, 600.0));
    window.center();
    window.setFrameAutosaveName(&NSString::from_str("KiloMain"));

    let root = NSView::new(mtm);
    root.setWantsLayer(true);
    if let Some(layer) = root.layer() {
        layer.setBackgroundColor(Some(&bg().CGColor()));
    }
    window.setContentView(Some(&root));
    let focus_sink = super::FocusSink::new(mtm);
    root.addSubview(&focus_sink);
    window.setInitialFirstResponder(Some(&focus_sink));

    // Sidebar.
    let sidebar = NSView::new(mtm);
    sidebar.setWantsLayer(true);
    if let Some(layer) = sidebar.layer() {
        layer.setBackgroundColor(Some(&sidebar_bg().CGColor()));
    }
    let brand = label("Kilo", 22.0, Weight::Bold, false, mtm);
    let nav: Vec<Retained<NSButton>> =
        [("Home", "house.fill", sel!(home:)), ("Explore", "safari", sel!(explore:)), ("Library", "books.vertical", sel!(library:))]
            .into_iter()
            .map(|(title, symbol, action)| nav_button(title, symbol, action, mtm))
            .collect();
    let mut items: Vec<&NSView> = vec![&brand];
    items.extend(nav.iter().map(|b| &**b as &NSView));
    let nav_stack = stack(&items, true, 6.0, mtm);
    nav_stack.setAlignment(objc2_app_kit::NSLayoutAttribute::Leading);
    nav_stack.setCustomSpacing_afterView(20.0, &brand);
    sidebar.addSubview(&nav_stack);
    nav_stack.setTranslatesAutoresizingMaskIntoConstraints(false);
    nav_stack.topAnchor().constraintEqualToAnchor_constant(&sidebar.topAnchor(), 52.0).setActive(true);
    nav_stack.leadingAnchor().constraintEqualToAnchor_constant(&sidebar.leadingAnchor(), 16.0).setActive(true);
    nav_stack.trailingAnchor().constraintEqualToAnchor_constant(&sidebar.trailingAnchor(), -16.0).setActive(true);
    for b in &nav {
        b.widthAnchor().constraintEqualToAnchor(&nav_stack.widthAnchor()).setActive(true);
    }

    // Top bar: back + search.
    let back = symbol_button("chevron.left", sel!(back:), 15.0, mtm);
    let search = search_field(mtm);
    size(&search, Some(460.0), None);
    let top = stack(&[&back, &search], false, 14.0, mtm);
    top.setAlignment(objc2_app_kit::NSLayoutAttribute::CenterY);

    let content = NSView::new(mtm);
    let (bar_view, bar) = player_bar(mtm);

    for v in [&*sidebar as &NSView, &*top, &*content, &*bar_view] {
        root.addSubview(v);
        v.setTranslatesAutoresizingMaskIntoConstraints(false);
    }
    let c = |a: Retained<objc2_app_kit::NSLayoutConstraint>| a.setActive(true);
    c(bar_view.leadingAnchor().constraintEqualToAnchor(&root.leadingAnchor()));
    c(bar_view.trailingAnchor().constraintEqualToAnchor(&root.trailingAnchor()));
    c(bar_view.bottomAnchor().constraintEqualToAnchor(&root.bottomAnchor()));
    c(bar_view.heightAnchor().constraintEqualToConstant(PLAYER_BAR));
    c(sidebar.topAnchor().constraintEqualToAnchor(&root.topAnchor()));
    c(sidebar.leadingAnchor().constraintEqualToAnchor(&root.leadingAnchor()));
    c(sidebar.bottomAnchor().constraintEqualToAnchor(&bar_view.topAnchor()));
    c(sidebar.widthAnchor().constraintEqualToConstant(SIDEBAR));
    c(top.topAnchor().constraintEqualToAnchor_constant(&root.topAnchor(), 12.0));
    c(top.leadingAnchor().constraintEqualToAnchor_constant(&sidebar.trailingAnchor(), 24.0));
    c(top.heightAnchor().constraintEqualToConstant(TOP_BAR - 24.0));
    c(content.topAnchor().constraintEqualToAnchor_constant(&root.topAnchor(), TOP_BAR));
    c(content.leadingAnchor().constraintEqualToAnchor(&sidebar.trailingAnchor()));
    c(content.trailingAnchor().constraintEqualToAnchor(&root.trailingAnchor()));
    c(content.bottomAnchor().constraintEqualToAnchor(&bar_view.topAnchor()));

    Shell { window, focus_sink, content, back, search, nav, bar }
}

fn nav_button(title: &str, symbol: &str, action: objc2::runtime::Sel, mtm: MainThreadMarker) -> Retained<NSButton> {
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(symbol), None).unwrap_or_default();
    let target: &AnyObject = actions(mtm);
    // SAFETY: the action target lives for the whole app.
    let b = unsafe { NSButton::buttonWithTitle_image_target_action(&NSString::from_str(title), &image, Some(target), Some(action), mtm) };
    b.setBordered(false);
    b.setImagePosition(NSCellImagePosition::ImageLeading);
    b.setAlignment(NSTextAlignment::Left);
    b.setFont(Some(&font(15.0, Weight::Medium)));
    b.setContentTintColor(Some(&text()));
    b.setWantsLayer(true);
    if let Some(layer) = b.layer() {
        layer.setCornerRadius(8.0);
    }
    size(&b, None, Some(40.0));
    b
}

/// Highlights the sidebar entry for the current section (or none).
pub fn select_nav(shell: &Shell, index: Option<usize>) {
    for (i, b) in shell.nav.iter().enumerate() {
        if let Some(layer) = b.layer() {
            let color = (Some(i) == index).then(|| super::hover().CGColor());
            layer.setBackgroundColor(color.as_deref());
        }
    }
}

fn player_bar(mtm: MainThreadMarker) -> (Retained<NSView>, PlayerBar) {
    let root = NSView::new(mtm);
    root.setWantsLayer(true);
    if let Some(layer) = root.layer() {
        layer.setBackgroundColor(Some(&bar_bg().CGColor()));
    }

    let progress = Bar::new(Kind::Progress, 3.0, &accent(), mtm);

    let prev = symbol_button("backward.fill", sel!(previous:), 18.0, mtm);
    let play = symbol_button("play.fill", sel!(playPause:), 26.0, mtm);
    let next = symbol_button("forward.fill", sel!(next:), 18.0, mtm);
    let time = label("", 12.0, Weight::Regular, true, mtm);
    let controls = stack(&[&prev, &play, &next, &time], false, 18.0, mtm);
    controls.setAlignment(objc2_app_kit::NSLayoutAttribute::CenterY);
    controls.setCustomSpacing_afterView(24.0, &next);

    let art = image_view(44.0, 44.0, 4.0, mtm);
    let title = label("Nothing playing", 14.0, Weight::Medium, false, mtm);
    let artist = label("", 13.0, Weight::Regular, true, mtm);
    let text = stack(&[&title, &artist], true, 2.0, mtm);
    text.setAlignment(objc2_app_kit::NSLayoutAttribute::Leading);
    title.widthAnchor().constraintLessThanOrEqualToConstant(360.0).setActive(true);
    artist.widthAnchor().constraintLessThanOrEqualToConstant(360.0).setActive(true);
    let now = stack(&[&art, &text], false, 12.0, mtm);
    now.setAlignment(objc2_app_kit::NSLayoutAttribute::CenterY);

    // Only an icon: disabled, so its action never fires.
    let speaker = symbol_button("speaker.wave.2.fill", sel!(playPause:), 14.0, mtm);
    speaker.setEnabled(false);
    let volume = Bar::new(Kind::Volume, 4.0, &super::text(), mtm);
    size(&volume, Some(110.0), Some(16.0));
    let right = stack(&[&speaker, &volume], false, 8.0, mtm);
    right.setAlignment(objc2_app_kit::NSLayoutAttribute::CenterY);

    for v in [&*progress as &NSView, &controls, &now, &right] {
        root.addSubview(v);
        v.setTranslatesAutoresizingMaskIntoConstraints(false);
    }
    let c = |a: Retained<objc2_app_kit::NSLayoutConstraint>| a.setActive(true);
    c(progress.topAnchor().constraintEqualToAnchor(&root.topAnchor()));
    c(progress.heightAnchor().constraintEqualToConstant(10.0));
    c(progress.leadingAnchor().constraintEqualToAnchor(&root.leadingAnchor()));
    c(progress.trailingAnchor().constraintEqualToAnchor(&root.trailingAnchor()));
    c(controls.leadingAnchor().constraintEqualToAnchor_constant(&root.leadingAnchor(), 20.0));
    c(controls.centerYAnchor().constraintEqualToAnchor_constant(&root.centerYAnchor(), 4.0));
    c(now.centerXAnchor().constraintEqualToAnchor(&root.centerXAnchor()));
    c(now.centerYAnchor().constraintEqualToAnchor_constant(&root.centerYAnchor(), 4.0));
    c(right.trailingAnchor().constraintEqualToAnchor_constant(&root.trailingAnchor(), -20.0));
    c(right.centerYAnchor().constraintEqualToAnchor_constant(&root.centerYAnchor(), 4.0));

    (root, PlayerBar { art, title, artist, play, time, progress, volume })
}

/// The menu bar: app, edit (so copy/paste work in the search field),
/// playback and window menus.
pub fn install_menu(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    let bar = NSMenu::new(mtm);
    let target: &AnyObject = actions(mtm);

    let app_menu = menu("Kilo", mtm);
    add(&app_menu, "About Kilo", Some(sel!(orderFrontStandardAboutPanel:)), "", None, mtm);
    app_menu.addItem(&NSMenuItem::separatorItem(mtm));
    add(&app_menu, "Hide Kilo", Some(sel!(hide:)), "h", None, mtm);
    add(
        &app_menu,
        "Hide Others",
        Some(sel!(hideOtherApplications:)),
        "h",
        Some(NSEventModifierFlags::Command | NSEventModifierFlags::Option),
        mtm,
    );
    add(&app_menu, "Show All", Some(sel!(unhideAllApplications:)), "", None, mtm);
    app_menu.addItem(&NSMenuItem::separatorItem(mtm));
    add(&app_menu, "Quit Kilo", Some(sel!(terminate:)), "q", None, mtm);

    let edit = menu("Edit", mtm);
    for (title, action, key) in [
        ("Undo", sel!(undo:), "z"),
        ("Redo", sel!(redo:), "Z"),
        ("Cut", sel!(cut:), "x"),
        ("Copy", sel!(copy:), "c"),
        ("Paste", sel!(paste:), "v"),
        ("Select All", sel!(selectAll:), "a"),
    ] {
        add(&edit, title, Some(action), key, None, mtm);
    }
    let find = add(&edit, "Search", Some(sel!(focusSearch:)), "f", None, mtm);
    // SAFETY: the target lives for the whole app.
    unsafe { find.setTarget(Some(target)) };

    let playback = menu("Playback", mtm);
    for (title, action, key) in [
        ("Play/Pause", sel!(playPause:), "p"),
        ("Next", sel!(next:), "\u{F703}"),
        ("Previous", sel!(previous:), "\u{F702}"),
        ("Back", sel!(back:), "["),
    ] {
        let item = add(&playback, title, Some(action), key, None, mtm);
        // SAFETY: as above.
        unsafe { item.setTarget(Some(target)) };
    }

    let window = menu("Window", mtm);
    add(&window, "Minimize", Some(sel!(performMiniaturize:)), "m", None, mtm);
    add(&window, "Close", Some(sel!(performClose:)), "w", None, mtm);

    for m in [&app_menu, &edit, &playback, &window] {
        let holder = NSMenuItem::new(mtm);
        holder.setSubmenu(Some(m));
        bar.addItem(&holder);
    }
    app.setMainMenu(Some(&bar));
    app.setWindowsMenu(Some(&window));
}

fn menu(title: &str, mtm: MainThreadMarker) -> Retained<NSMenu> {
    NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title))
}

fn add(
    menu: &NSMenu,
    title: &str,
    action: Option<objc2::runtime::Sel>,
    key: &str,
    modifiers: Option<NSEventModifierFlags>,
    mtm: MainThreadMarker,
) -> Retained<NSMenuItem> {
    // SAFETY: standard NSMenuItem initializer.
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &NSString::from_str(title), action, &NSString::from_str(key))
    };
    if let Some(m) = modifiers {
        item.setKeyEquivalentModifierMask(m);
    }
    menu.addItem(&item);
    item
}
