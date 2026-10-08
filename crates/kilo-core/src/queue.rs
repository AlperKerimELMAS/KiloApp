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

    /// Swaps the current entry for another version of it.
    pub fn replace_current(&mut self, entry: Entry) {
        if let Some(current) = self.entries.get_mut(self.index) {
            *current = entry;
        }
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
            if self.is_new(&e) {
                self.entries.push(e);
            }
        }
    }

    /// Adds entries at random places among the tracks still to come (more
    /// of a shuffled playlist arriving), skipping ones already queued.
    pub fn extend_shuffled(&mut self, more: Vec<Entry>, rng: &mut Rng) {
        for e in more {
            if self.is_new(&e) {
                // Any slot after the current track, up to the end.
                let at = match self.entries.len().checked_sub(self.index) {
                    Some(upcoming @ 1..) => self.index + 1 + rng.below(upcoming),
                    _ => self.entries.len(),
                };
                self.entries.insert(at, e);
            }
        }
    }

    fn is_new(&self, e: &Entry) -> bool {
        e.video_id().is_some() && !self.entries.iter().any(|q| q.video_id() == e.video_id())
    }
}

/// xorshift64: plenty for shuffling a playlist, and no dependency.
pub struct Rng(u64);

impl Rng {
    /// Seeded from the clock and the process id.
    pub fn new() -> Self {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
        Rng((nanos ^ u64::from(std::process::id()) ^ 0x9E37_79B9_7F4A_7C15) | 1)
    }

    /// A number in `0..n` (`n` > 0).
    pub fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }

    /// Fisher–Yates.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.below(i + 1));
        }
    }
}

impl Default for Rng {
    fn default() -> Self {
        Self::new()
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
            target: Target::Play { video_id: id.into(), playlist_id: None, music_video: false },
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

    #[test]
    fn shuffled_additions_land_after_the_current_track() {
        let mut q = Queue::from_entries(&[song("aaaaaaaaaaa"), song("bbbbbbbbbbb")], "bbbbbbbbbbb");
        let mut rng = Rng::new();
        q.extend_shuffled(vec![song("ccccccccccc"), song("ddddddddddd"), song("aaaaaaaaaaa")], &mut rng);
        assert_eq!(q.entries().len(), 4);
        assert_eq!(q.current().and_then(Entry::video_id), Some("bbbbbbbbbbb"));
        assert_eq!(q.entries()[0].video_id(), Some("aaaaaaaaaaa"));
    }

    #[test]
    fn shuffled_additions_to_an_empty_queue_dont_panic() {
        let mut q = Queue::default();
        q.extend_shuffled(vec![song("aaaaaaaaaaa"), song("bbbbbbbbbbb")], &mut Rng::new());
        assert_eq!(q.entries().len(), 2);
        assert_eq!(q.current().and_then(Entry::video_id), Some("aaaaaaaaaaa"));
    }
}
