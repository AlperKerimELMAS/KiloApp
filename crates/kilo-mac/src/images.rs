//! Thumbnails. Each is downloaded once (kept on disk, not in RAM), decoded on
//! a worker thread by ImageIO at exactly the pixel size it's drawn at, into
//! an IOSurface that Core Animation shows without a copy, and shared by
//! every view that shows it. Images nothing shows any more stay cached for
//! scrolling back, as purgeable memory: it doesn't count toward Kilo's
//! footprint, and macOS takes it back whenever it needs to (then the image
//! is simply decoded again).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::OnceLock;

use kilo_core::http::Http;
use kilo_core::image;
use objc2::runtime::AnyObject;
use objc2_core_foundation::{CFBoolean, CFData, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGColorSpace, CGContext, CGImage, kCGColorSpaceSRGB};
use objc2_image_io::{
    CGImageSource, kCGImageSourceCreateThumbnailFromImageAlways, kCGImageSourceCreateThumbnailWithTransform,
    kCGImageSourceShouldCacheImmediately, kCGImageSourceThumbnailMaxPixelSize,
};
use objc2_io_surface::{
    IOSurfaceLockOptions, IOSurfaceRef, kIOSurfaceBytesPerElement, kIOSurfaceBytesPerRow, kIOSurfaceColorSpace, kIOSurfaceHeight,
    kIOSurfacePixelFormat, kIOSurfaceWidth,
};

use crate::net::{self, Pool};
use crate::paths;

/// Decoded images kept beyond what views are showing (purgeable).
const MEMORY_BUDGET: usize = 16 * 1024 * 1024;
/// Disk cache: trimmed to `DISK_LOW` when it grows past `DISK_HIGH`.
const DISK_HIGH: u64 = 64 * 1024 * 1024;
const DISK_LOW: u64 = 48 * 1024 * 1024;

/// A decoded thumbnail. It lives in an IOSurface because Core Animation can
/// show one straight from our memory: a `CGImage` as layer contents gets
/// copied, which measured as every image on screen costing twice its size.
#[derive(Clone)]
pub struct Image(CFRetained<IOSurfaceRef>);

// SAFETY: IOSurfaces are made to be shared between threads and processes.
// Each is filled on one worker thread before anything else sees it; after
// that it's only retained, released and handed to Core Animation.
unsafe impl Send for Image {}

impl Image {
    /// What to set as a layer's `contents`.
    pub fn contents(&self) -> &AnyObject {
        let cf: &CFType = &self.0;
        cf.as_ref()
    }

    fn bytes(&self) -> usize {
        self.0.alloc_size()
    }

    /// Marks the pixels purgeable (or not). Returns false if macOS already
    /// took them.
    fn set_volatile(&self, volatile: bool) -> bool {
        let mut old = 0;
        // SAFETY: `old` is a valid out-pointer; 1 is kIOSurfacePurgeableVolatile,
        // 0 kIOSurfacePurgeableNonVolatile.
        let ok = unsafe { self.0.set_purgeable(u32::from(volatile), &mut old) } == 0;
        // 2 is kIOSurfacePurgeableEmpty.
        ok && old != 2
    }

    /// Only the cache holds it: no layer shows it.
    fn unused(&self) -> bool {
        self.0.retain_count() == 1
    }
}

type Waiter = Box<dyn FnOnce(Option<Image>)>;

struct Cached {
    image: Image,
    bytes: usize,
    used: u64,
    volatile: bool,
}

#[derive(Default)]
struct Cache {
    images: HashMap<String, Cached>,
    bytes: usize,
    tick: u64,
    in_flight: HashMap<String, Vec<Waiter>>,
}

impl Cache {
    fn get(&mut self, url: &str) -> Option<Image> {
        self.tick += 1;
        let c = self.images.get_mut(url)?;
        if c.volatile {
            c.volatile = false;
            if !c.image.set_volatile(false) {
                // macOS took the pixels: decode again.
                self.remove(url);
                return None;
            }
        }
        c.used = self.tick;
        Some(c.image.clone())
    }

    fn insert(&mut self, url: String, image: Image) {
        let bytes = image.bytes();
        self.tick += 1;
        self.remove(&url);
        self.images.insert(url, Cached { image, bytes, used: self.tick, volatile: false });
        self.bytes += bytes;
        while self.bytes > MEMORY_BUDGET {
            let Some(oldest) = self.images.iter().min_by_key(|(_, c)| c.used).map(|(k, _)| k.clone()) else {
                break;
            };
            self.remove(&oldest);
        }
    }

    fn remove(&mut self, url: &str) {
        if let Some(c) = self.images.remove(url) {
            self.bytes -= c.bytes;
        }
    }
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
    static SWEEP_PENDING: Cell<bool> = const { Cell::new(false) };
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
    let letterboxed = url.contains("i.ytimg.com/");
    net::run(
        Pool::Images,
        move || fetch(&url).and_then(|bytes| decode(&bytes, px, letterboxed)),
        move |img| {
            let waiters = CACHE.with_borrow_mut(|c| {
                if let Some(img) = &img {
                    c.insert(key.clone(), img.clone());
                }
                c.in_flight.remove(&key).unwrap_or_default()
            });
            for w in waiters {
                w(img.clone());
            }
        },
    );
}

