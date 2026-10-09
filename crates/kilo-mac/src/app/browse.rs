//! Pages: navigation, loading a page, and loading more of it as it scrolls.

use std::cell::Cell;
use std::rc::Rc;

use dispatch2::{DispatchQueue, DispatchTime};
use kilo_core::client::Client;
use kilo_core::model::{Page, Section, Target};
use kilo_core::parse;
use kilo_core::queue::Queue;

use super::queue::{self, Follow};
use super::{Screen, mtm, session, set_content, show_error, show_loading, show_sign_in, with};
use crate::paths;
use crate::strings::{S, t};
use crate::ui::page::{Items, More, PageView};
use crate::ui::shell;
use crate::{images, net, pagecache};

thread_local! {
    static REFRESH_PENDING: Cell<bool> = const { Cell::new(false) };
}

/// A page in the history, and how far down it was scrolled when left (to
/// come back to the same place).
#[derive(Clone, Debug)]
pub(super) struct Visit {
    pub route: Route,
    pub scrolled: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Route {
    Home,
    Explore,
    Library,
    Browse { id: Box<str>, params: Option<Box<str>> },
    Search(String),
}

impl Route {
    /// What the page is, for the page cache.
    fn cache_name(&self) -> String {
        match self {
            Route::Home => "home".into(),
            Route::Explore => "explore".into(),
            Route::Library => "library".into(),
            Route::Browse { id, params } => format!("browse {id} {}", params.as_deref().unwrap_or("")),
            Route::Search(q) => format!("search {q}"),
        }
    }

