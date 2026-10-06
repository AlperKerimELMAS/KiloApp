//! Thumbnails. Each is downloaded once (kept on disk, not in RAM), decoded on
//! a worker thread by ImageIO at exactly the pixel size it's drawn at, and
//! shared by every view that shows it. Beyond what's on screen, at most
//! `MEMORY_BUDGET` of decoded images is kept for scrolling back.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use kilo_core::http::Http;
use objc2_core_foundation::{CFBoolean, CFData, CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_core_graphics::CGImage;
use objc2_image_io::{
    CGImageSource, kCGImageSourceCreateThumbnailFromImageAlways, kCGImageSourceCreateThumbnailWithTransform,
    kCGImageSourceShouldCacheImmediately, kCGImageSourceThumbnailMaxPixelSize,
};

use crate::net::{self, Pool};
use crate::paths;

/// Decoded images kept in memory beyond what views are showing.
const MEMORY_BUDGET: usize = 8 * 1024 * 1024;
/// Disk cache: trimmed to `DISK_LOW` when it grows past `DISK_HIGH`.
const DISK_HIGH: u64 = 64 * 1024 * 1024;
const DISK_LOW: u64 = 48 * 1024 * 1024;

pub type Image = CFRetained<CGImage>;
type Waiter = Box<dyn FnOnce(Option<Image>)>;

#[derive(Default)]
struct Cache {
    images: HashMap<String, (Image, usize, u64)>,
    bytes: usize,
    tick: u64,
    in_flight: HashMap<String, Vec<Waiter>>,
}

impl Cache {
    fn get(&mut self, url: &str) -> Option<Image> {
        self.tick += 1;
        let tick = self.tick;
        self.images.get_mut(url).map(|(img, _, used)| {
            *used = tick;
            img.clone()
        })
    }

    fn insert(&mut self, url: String, img: Image) {
        let bytes = CGImage::bytes_per_row(Some(&img)) * CGImage::height(Some(&img));
        self.tick += 1;
        if let Some((_, old, _)) = self.images.insert(url, (img, bytes, self.tick)) {
            self.bytes -= old;
        }
        self.bytes += bytes;
        while self.bytes > MEMORY_BUDGET {
            let Some(oldest) = self.images.iter().min_by_key(|(_, (_, _, used))| *used).map(|(k, _)| k.clone()) else {
                break;
            };
            if let Some((_, b, _)) = self.images.remove(&oldest) {
                self.bytes -= b;
            }
        }
    }
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

fn http() -> &'static Http {
    static HTTP: OnceLock<Http> = OnceLock::new();
    HTTP.get_or_init(Http::new)
}

/// Delivers the image at `url` (already sized by `Thumb::sized`), decoded to
/// at most `px` pixels on its longer side, to `done` on the main thread.
pub fn load(url: String, px: u32, done: impl FnOnce(Option<Image>) + 'static) {
    if let Some(img) = CACHE.with_borrow_mut(|c| c.get(&url)) {
        return done(Some(img));
    }
    let first = CACHE.with_borrow_mut(|c| {
        let waiters = c.in_flight.entry(url.clone()).or_default();
        waiters.push(Box::new(done));
        waiters.len() == 1
    });
    if !first {
        return;
    }
    let key = url.clone();
    net::run(Pool::Images, move || fetch(&url).and_then(|bytes| decode(&bytes, px)), move |img| {
        let waiters = CACHE.with_borrow_mut(|c| {
            if let Some(img) = &img {
                c.insert(key.clone(), img.clone());
            }
            c.in_flight.remove(&key).unwrap_or_default()
        });
        for w in waiters {
            w(img.clone());
        }
    });
}

/// Drops every cached decoded image (e.g. when the window closes).
pub fn purge_memory() {
    CACHE.with_borrow_mut(|c| {
        c.images.clear();
        c.bytes = 0;
    });
}

fn cache_file(url: &str) -> PathBuf {
    // FNV-1a: a stable, dependency-free file name for a URL.
    let hash = url.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    paths::image_cache().join(format!("{hash:016x}"))
}

fn fetch(url: &str) -> Option<Vec<u8>> {
    let file = cache_file(url);
    if let Ok(bytes) = std::fs::read(&file) {
        return Some(bytes);
    }
    let bytes = http().get(url, &[]).ok()?;
    if std::fs::create_dir_all(paths::image_cache()).is_ok() {
        let _ = std::fs::write(&file, &bytes);
    }
    Some(bytes)
}

fn decode(bytes: &[u8], px: u32) -> Option<Image> {
    let data = CFData::from_bytes(bytes);
    // SAFETY: ImageIO calls with valid CF objects; the options dictionary
    // holds CFString keys and CFBoolean/CFNumber values as documented.
    unsafe {
        let source = CGImageSource::with_data(&data, None)?;
        let yes: &CFType = CFBoolean::new(true);
        let size = CFNumber::new_i32(px as i32);
        let keys: [&CFString; 4] = [
            kCGImageSourceCreateThumbnailFromImageAlways,
            kCGImageSourceThumbnailMaxPixelSize,
            kCGImageSourceShouldCacheImmediately,
            kCGImageSourceCreateThumbnailWithTransform,
        ];
        let values: [&CFType; 4] = [yes, &size, yes, yes];
        let options = CFDictionary::from_slices(&keys, &values);
        source.thumbnail_at_index(0, Some(options.as_opaque()))
    }
}

/// Keeps the on-disk cache bounded. Runs once per launch, off the main thread.
pub fn trim_disk_cache() {
    net::run(
        Pool::Images,
        || {
            let Ok(dir) = std::fs::read_dir(paths::image_cache()) else { return };
            let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = dir
                .flatten()
                .filter_map(|e| {
                    let meta = e.metadata().ok()?;
                    Some((meta.modified().ok()?, meta.len(), e.path()))
                })
                .collect();
            let mut total: u64 = files.iter().map(|f| f.1).sum();
            if total <= DISK_HIGH {
                return;
            }
            files.sort();
            for (_, len, path) in files {
                if total <= DISK_LOW {
                    break;
                }
                if std::fs::remove_file(path).is_ok() {
                    total -= len;
                }
            }
        },
        |()| {},
    );
}
