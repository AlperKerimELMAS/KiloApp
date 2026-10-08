//! What plays next: queues started from a page, a playlist or one track; the
//! rest of a long list joining as it loads; and, once it all runs out, the
//! radio of what's playing.

use std::sync::Arc;

use kilo_core::client::Client;
use kilo_core::model::{Entry, Header, Section, Target};
use kilo_core::parse;
use kilo_core::queue::Queue;

use super::browse::fetch_more;
use super::player::play_current;
use super::with;
use crate::net;
use crate::ui::page::More;

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

/// Starts playing `queue`. With `follow`, the rest of the list it came from
/// is fetched and added as it arrives.
pub(super) fn start(queue: Queue, follow: Option<Follow>) {
    let next = with(|a| {
        a.queue = queue;
        a.queue_id += 1;
        a.radio_for = None;
        a.follow = follow.map(|f| Follow { queue_id: a.queue_id, ..f });
        a.follow.as_ref().map(|f| f.next.clone())
    })
    .flatten();
    play_current(0.0);
    if let Some(next) = next {
        fetch_more(More::List, next);
    }
}

/// "Play" / "Shuffle" on a page header. A long list starts right away with
/// what's loaded; the rest joins the queue as it arrives.
pub fn play_all(shuffle: bool) {
    let Some(Some((mut entries, header, rest))) = with(|a| {
        let page = a.page.as_ref()?;
        let list = page.sections.iter().find(|s| matches!(s, Section::List { .. }))?;
        let rest = match list {
            Section::List { continuation, .. } => continuation.clone(),
            _ => None,
        };
        let entries = list.entries().iter().filter(|e| e.video_id().is_some()).cloned().collect::<Vec<_>>();
        Some((entries, page.header.clone(), rest))
    }) else {
        return;
    };
    if shuffle {
        with(|a| a.rng.shuffle(&mut entries));
    }
    let Some(first) = entries.first().and_then(|e| e.video_id().map(str::to_owned)) else { return };
    let follow = rest.map(|next| Follow::new(next, shuffle, header.clone()));
    start(Queue::from_entries(&decorate(entries, header.as_ref()), &first), follow);
}

/// Plays a whole playlist or mix from its start.
pub(super) fn play_playlist(playlist_id: String) {
    let Some(Some((client, queue_id))) = with(|a| a.client.clone().map(|c| (c, a.queue_id))) else { return };
    net::run(
        net::Pool::Api,
        move || client.next_playlist(&playlist_id).and_then(|j| parse::up_next(&j)),
        move |result| {
            if with(|a| a.queue_id) != Some(queue_id) {
                return; // something else started playing meanwhile
            }
            if let Ok(entries) = result
                && let Some(first) = entries.first().and_then(|e| e.video_id().map(str::to_owned))
            {
                start(Queue::from_entries(&entries, &first), None);
            }
        },
    );
}

/// Plays a song by id, queued with its radio (as YouTube Music does when a
/// single song is started).
pub fn play_video(video_id: String) {
    let Some(Some((client, queue_id))) = with(|a| a.client.clone().map(|c| (c, a.queue_id))) else { return };
    net::run(
        net::Pool::Api,
        move || client.next(&video_id, None).and_then(|j| parse::up_next(&j)).map(|e| (e, video_id)),
        move |result| {
            if with(|a| a.queue_id) != Some(queue_id) {
                return; // something else started playing meanwhile
            }
            if let Ok((entries, id)) = result {
                start(Queue::from_entries(&entries, &id), None);
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
/// following that list, and asks for the part after it.
pub(super) fn feed(token: &str, entries: Vec<Entry>, next: Option<Box<str>>) {
    let fetch_next = with(|a| {
        let mut follow = a.follow.take()?;
        if follow.queue_id != a.queue_id || &*follow.next != token {
            a.follow = Some(follow);
            return None;
        }
        let entries = decorate(entries, follow.header.as_ref());
        if follow.shuffle {
            a.queue.extend_shuffled(entries, &mut a.rng);
        } else {
            a.queue.extend(entries);
        }
        // Without a next part the list is complete, and the follow ends.
        let next = next?;
        follow.next = next.clone();
        a.follow = Some(follow);
        Some(next)
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

/// When the current track is the last one, makes sure more is on its way:
/// the next part of the list the queue follows or, once that's complete,
/// the radio of what's playing, like YouTube Music.
pub(super) fn extend() {
    enum Wanted {
        /// The list's next part, by its continuation token.
        ListPart(Box<str>),
        /// The radio of this track, for queue `u64`.
        Radio(Arc<Client>, String, u64),
    }
    let wanted = with(|a| {
        if !a.queue.needs_more() {
            return None;
        }
        if let Some(f) = &a.follow {
            // Normally already on its way; asked again if fetching it failed.
            return Some(Wanted::ListPart(f.next.clone()));
        }
        let current = a.queue.current()?.video_id()?.to_owned();
        if a.radio_for.as_deref() == Some(&*current) {
            return None;
        }
        let client = a.client.clone()?;
        a.radio_for = Some(current.as_str().into());
        Some(Wanted::Radio(client, current, a.queue_id))
    })
    .flatten();
    match wanted {
        Some(Wanted::ListPart(next)) => fetch_more(More::List, next),
        Some(Wanted::Radio(client, id, queue_id)) => net::run(
            net::Pool::Api,
            move || client.next(&id, None).and_then(|j| parse::up_next(&j)).map_err(|_| id),
            move |result| match result {
                Ok(entries) => {
                    if with(|a| a.queue_id == queue_id).unwrap_or(false) {
                        with(|a| a.queue.extend(entries));
                        carry_on();
                    }
                }
                // Let a later track change or end ask again.
                Err(id) => {
                    with(|a| {
                        if a.radio_for.as_deref() == Some(&*id) {
                            a.radio_for = None;
                        }
                    });
                }
            },
        ),
        None => {}
    }
}