/// Soon, makes the cached images nothing shows purgeable. Call whenever
/// views may have let go of images (after scrolling, on a new page). It
/// waits a moment so Core Animation has committed the change.
pub fn sweep_soon() {
    if SWEEP_PENDING.replace(true) {
        return;
    }
    let when = dispatch2::DispatchTime::NOW.time(500_000_000);
    let _ = dispatch2::DispatchQueue::main().after(when, || {
        SWEEP_PENDING.set(false);
        CACHE.with_borrow_mut(|cache| {
            for c in cache.images.values_mut() {
                if !c.volatile && c.image.unused() {
                    c.volatile = c.image.set_volatile(true);
                }
            }
        });
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

/// The image at `url`, from the disk cache or the network. Only images from
/// Google's servers, in a thumbnail format, get past here
/// (`kilo_core::image`), so ImageIO never decodes anything else.
fn fetch(url: &str) -> Option<Vec<u8>> {
    if !image::trusted_url(url) {
        crate::debug::trace(|| format!("images: refused {url}"));
        return None;
    }
    let file = cache_file(url);
    if let Ok(bytes) = std::fs::read(&file)
        && image::known_format(&bytes)
    {
        return Some(bytes);
    }
    let get = |url: &str| http().get(url, &[]).ok().filter(|b| image::known_format(b));
    let bytes = get(url).or_else(|| {
        // Not every video has WebP thumbnails; the JPEG always exists.
        get(&format!("{}.jpg", url.strip_suffix(".webp")?.replacen("/vi_webp/", "/vi/", 1)))
    })?;
    if std::fs::create_dir_all(paths::image_cache()).is_ok() {
        let _ = std::fs::write(&file, &bytes);
    }
    Some(bytes)
}

/// Decodes to at most `px` pixels on the longer side. `letterboxed` images
/// may be 4:3 frames around a 16:9 picture; only the picture is kept.
fn decode(bytes: &[u8], px: u32, letterboxed: bool) -> Option<Image> {
    let image = thumbnail(bytes, px)?;
    let (w, h) = (CGImage::width(Some(&image)), CGImage::height(Some(&image)));
    // YouTube's larger video thumbnails are 4:3, with the 16:9 picture
    // between black bars. Keeping just the picture saves a quarter of the
    // pixels, and square art views no longer show the bars.
    let band = if letterboxed && w > 0 && (h * 4).abs_diff(w * 3) <= 8 { w * 9 / 16 } else { h };
    let surface = surface(w, band)?;
    // SAFETY: kCGColorSpaceSRGB is a constant string.
    let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))?;
    // SAFETY: the surface is locked while CoreGraphics draws into its
    // memory, which is `bytes_per_row` × `band` bytes as created; the
    // context is gone before it's unlocked.
    unsafe {
        if surface.lock(IOSurfaceLockOptions(0), std::ptr::null_mut()) != 0 {
            return None;
        }
        if let Some(context) = bitmap_context(Some(surface.base_address()), w, band, surface.bytes_per_row(), &space, PREMULTIPLIED_BGRA) {
            // Centered: the bars fall outside the surface.
            let y = -((h - band) as f64) / 2.0;
            CGContext::draw_image(Some(&context), CGRect::new(CGPoint::new(0.0, y), CGSize::new(w as f64, h as f64)), Some(&image));
        }
        surface.unlock(IOSurfaceLockOptions(0), std::ptr::null_mut());
        if let Some(list) = space.property_list() {
            surface.set_value(kIOSurfaceColorSpace, &list);
        }
    }
    Some(Image(surface))
}

/// A `w`×`h` BGRA surface.
fn surface(w: usize, h: usize) -> Option<CFRetained<IOSurfaceRef>> {
    // SAFETY: the IOSurface property keys are constant strings, and every
    // value is a CFNumber, as IOSurfaceCreate expects.
    unsafe {
        let row = IOSurfaceRef::align_property(kIOSurfaceBytesPerRow, w * 4);
        let numbers = [w, h, 4, row, 0x4247_5241 /* 'BGRA' */].map(|n| CFNumber::new_i64(n as i64));
        let keys: [&CFString; 5] =
            [kIOSurfaceWidth, kIOSurfaceHeight, kIOSurfaceBytesPerElement, kIOSurfaceBytesPerRow, kIOSurfacePixelFormat];
        let values: [&CFType; 5] = [&numbers[0], &numbers[1], &numbers[2], &numbers[3], &numbers[4]];
        IOSurfaceRef::new(CFDictionary::from_slices(&keys, &values).as_opaque())
    }
}

fn thumbnail(bytes: &[u8], px: u32) -> Option<CFRetained<CGImage>> {
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

/// kCGImageAlphaPremultipliedFirst | kCGBitmapByteOrder32Little.
pub const PREMULTIPLIED_BGRA: u32 = 2 | (2 << 12);

unsafe extern "C-unwind" {
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: Option<&CGColorSpace>,
        bitmap_info: u32,
    ) -> Option<NonNull<CGContext>>;
}

/// An 8-bit-per-channel bitmap context drawing into `data` (rows of
/// `bytes_per_row`), or into pixels CoreGraphics allocates when `data` is
/// `None` (`bytes_per_row` 0 lets it choose).
///
/// # Safety
///
/// `data`, if given, must be writable for `bytes_per_row` × `height` bytes
/// for as long as the context is used.
pub unsafe fn bitmap_context(
    data: Option<NonNull<c_void>>,
    width: usize,
    height: usize,
    bytes_per_row: usize,
    space: &CGColorSpace,
    info: u32,
) -> Option<CFRetained<CGContext>> {
    let data = data.map_or(std::ptr::null_mut(), NonNull::as_ptr);
    // SAFETY: the caller vouches for `data`; the returned context is +1,
    // which `from_raw` takes over.
    unsafe {
        let context = CGBitmapContextCreate(data, width, height, 8, bytes_per_row, Some(space), info)?;
        Some(CFRetained::from_raw(context))
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