    /// The sidebar entry this route highlights, if any.
    pub(super) fn nav_index(&self) -> Option<usize> {
        match self {
            Route::Home => Some(0),
            Route::Explore => Some(1),
            Route::Library => Some(2),
            _ => None,
        }
    }
}

pub fn go(route: Route) {
    super::release_search_focus();
    let same = with(|a| {
        let scrolled = a.view.as_ref().map_or(0.0, |v| v.scrolled());
        if let Some(last) = a.history.last_mut() {
            last.scrolled = scrolled;
        }
        let same = a.history.last().is_some_and(|v| v.route == route);
        if !same {
            a.history.push(Visit { route: route.clone(), scrolled: 0.0 });
        }
        same
    })
    .unwrap_or(false);
    // The page already showing, chosen again: fresh from YouTube.
    load(route, same, 0.0);
}

/// The previous page, where it was scrolled to.
pub fn back() {
    if let Some(Some(visit)) = with(|a| {
        (a.history.len() > 1).then(|| {
            a.history.pop();
            a.history.last().cloned()
        })?
    }) {
        load(visit.route, false, visit.scrolled);
    }
}

/// Loads the current page again, fresh from YouTube (Reload, Try again), or
/// connects again if nothing loaded yet. Not while signing in or out.
pub fn reload() {
    let Some((route, busy)) = with(|a| (a.history.last().map(|v| v.route.clone()), a.account_busy)) else { return };
    if busy {
        return;
    }
    if let Some(route) = route {
        load(route, true, 0.0);
    } else if let Some(s) = session::saved_session() {
        session::start(s);
    } else {
        show_sign_in();
    }
}

/// Shows `route`'s page, scrolled to `scrolled`: at once from the page
/// cache if it's there (and, unless it's fresh, or `force`, again from
/// YouTube after).
fn load(route: Route, force: bool, scrolled: f64) {
    let Some((client, generation)) = with(|a| {
        a.generation += 1;
        if let Some(s) = &a.shell {
            s.back.setEnabled(a.history.len() > 1);
            shell::select_nav(s, route.nav_index());
        }
        (a.client.clone(), a.generation)
    }) else {
        return;
    };
    let Some(client) = client else { return show_sign_in() };
    // A page shown before appears at once; if it's older than half an hour,
    // a fresh one follows.
    let file = pagecache::file(&route.cache_name(), &client.config().hl, client.session().fingerprint());
    let cached = pagecache::read(&file).and_then(|(json, fresh)| Some((Rc::new(parse_page(&route, &json).ok()?), fresh && !force)));
    let from_cache = cached.is_some();
    match cached {
        Some((page, fresh)) => {
            crate::debug::trace(|| format!("page: from cache{}", if fresh { "" } else { " (stale)" }));
            show_page_at(page, scrolled);
            with(|a| a.stale_page = !fresh);
            if fresh {
                return;
            }
        }
        None => show_loading(),
    }
    crate::debug::trace(|| format!("page: {route:?} requested"));
    let epoch = paths::epoch();
    net::run(
        net::Pool::Api,
        move || {
            let json = fetch(&client, &route)?;
            let page = parse_page(&route, &json)?;
            pagecache::write(&file, &json, epoch);
            Ok(page)
        },
        move |result: kilo_core::Result<Page>| {
            crate::debug::trace(|| "page: fetched".into());
            if with(|a| a.generation) != Some(generation) {
                return; // the user moved on
            }
            match result {
                // Over the stale copy only while it's untouched (not
                // scrolled), and only if it changed.
                Ok(page) if from_cache => {
                    let untouched =
                        with(|a| a.view.as_ref().is_some_and(|v| v.scrolled() < 1.0) && a.page.as_deref() != Some(&page)).unwrap_or(false);
                    if untouched {
                        show_page(Rc::new(page));
                    }
                }
                Ok(page) => show_page_at(Rc::new(page), scrolled),
                Err(kilo_core::Error::SignedOut) => session::session_ended(),
                Err(e) => {
                    eprintln!("kilo: {e}");
                    if !from_cache {
                        show_error(t(S::LoadFailed));
                    }
                }
            }
        },
    );
}

/// The first part of a page, raw. More shelves and the rest of long lists
/// load as they scroll into view (`check_more`).
fn fetch(client: &Client, route: &Route) -> kilo_core::Result<Vec<u8>> {
    match route {
        Route::Home => client.browse("FEmusic_home", None),
        Route::Explore => client.browse("FEmusic_explore", None),
        Route::Library => client.browse("FEmusic_liked_playlists", None),
        Route::Browse { id, params } => client.browse(id, params.as_deref()),
        Route::Search(q) => client.search(q, None),
    }
}

fn parse_page(route: &Route, json: &[u8]) -> kilo_core::Result<Page> {
    match route {
        Route::Search(_) => parse::search(json),
        _ => parse::browse(json),
    }
}

/// Replaces the page area with `page`, at the top.
pub(super) fn show_page(page: Rc<Page>) {
    show_page_at(page, 0.0);
}

/// Replaces the page area with `page`, scrolled `scrolled` points down (as
/// far as it goes).
pub(super) fn show_page_at(page: Rc<Page>, scrolled: f64) {
    let mtm = mtm();
    if page.header.is_none() && page.sections.iter().all(|s| s.entries().is_empty() && !matches!(s, Section::Text { .. })) {
        return super::show_empty();
    }
    let Some(scale) = with(|a| {
        a.view = None;
        a.screen = Screen::Page;
        a.stale_page = false;
        a.page = Some(page.clone());
        a.shell.as_ref().map(|s| s.window.backingScaleFactor())
    })
    .flatten() else {
        return; // the window is closed: the page shows when it reopens
    };
    let mut items = Items::new();
    let view = PageView::new(page, scale, &mut items, mtm);
    let root = view.root.clone();
    with(|a| a.items = items);
    set_content(&root);
    // Lay out now so the first refresh knows the page's size.
    with(|a| {
        if let Some(s) = &a.shell {
            s.window.layoutIfNeeded();
            if scrolled > 0.0 {
                view.scroll_to(Some(scrolled));
            }
            view.refresh();
            crate::debug::schedule_snapshot(&s.window);
        }
        a.view = Some(view);
    });
    crate::debug::trace(|| "page: shown".into());
    images::sweep_soon();
    check_more();
}

/// After scrolling or resizing (coalesced): creates and drops the page's
/// views, and loads more of the page if its end is in sight.
pub(super) fn schedule_refresh() {
    if REFRESH_PENDING.replace(true) {
        return;
    }
    net::later(|| {
        REFRESH_PENDING.set(false);
        with(|a| {
            if let Some(v) = &a.view {
                v.refresh();
            }
        });
        images::sweep_soon();
        check_more();
    });
}

/// Loads the next part of the page if its end is coming into view.
fn check_more() {
    if let Some(Some((kind, token))) = with(|a| a.view.as_ref()?.wants_more()) {
        fetch_more(kind, token);
    }
}

/// Fetches the part of a page that `token` points to, then adds it to the
/// page (if it's still on screen) and to a queue following it.
pub(super) fn fetch_more(kind: More, token: Box<str>) {
    let Some(Some((client, session))) = with(|a| {
        let client = a.client.clone()?;
        if a.fetching.contains(&token) {
            return None;
        }
        a.fetching.push(token.clone());
        Some((client, a.session))
    }) else {
        return;
    };
    let t = token.clone();
    net::run(
        net::Pool::Api,
        move || client.continuation(&t).and_then(|j| parse::browse(&j)),
        move |result| {
            if with(|a| a.session) != Some(session) {
                return; // signed out meanwhile: nothing of it is wanted
            }
            let more = match result {
                Ok(more) => more,
                Err(kilo_core::Error::SignedOut) => return session::session_ended(),
                Err(_) => {
                    // A page from a stale cache may hold expired tokens: load
                    // it again (the cache is fresh by now). Only if the token
                    // is the page's: the queue may be following another list.
                    let reload_page =
                        with(|a| a.page.as_deref().is_some_and(|p| holds_token(p, &token)) && std::mem::take(&mut a.stale_page));
                    if reload_page.unwrap_or(false) {
                        crate::debug::trace(|| "page: stale token; reloading".into());
                        with(|a| a.fetching.retain(|f| *f != token));
                        return reload();
                    }
                    // The token stays on the page, so a later scroll retries; not
                    // before a few seconds, though, or every scroll would (offline).
                    let when = DispatchTime::NOW.time(5_000_000_000);
                    let _ = DispatchQueue::main().after(when, move || {
                        with(|a| a.fetching.retain(|f| *f != token));
                    });
                    return;
                }
            };
            with(|a| a.fetching.retain(|f| *f != token));
            match kind {
                More::Page => {
                    let next = if more.sections.is_empty() { None } else { more.continuation };
                    grow_page(|page| {
                        if page.continuation.as_deref() != Some(&*token) {
                            return false;
                        }
                        page.sections.extend(more.sections);
                        page.continuation = next;
                        true
                    });
                }
                More::List => {
                    let (entries, next) = more
                        .sections
                        .into_iter()
                        .find_map(|s| match s {
                            Section::List { entries, continuation, .. } => Some((entries, continuation)),
                            _ => None,
                        })
                        .unwrap_or_default();
                    let next = if entries.is_empty() { None } else { next };
                    grow_page(|page| {
                        let Some(Section::List { entries: list, continuation, .. }) =
                            page.sections.iter_mut().find(|s| matches!(s, Section::List { continuation: Some(c), .. } if *c == token))
                        else {
                            return false;
                        };
                        list.extend(entries.iter().cloned());
                        *continuation = next.clone();
                        true
                    });
                    queue::feed(&token, entries, next);
                }
            }
        },
    );
}

/// Whether `token` is where `page` goes on: its own next part, or the rest
/// of one of its lists.
fn holds_token(page: &Page, token: &str) -> bool {
    page.continuation.as_deref() == Some(token)
        || page.sections.iter().any(|s| matches!(s, Section::List { continuation: Some(c), .. } if &**c == token))
}

/// Replaces the page on screen with a grown copy of it, keeping the scroll
/// position. `grow` returns false if the page isn't the one the new part
/// belongs to (it checks the continuation token).
fn grow_page(grow: impl FnOnce(&mut Page) -> bool) {
    let grown = with(|a| {
        let Some(current) = &a.page else { return false };
        let mut page = (**current).clone();
        if !grow(&mut page) {
            return false;
        }
        let page = Rc::new(page);
        a.page = Some(page.clone());
        if let Some(view) = a.view.as_mut() {
            view.set_page(page, &mut a.items);
            view.refresh();
        }
        true
    });
    if grown == Some(true) {
        // Keep going until the screen is full.
        net::later(check_more);
    }
}

/// A card or row was clicked.
pub fn activate(item: u32) {
    let Some(Some((entry, siblings, rest, header, ei))) = with(|a| {
        let &(si, ei) = a.items.get(item as usize)?;
        let page = a.page.as_ref()?;
        let section = page.sections.get(si)?;
        let rest = match section {
            Section::List { continuation, .. } => Some(continuation.clone()),
            _ => None,
        };
        Some((section.entries().get(ei)?.clone(), section.entries().to_vec(), rest, page.header.clone(), ei))
    }) else {
        return;
    };
    match &entry.target {
        Target::Play { .. } => {
            // A row in a list plays the list from there, like YouTube Music.
            let (pool, at, rest) = match rest {
                Some(rest) => (siblings, ei, rest),
                None => (vec![entry.clone()], 0, None),
            };
            let follow = rest.map(|next| Follow::new(next, false, header.clone()));
            queue::start(Queue::starting_at(&queue::decorate(pool, header.as_ref()), at), follow);
        }
        Target::PlayPlaylist { playlist_id } => queue::play_playlist(playlist_id.to_string()),
        Target::Browse { id, params, .. } => go(Route::Browse { id: id.clone(), params: params.clone() }),
        Target::None => {}
    }
}

/// A card's play button: plays its track or mix as a click does, and an
/// album or playlist without opening it.
pub fn play_item(item: u32) {
    let Some(Some(entry)) = with(|a| {
        let &(si, ei) = a.items.get(item as usize)?;
        Some(a.page.as_ref()?.sections.get(si)?.entries().get(ei)?.clone())
    }) else {
        return;
    };
    let Target::Browse { id, params, .. } = entry.target else { return activate(item) };
    let Some((client, intent)) = queue::new_intent() else { return };
    net::run(
        net::Pool::Api,
        move || client.browse(&id, params.as_deref()).and_then(|j| parse::browse(&j)),
        move |result| {
            if !queue::still_wanted(intent) {
                return; // something else was started meanwhile
            }
            match result {
                Ok(page) => queue::play_page(&page, false),
                Err(e) => queue::failed(e),
            }
        },
    );
}

/// Scrolls the page to `y` points, or to its end (Home and End, and a
/// developer switch).
pub fn scroll_to(y: Option<f64>) {
    with(|a| {
        if let Some(v) = &a.view {
            v.scroll_to(y);
        }
    });
}

/// Scrolls the page by `points`, plus `screens` screenfuls (keyboard
/// scrolling).
pub fn scroll_by(points: f64, screens: f64) {
    with(|a| {
        if let Some(v) = &a.view {
            v.scroll_by(points, screens);
        }
    });
}

#[cfg(test)]
mod tests {
    use kilo_core::model::{Page, Section};

    use super::holds_token;

    #[test]
    fn knows_which_tokens_are_the_pages() {
        let list = |token: Option<&str>| Section::List { title: "".into(), entries: Vec::new(), continuation: token.map(Into::into) };
        let page = Page { header: None, sections: vec![list(None), list(Some("ROWS"))], continuation: Some("SHELVES".into()) };
        assert!(holds_token(&page, "SHELVES"));
        assert!(holds_token(&page, "ROWS"));
        // Another list's (one a queue follows, say).
        assert!(!holds_token(&page, "ELSEWHERE"));
    }
}
