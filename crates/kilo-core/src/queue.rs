//! The play queue. Kilo owns it (YouTube's player only ever plays the one
//! track it's told to), so next/previous and autoplay are instant and local.

use crate::model::Entry;

#[derive(Clone, Debug, Default)]
pub struct Queue {
    entries: Vec<Entry>,
    index: usize,
}

impl Queue {
    /// A queue of the playable entries in `entries`, starting at the entry
    /// with `start`'s video id.
    pub fn from_entries(entries: &[Entry], start: &str) -> Self {
        let entries: Vec<Entry> = entries.iter().filter(|e| e.video_id().is_some()).cloned().collect();
        let index = entries.iter().position(|e| e.video_id() == Some(start)).unwrap_or(0);
        Queue { entries, index }
    }

    pub fn current(&self) -> Option<&Entry> {
        self.entries.get(self.index)
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn advance(&mut self) -> Option<&Entry> {
        (self.index + 1 < self.entries.len()).then(|| {
            self.index += 1;
            &self.entries[self.index]
        })
    }

    pub fn back(&mut self) -> Option<&Entry> {
        (self.index > 0).then(|| {
            self.index -= 1;
            &self.entries[self.index]
        })
    }

    pub fn jump(&mut self, index: usize) -> Option<&Entry> {
        (index < self.entries.len()).then(|| {
            self.index = index;
            &self.entries[index]
        })
    }

    /// True when the current track is the last one: time to fetch more
    /// (the track's radio) so playback keeps going like YouTube Music does.
    pub fn needs_more(&self) -> bool {
        self.index + 1 >= self.entries.len()
    }

    /// Appends entries, skipping ones already queued.
    pub fn extend(&mut self, more: Vec<Entry>) {
        for e in more {
            if e.video_id().is_some() && !self.entries.iter().any(|q| q.video_id() == e.video_id()) {
                self.entries.push(e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Target;

    fn song(id: &str) -> Entry {
        Entry {
            title: id.into(),
            subtitle: "".into(),
            thumb: None,
            target: Target::Play { video_id: id.into(), playlist_id: None },
            duration: "".into(),
        }
    }

    #[test]
    fn walks_the_queue_and_dedupes() {
        let mut q = Queue::from_entries(&[song("aaaaaaaaaaa"), song("bbbbbbbbbbb")], "bbbbbbbbbbb");
        assert_eq!(q.index(), 1);
        assert!(q.needs_more());
        q.extend(vec![song("bbbbbbbbbbb"), song("ccccccccccc")]);
        assert_eq!(q.entries().len(), 3);
        assert_eq!(q.advance().and_then(Entry::video_id), Some("ccccccccccc"));
        assert!(q.advance().is_none());
        assert_eq!(q.back().and_then(Entry::video_id), Some("bbbbbbbbbbb"));
    }
}
