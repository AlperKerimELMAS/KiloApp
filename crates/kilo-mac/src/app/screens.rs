//! The page area: filling it, and its message screens (loading, sign-in,
//! signing out, errors, nothing here).

use objc2::rc::Retained;
use objc2::runtime::Sel;
use objc2::sel;
use objc2_app_kit::{NSApplication, NSAutoresizingMaskOptions, NSImageView, NSTextAlignment, NSView};

use super::{Screen, mtm, with, with_shell};
use crate::strings::{S, t};
use crate::ui::shell;
use crate::ui::{self, Weight};

/// Fills the page area with `view`, sized by autoresizing rather than
/// constraints so the page stays out of Auto Layout.
pub(super) fn set_content(view: &NSView) {
    with_shell(|s| {
        for old in s.content.subviews().iter() {
            old.removeFromSuperview();
        }
        view.setFrame(s.content.bounds());
        view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable);
        s.content.addSubview(view);
    });
}

/// The page area while a page loads: its shape in placeholder gray.
pub(super) fn show_loading() {
    with(|a| {
        a.view = None;
        a.screen = Screen::Loading;
    });
    if with(|a| a.shell.is_some()).unwrap_or(false) {
        set_content(&ui::page::skeleton(mtm()));
    }
}

/// A button on a message screen: its title, action, and whether it's the
/// filled call to action.
pub(super) type Button<'a> = (&'a str, Sel, bool);

/// Fills the page area with a centered message: the app's icon (`icon`),
/// a title, text, buttons and a small note under them.
pub(super) fn message(screen: Screen, icon: bool, title: &str, body: &str, buttons: &[Button], note: &str) {
    let mtm = mtm();
    with(|a| {
        a.view = None;
        a.screen = screen;
    });
    if with(|a| a.shell.is_none()).unwrap_or(true) {
        return;
    }
    let mut views: Vec<Retained<NSView>> = Vec::new();
    if icon && let Some(image) = NSApplication::sharedApplication(mtm).applicationIconImage() {
        let image = NSImageView::imageViewWithImage(&image, mtm);
        ui::size(&image, Some(96.0), Some(96.0));
        views.push(Retained::into_super(Retained::into_super(image)));
    }
    views.push(Retained::into_super(Retained::into_super(ui::label(title, 28.0, Weight::Bold, false, mtm))));
    if !body.is_empty() {
        let b = ui::paragraph(body, 15.0, true, 4, 420.0, mtm);
        b.setAlignment(NSTextAlignment::Center);
        views.push(Retained::into_super(Retained::into_super(b)));
    }
    let buttons_at = views.len();
    if !buttons.is_empty() {
        let made: Vec<Retained<objc2_app_kit::NSButton>> =
            buttons.iter().map(|&(label, action, primary)| ui::pill_button(label, action, 44.0, primary, mtm)).collect();
        let refs: Vec<&NSView> = made.iter().map(|b| &**b as &NSView).collect();
        views.push(Retained::into_super(ui::stack(&refs, false, 12.0, mtm)));
    }
    if !note.is_empty() {
        let n = ui::paragraph(note, 12.0, true, 2, 360.0, mtm);
        n.setAlignment(NSTextAlignment::Center);
        views.push(Retained::into_super(Retained::into_super(n)));
    }
    let refs: Vec<&NSView> = views.iter().map(|v| &**v).collect();
    let col = ui::stack(&refs, true, 14.0, mtm);
    if !buttons.is_empty() && buttons_at > 0 {
        // More room above the buttons.
        col.setCustomSpacing_afterView(24.0, refs[buttons_at - 1]);
    }
    let holder = NSView::new(mtm);
    holder.addSubview(&col);
    col.setTranslatesAutoresizingMaskIntoConstraints(false);
    col.centerXAnchor().constraintEqualToAnchor(&holder.centerXAnchor()).setActive(true);
    col.centerYAnchor().constraintEqualToAnchor_constant(&holder.centerYAnchor(), -20.0).setActive(true);
    set_content(&holder);
    with_shell(|s| crate::debug::schedule_snapshot(&s.window));
}

pub(super) fn show_error(text: &str) {
    message(Screen::Error(text.into()), false, t(S::SomethingWrong), text, &[(t(S::TryAgain), sel!(retry:), true)], "");
}

pub(super) fn show_sign_in() {
    with_shell(|s| shell::select_nav(s, None));
    message(Screen::SignIn, true, t(S::Welcome), t(S::WelcomeBody), &[(t(S::SignInWithGoogle), sel!(signIn:), true)], t(S::SignInNote));
}

/// The sign-in screen for someone whose session ended (it lasts a week at
/// most: Google gives youtube.com's copy of it 7 days).
pub(super) fn show_session_ended() {
    with_shell(|s| shell::select_nav(s, None));
    message(
        Screen::SessionEnded,
        true,
        t(S::SessionEnded),
        t(S::SessionEndedBody),
        &[(t(S::SignInWithGoogle), sel!(signIn:), true)],
        t(S::SignInNote),
    );
}

pub(super) fn show_empty() {
    message(Screen::Empty, false, t(S::NothingHere), t(S::NothingHereBody), &[], "");
}

pub(super) fn show_signing_out() {
    message(Screen::SigningOut, false, t(S::SigningOut), "", &[], "");
}

pub(super) fn show_sign_out_failed() {
    message(Screen::SignOutFailed, false, t(S::SignOutFailed), t(S::SignOutFailedBody), &[(t(S::TryAgain), sel!(signOut:), true)], "");
}
