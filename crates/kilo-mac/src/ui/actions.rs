//! The target every button and menu item sends its action to, and the
//! search field's delegate.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSButton, NSControl, NSControlStateValueOff, NSControlStateValueOn, NSControlTextEditingDelegate, NSMenuItem, NSMenuItemValidation,
    NSTextField, NSTextFieldDelegate, NSTextView, NSView,
};
use objc2_foundation::{NSNotification, NSString};

use super::search::set_ring;
use crate::app::{self, Route};
use crate::strings::{S, t};

// The target every button and menu item sends its action to, and the
// search field's delegate.
define_class!(
    // SAFETY: NSObject subclass with no ivars; action methods take a sender;
    // protocol methods match AppKit's signatures.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "KiloActions"]
    pub struct Actions;

    impl Actions {
        #[unsafe(method(home:))]
        fn home(&self, _sender: Option<&AnyObject>) {
            app::go(Route::Home);
        }

        #[unsafe(method(explore:))]
        fn explore(&self, _sender: Option<&AnyObject>) {
            app::go(Route::Explore);
        }

        #[unsafe(method(library:))]
        fn library(&self, _sender: Option<&AnyObject>) {
            app::go(Route::Library);
        }

        #[unsafe(method(back:))]
        fn back(&self, _sender: Option<&AnyObject>) {
            app::back();
        }

        #[unsafe(method(search:))]
        fn search(&self, sender: Option<&AnyObject>) {
            if let Some(field) = sender.and_then(|s| s.downcast_ref::<NSTextField>()) {
                let query = field.stringValue().to_string();
                if !query.trim().is_empty() {
                    app::go(Route::Search(query.trim().to_owned()));
                    app::release_search_focus();
                }
            }
        }

        #[unsafe(method(focusSearch:))]
        fn focus_search(&self, _sender: Option<&AnyObject>) {
            app::focus_search();
        }

        #[unsafe(method(playPause:))]
        fn play_pause(&self, _sender: Option<&AnyObject>) {
            app::play_pause();
        }

        #[unsafe(method(next:))]
        fn next(&self, _sender: Option<&AnyObject>) {
            app::next();
        }

        #[unsafe(method(previous:))]
        fn previous(&self, _sender: Option<&AnyObject>) {
            app::previous();
        }

        #[unsafe(method(playAll:))]
        fn play_all(&self, _sender: Option<&AnyObject>) {
            app::play_all(false);
        }

        #[unsafe(method(shuffleAll:))]
        fn shuffle_all(&self, _sender: Option<&AnyObject>) {
            app::play_all(true);
        }

        #[unsafe(method(signIn:))]
        fn sign_in(&self, _sender: Option<&AnyObject>) {
            app::sign_in();
        }

        #[unsafe(method(signOut:))]
        fn sign_out(&self, _sender: Option<&AnyObject>) {
            app::sign_out();
        }

        #[unsafe(method(retry:))]
        fn retry(&self, _sender: Option<&AnyObject>) {
            app::reload();
        }

        #[unsafe(method(cancelSignIn:))]
        fn cancel_sign_in(&self, _sender: Option<&AnyObject>) {
            app::cancel_sign_in();
        }

        #[unsafe(method(playItem:))]
        fn play_item(&self, sender: Option<&AnyObject>) {
            if let Some(button) = sender.and_then(|s| s.downcast_ref::<NSButton>()) {
                let item = button.tag() as u32;
                crate::net::later(move || app::play_item(item));
            }
        }

        #[unsafe(method(account:))]
        fn account(&self, _sender: Option<&AnyObject>) {
            // The menu runs its own event loop: outside this action.
            crate::net::later(app::show_account_menu);
        }

        #[unsafe(method(setAppearance:))]
        fn set_appearance(&self, sender: Option<&AnyObject>) {
            if let Some(item) = sender.and_then(|s| s.downcast_ref::<NSMenuItem>()) {
                let tag = item.tag();
                crate::net::later(move || app::set_appearance(tag));
            }
        }

        #[unsafe(method(setLanguage:))]
        fn set_language(&self, sender: Option<&AnyObject>) {
            if let Some(item) = sender.and_then(|s| s.downcast_ref::<NSMenuItem>()) {
                let tag = item.tag();
                crate::net::later(move || app::set_language(tag));
            }
        }

        #[unsafe(method(mute:))]
        fn mute(&self, _sender: Option<&AnyObject>) {
            app::toggle_mute();
        }

        #[unsafe(method(toggleSidebar:))]
        fn toggle_sidebar(&self, _sender: Option<&AnyObject>) {
            app::toggle_sidebar();
        }

        /// The sidebar animation's display link.
        #[unsafe(method(sidebarFrame:))]
        fn sidebar_frame(&self, _link: Option<&AnyObject>) {
            app::sidebar_frame();
        }

        /// A menu item for a keyboard shortcut; its tag is the action.
        #[unsafe(method(shortcut:))]
        fn shortcut(&self, sender: Option<&AnyObject>) {
            if let Some(action) = sender.and_then(|s| s.downcast_ref::<NSMenuItem>()).and_then(|i| crate::keys::action(i.tag())) {
                crate::keys::perform(action);
            }
        }
    }

    unsafe impl NSObjectProtocol for Actions {}

    unsafe impl NSMenuItemValidation for Actions {
        #[unsafe(method(validateMenuItem:))]
        fn validate_menu_item(&self, item: &NSMenuItem) -> bool {
            // (No early returns: `define_class!` converts only the last
            // expression to Objective-C's BOOL.)
            let action = (item.action() == Some(sel!(shortcut:))).then(|| crate::keys::action(item.tag())).flatten();
            if action == Some(kilo_core::shortcuts::Action::Mute) {
                item.setState(if app::is_muted() { NSControlStateValueOn } else { NSControlStateValueOff });
            }
            if action == Some(kilo_core::shortcuts::Action::ToggleSidebar) {
                let title = if crate::settings::sidebar_collapsed() { S::ExpandSidebar } else { S::CollapseSidebar };
                item.setTitle(&NSString::from_str(t(title)));
            }
            // A key typed into the search field: the arrows and plain keys
            // are for the text, so the item lets them through.
            let mtm = MainThreadMarker::new().expect("main thread");
            action.is_none()
                || !crate::keys::typing_key(mtm)
                || action.and_then(kilo_core::shortcuts::shown).is_some_and(kilo_core::shortcuts::Shortcut::while_typing)
        }
    }

    unsafe impl NSControlTextEditingDelegate for Actions {
        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn do_command(&self, control: &NSControl, _view: &NSTextView, command: Sel) -> bool {
            // Escape clears the search field, then leaves it.
            let escape = command == sel!(cancelOperation:);
            if escape && control.stringValue().length() == 0 {
                crate::net::later(app::release_search_focus);
            } else if escape {
                control.setStringValue(&NSString::from_str(""));
            }
            escape
        }

        #[unsafe(method(controlTextDidEndEditing:))]
        fn did_end_editing(&self, notification: &NSNotification) {
            if let Some(field) = notification.object().and_then(|o| o.downcast::<NSView>().ok()) {
                set_ring(&field, false);
            }
        }
    }

    unsafe impl NSTextFieldDelegate for Actions {}
);

thread_local! {
    static ACTIONS: std::cell::OnceCell<Retained<Actions>> = const { std::cell::OnceCell::new() };
}

/// The app-wide action target (lives as long as the app).
pub fn actions(mtm: MainThreadMarker) -> &'static Actions {
    ACTIONS.with(|a| {
        let actions = a.get_or_init(|| {
            // SAFETY: plain NSObject init.
            unsafe { msg_send![Actions::alloc(mtm), init] }
        });
        // SAFETY: the Retained lives in a thread-local for the rest of the
        // process, on the main thread only.
        unsafe { &*Retained::as_ptr(actions) }
    })
}
