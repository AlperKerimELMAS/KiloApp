//! What plays next: queues started from a page, a playlist or one track; the
//! rest of a long list joining as the queue gets to it; and, once it all
//! runs out, the radio of what's playing. Radios and mixes are endless:
//! more of the same arrives as the queue runs low, as in YouTube Music.

use std::sync::Arc;

use kilo_core::client::Client;
use kilo_core::model::{Entry, Header, Page, Section, Target};
use kilo_core::parse::{self, UpNext};
use kilo_core::queue::Queue;

use super::browse::fetch_more;
use super::player::play_current;
use super::{session, with};
use crate::net;
use crate::ui::page::More;

/// When this few tracks are left, more is fetched (the list's next part,
/// more of the radio): the next track is always there when it's needed.
const LOW: usize = 3;

/// A queue started from a long list that's still loading: the rest of the
/// list joins the queue as it arrives.
pub(super) struct Follow {
    queue_id: u64,
    shuffle: bool,
    /// For decorating album tracks, as when the queue started.
    header: Option<Header>,
    /// The continuation token of the part that comes next.
    next: Box<str>,
}

impl Follow {
    pub(super) fn new(next: Box<str>, shuffle: bool, header: Option<Header>) -> Self {
        Follow { queue_id: 0, shuffle, header, next }
    }
}

/// More of the radio or mix queue `queue_id` came from.
pub(super) struct Endless {
    queue_id: u64,
    playlist: Box<str>,
    token: Box<str>,
}

/// Album tracks come without art or artist; borrow the album's.
pub(super) fn decorate(mut entries: Vec<Entry>, header: Option<&Header>) -> Vec<Entry> {
    if let Some(h) = header {
        let artist = h.detail.split(" • ").next().unwrap_or("").to_owned();
        for e in &mut entries {
            if e.thumb.is_none() {
                e.thumb = h.thumb.clone();
                if !artist.is_empty() {
                    e.subtitle = artist.clone().into();
                }
            }
        }
    }
    entries
}

/// The user asked for something to play that takes a request first: the
/// client to ask with, and the number of this wish. Only the latest wish
/// starts (`still_wanted`).
pub(super) fn new_intent() -> Option<(Arc<Client>, u64)> {
    with(|a| {
        a.play_intent += 1;
        Some((a.client.clone()?, a.play_intent))
    })
    .flatten()
}

/// Whether wish `intent` is still the latest (nothing was started since).
pub(super) fn still_wanted(intent: u64) -> bool {
    with(|a| a.play_intent == intent).unwrap_or(false)
}

/// Starts playing `queue`. With `follow`, the rest of the list it came from
/// joins as the queue gets to it.
pub(super) fn start(queue: Queue, follow: Option<Follow>) {
    start_endless(queue, follow, None);
}

/// `start`, for a queue that's the beginning of a radio or mix
/// (`playlist`) whose more comes with `token`.
fn start_endless(queue: Queue, follow: Option<Follow>, endless: Option<(Box<str>, Box<str>)>) {
    with(|a| {
        a.play_intent += 1;
        a.queue = queue;
        a.queue_id += 1;
        a.radio_for = None;
        a.follow = follow.map(|f| Follow { queue_id: a.queue_id, ..f });
        a.endless = endless.map(|(playlist, token)| Endless { queue_id: a.queue_id, playlist, token });
    });
    play_current(0.0);
}

/// "Play" / "Shuffle" on a page header.
pub fn play_all(shuffle: bool) {
    if let Some(Some(page)) = with(|a| a.page.clone()) {
        play_page(&page, shuffle);
    }
}

/// Plays the first list of `page` (an album's or playlist's tracks). A long
/// list starts right away with what's loaded; the rest joins as the queue
/// gets to it (all of it at once when shuffled, to shuffle it all).
pub(super) fn play_page(page: &Page, shuffle: bool) {
    let Some(list) = page.sections.iter().find(|s| matches!(s, Section::List { .. })) else { return };
    let rest = match list {
        Section::List { continuation, .. } => continuation.clone(),
        _ => None,
    };
    let mut entries = list.entries().iter().filter(|e| e.video_id().is_some()).cloned().collect::<Vec<_>>();
    let header = page.header.clone();
    if shuffle {
        with(|a| a.rng.shuffle(&mut entries));
    }
    if entries.is_empty() {
        return;
    }
    let follow = rest.map(|next| Follow::new(next, shuffle, header.clone()));
    start(Queue::starting_at(&decorate(entries, header.as_ref()), 0), follow);
}

/// Plays a whole playlist or mix from its start.
pub(super) fn play_playlist(playlist_id: String) {
    let Some((client, intent)) = new_intent() else { return };
    net::run(
        net::Pool::Api,
        move || client.next_playlist(&playlist_id).and_then(|j| parse::up_next(&j)).map(|q| (q, playlist_id)),
        move |result| {
            if !still_wanted(intent) {
                return; // something else was started meanwhile
            }
            match result {
                Ok((up, playlist)) if !up.entries.is_empty() => {
                    let endless = up.continuation.map(|token| (playlist.into(), token));
                    start_endless(Queue::starting_at(&up.entries, 0), None, endless);
                }
                Ok(_) => {}
                Err(e) => failed(e),
            }
        },
    );
}

/// A request for something to play failed. What was playing goes on; a
/// session YouTube no longer accepts ends here too.
pub(super) fn failed(e: kilo_core::Error) {
    match e {
        kilo_core::Error::SignedOut => session::session_ended(),
        e => eprintln!("kilo: {e}"),
    }
}

/// Plays a song by id, queued with its radio (as YouTube Music does when a
/// single song is started).
pub fn play_video(video_id: String) {
    let Some((client, intent)) = new_intent() else { return };
    net::run(
        net::Pool::Api,
        move || client.next(&video_id, None).and_then(|j| parse::up_next(&j)).map(|q| (q, video_id)),
        move |result| {
            if !still_wanted(intent) {
                return; // something else was started meanwhile
            }
            match result {
                Ok((up, id)) => {
                    let endless = up.continuation.map(|token| (Client::radio_id(&id).into(), token));
                    start_endless(Queue::from_entries(&up.entries, &id), None, endless);
                }
                Err(e) => failed(e),
            }
        },
    );
}

/// Developer switch: plays one music video the way a click on its card
/// does (a queue of one, then its radio).
pub fn play_music_video(video_id: &str) {
    let entry = Entry {
        title: video_id.into(),
        subtitle: "".into(),
        thumb: None,
        target: Target::Play { video_id: video_id.into(), playlist_id: None, music_video: true },
        duration: "".into(),
    };
    start(Queue::from_entries(&[entry], video_id), None);
}

/// Hands the part of a list that `token` pointed to to the queue that's
/// following that list (repeats and all), and asks for the part after it
/// if the queue will soon need it.
pub(super) fn feed(token: &str, entries: Vec<Entry>, next: Option<Box<str>>) {
    let fetch_next = with(|a| {
        let mut follow = a.follow.take()?;
        if follow.queue_id != a.queue_id || &*follow.next != token {
            a.follow = Some(follow);
            return None;
        }
        let entries = decorate(entries, follow.header.as_ref());
        if follow.shuffle {
            a.queue.append_shuffled(entries, &mut a.rng);
        } else {
            a.queue.append(entries);
        }
        // Without a next part the list is complete, and the follow ends.
        let next = next?;
        follow.next = next.clone();
        let soon = follow.shuffle || a.queue.remaining() <= LOW;
        a.follow = Some(follow);
        soon.then_some(next)
    })
    .flatten();
    if let Some(next) = fetch_next {
        fetch_more(More::List, next);
    }
    carry_on();
    extend();
}

/// If the queue ran out before more tracks arrived, plays the next one now
/// that they have.
fn carry_on() {
    if with(|a| a.ran_out && a.queue.advance().is_some()).unwrap_or(false) {
        play_current(0.0);
    }
}

/// When few tracks are left, makes sure more is on its way: the next part of
/// the list the queue follows, more of its radio or mix or, once all that's
/// done, the radio of what's playing, like YouTube Music.
pub(super) fn extend() {
    /// What to ask YouTube for.
    #[derive(Clone)]
    enum Ask {
        /// More of the radio or mix `playlist`, with its token.
        More { playlist: Box<str>, token: Box<str> },
        /// The radio of this track.
        Radio(String),
    }
    enum Wanted {
        /// The list's next part, by its continuation token.
        ListPart(Box<str>),
        Ask(Arc<Client>, Ask, u64),
    }
    let wanted = with(|a| {
        if a.queue.remaining() > LOW {
            return None;
        }
        if let Some(f) = &a.follow {
            // Fetched once (`fetch_more` won't ask twice at a time); asked
            // again if fetching it failed.
            return Some(Wanted::ListPart(f.next.clone()));
        }
        let client = a.client.clone()?;
        // Taken while it's fetched, so it's asked for once.
        if let Some(e) = a.endless.take_if(|e| e.queue_id == a.queue_id) {
            return Some(Wanted::Ask(client, Ask::More { playlist: e.playlist, token: e.token }, a.queue_id));
        }
        // A track's radio, once the queue is down to its last track.
        let current = a.queue.current()?.video_id()?.to_owned();
        if !a.queue.needs_more() || a.radio_for.as_deref() == Some(&*current) {
            return None;
        }
        a.radio_for = Some(current.as_str().into());
        Some(Wanted::Ask(client, Ask::Radio(current), a.queue_id))
    })
    .flatten();
    let (client, ask, queue_id) = match wanted {
        Some(Wanted::ListPart(next)) => return fetch_more(More::List, next),
        Some(Wanted::Ask(client, ask, queue_id)) => (client, ask, queue_id),
        None => return,
    };
    let job = ask.clone();
    net::run(
        net::Pool::Api,
        move || {
            match &job {
                Ask::More { playlist, token } => client.next_continuation(playlist, token),
                Ask::Radio(id) => client.next(id, None),
            }
            .and_then(|j| parse::up_next(&j))
        },
        move |result: kilo_core::Result<UpNext>| {
            if !with(|a| a.queue_id == queue_id).unwrap_or(false) {
                return; // another queue started meanwhile
            }
            match (result, ask) {
                (Ok(up), ask) => {
                    let playlist = match ask {
                        Ask::More { playlist, .. } => playlist,
                        Ask::Radio(id) => Client::radio_id(&id).into(),
                    };
                    with(|a| {
                        a.queue.extend(up.entries);
                        a.endless = up.continuation.map(|token| Endless { queue_id, playlist, token });
                    });
                    carry_on();
                }
                (Err(kilo_core::Error::SignedOut), _) => session::session_ended(),
                // Asked again later (the next track change, or end).
                (Err(_), Ask::More { playlist, token }) => {
                    with(|a| a.endless = Some(Endless { queue_id, playlist, token }));
                }
                (Err(_), Ask::Radio(_)) => {
                    with(|a| a.radio_for = None);
                }
            }
        },
    );
}
