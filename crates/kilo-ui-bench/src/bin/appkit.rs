//! Bench: the same YouTube Music-like window as the Slint bench, built with
//! native AppKit views. `BENCH_IMAGES=30 appkit` prints the footprint every
//! 2 s and quits after 10 s.

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    mac::main();
}

#[cfg(target_os = "macos")]
mod mac {
    use std::cell::Cell;
    use std::ptr::NonNull;

    use objc2::rc::Retained;
    use objc2::AnyThread;
    use objc2::MainThreadMarker;
    use objc2::MainThreadOnly;
    use objc2_app_kit::{
        NSAppearance, NSAppearanceCustomization, NSAppearanceNameDarkAqua, NSApplication, NSApplicationActivationPolicy, NSBackingStoreType,
        NSBitmapImageRep, NSColor, NSDeviceRGBColorSpace, NSFont, NSImage, NSImageScaling, NSImageView, NSScrollView,
        NSSearchField, NSStackView, NSTextField, NSUserInterfaceLayoutOrientation, NSView, NSWindow, NSWindowStyleMask,
    };
    use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString, NSTimer};

    fn label(text: &str, size: f64, bold: bool, gray: bool, mtm: MainThreadMarker) -> Retained<NSTextField> {
        let l = NSTextField::labelWithString(&NSString::from_str(text), mtm);
        let font = if bold { NSFont::boldSystemFontOfSize(size) } else { NSFont::systemFontOfSize(size) };
        l.setFont(Some(&font));
        let color = if gray { NSColor::secondaryLabelColor() } else { NSColor::labelColor() };
        l.setTextColor(Some(&color));
        l
    }

    fn gradient(px: usize, seed: usize) -> Retained<NSImage> {
        unsafe {
            let rep = NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
                NSBitmapImageRep::alloc(),
                std::ptr::null_mut(),
                px as isize,
                px as isize,
                8,
                4,
                true,
                false,
                NSDeviceRGBColorSpace,
                (px * 4) as isize,
                32,
            )
            .expect("bitmap");
            let data = std::slice::from_raw_parts_mut(rep.bitmapData(), px * px * 4);
            for (i, p) in data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let (x, y) = (i % px, i / px);
                p.copy_from_slice(&[(x * 255 / px) as u8, (y * 255 / px) as u8, (seed * 37 % 255) as u8, 255]);
            }
            let image = NSImage::initWithSize(NSImage::alloc(), NSSize::new(px as f64 / 2.0, px as f64 / 2.0));
            image.addRepresentation(&rep);
            image
        }
    }

    fn stack(views: &[Retained<NSView>], vertical: bool, spacing: f64, mtm: MainThreadMarker) -> Retained<NSStackView> {
        let refs: Vec<&NSView> = views.iter().map(|v| &**v).collect();
        let s = NSStackView::stackViewWithViews(&NSArray::from_slice(&refs), mtm);
        s.setOrientation(if vertical {
            NSUserInterfaceLayoutOrientation::Vertical
        } else {
            NSUserInterfaceLayoutOrientation::Horizontal
        });
        s.setSpacing(spacing);
        s
    }

    fn fixed(view: &NSView, w: Option<f64>, h: Option<f64>) {
        if let Some(w) = w {
            view.widthAnchor().constraintEqualToConstant(w).setActive(true);
        }
        if let Some(h) = h {
            view.heightAnchor().constraintEqualToConstant(h).setActive(true);
        }
    }

    pub fn main() {
        let mtm = MainThreadMarker::new().unwrap();
        let images: usize = std::env::var("BENCH_IMAGES").ok().and_then(|v| v.parse().ok()).unwrap_or(30);
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);

        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                NSRect::new(NSPoint::new(100.0, 100.0), NSSize::new(1280.0, 800.0)),
                NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Resizable | NSWindowStyleMask::Miniaturizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str("Kilo AppKit bench"));
        window.setAppearance(NSAppearance::appearanceNamed(unsafe { NSAppearanceNameDarkAqua }).as_deref());
        window.setBackgroundColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(0.012, 0.012, 0.012, 1.0)));

        let side: Vec<Retained<NSView>> = ["Home", "Explore", "Library"]
            .iter()
            .map(|t| Retained::into_super(Retained::into_super(label(t, 16.0, false, false, mtm))))
            .collect();
        let sidebar = stack(&side, true, 8.0, mtm);
        fixed(&sidebar, Some(240.0), None);

        let mut shelves: Vec<Retained<NSView>> = Vec::new();
        for s in 0..3 {
            let mut cards: Vec<Retained<NSView>> = Vec::new();
            for i in 0..10 {
                let iv = if s * 10 + i < images {
                    NSImageView::imageViewWithImage(&gradient(320, s * 10 + i), mtm)
                } else {
                    NSImageView::new(mtm)
                };
                iv.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
                fixed(&iv, Some(160.0), Some(160.0));
                let views: Vec<Retained<NSView>> = vec![
                    Retained::into_super(Retained::into_super(iv)),
                    Retained::into_super(Retained::into_super(label(&format!("Album title number {i}"), 14.0, false, false, mtm))),
                    Retained::into_super(Retained::into_super(label("Artist name · 2026", 13.0, false, true, mtm))),
                ];
                let card = stack(&views, true, 6.0, mtm);
                fixed(&card, Some(160.0), None);
                cards.push(Retained::into_super(card));
            }
            let row = stack(&cards, false, 16.0, mtm);
            let title = Retained::into_super(Retained::into_super(label(&format!("Shelf {s}"), 24.0, true, false, mtm)));
            shelves.push(Retained::into_super(stack(&[title, Retained::into_super(row)], true, 12.0, mtm)));
        }
        let content = stack(&shelves, true, 24.0, mtm);

        let scroll = NSScrollView::new(mtm);
        scroll.setHasVerticalScroller(true);
        scroll.setDrawsBackground(false);
        scroll.setDocumentView(Some(&content));

        let search = NSSearchField::new(mtm);
        let search_ref = search.clone();
        search.setPlaceholderString(Some(&NSString::from_str("Search songs, albums, artists")));
        fixed(&search, Some(480.0), Some(32.0));
        let bar = label("Never Gonna Give You Up · Rick Astley", 14.0, false, false, mtm);
        fixed(&bar, None, Some(72.0));

        let main_views: Vec<Retained<NSView>> = vec![
            Retained::into_super(Retained::into_super(Retained::into_super(search))),
            Retained::into_super(scroll),
            Retained::into_super(Retained::into_super(bar)),
        ];
        let main_col = stack(&main_views, true, 0.0, mtm);
        let root = stack(&[Retained::into_super(sidebar), Retained::into_super(main_col)], false, 0.0, mtm);
        let flag = |k: &str| std::env::var_os(k).is_some();
        if flag("BENCH_SYMBOLS") {
            for name in ["play.fill", "backward.fill", "forward.fill", "house.fill", "chevron.left"] {
                if let Some(img) = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None) {
                    let iv = NSImageView::imageViewWithImage(&img, mtm);
                    root.addArrangedSubview(&iv);
                }
            }
        }
        if flag("BENCH_SLIDER") {
            let s = objc2_app_kit::NSSlider::new(mtm);
            root.addArrangedSubview(&s);
        }
        if flag("BENCH_BUTTON") {
            let b = unsafe { objc2_app_kit::NSButton::buttonWithTitle_target_action(&NSString::from_str("Sign in"), None, None, mtm) };
            root.addArrangedSubview(&b);
        }
        if flag("BENCH_SPINNER") {
            let p = objc2_app_kit::NSProgressIndicator::new(mtm);
            p.setStyle(objc2_app_kit::NSProgressIndicatorStyle::Spinning);
            unsafe { p.startAnimation(None) };
            root.addArrangedSubview(&p);
        }
        if flag("BENCH_FULLSIZE") {
            window.setStyleMask(window.styleMask() | NSWindowStyleMask::FullSizeContentView);
            window.setTitlebarAppearsTransparent(true);
        }
        if flag("BENCH_MENU") {
            let bar = objc2_app_kit::NSMenu::new(mtm);
            let edit = objc2_app_kit::NSMenu::initWithTitle(objc2_app_kit::NSMenu::alloc(mtm), &NSString::from_str("Edit"));
            for (t, a, k) in [("Copy", objc2::sel!(copy:), "c"), ("Paste", objc2::sel!(paste:), "v")] {
                let item = unsafe { objc2_app_kit::NSMenuItem::initWithTitle_action_keyEquivalent(objc2_app_kit::NSMenuItem::alloc(mtm), &NSString::from_str(t), Some(a), &NSString::from_str(k)) };
                edit.addItem(&item);
            }
            let holder = objc2_app_kit::NSMenuItem::new(mtm);
            holder.setSubmenu(Some(&edit));
            bar.addItem(&holder);
            app.setMainMenu(Some(&bar));
        }
        window.setContentView(Some(&root));
        window.makeKeyAndOrderFront(None);
        if !flag("BENCH_BACKGROUND") {
            app.activate();
        }
        if flag("BENCH_FOCUS") {
            window.makeFirstResponder(Some(&*search_ref));
        }

        let ticks = Cell::new(0);
        let block = block2::RcBlock::new(move |_t: NonNull<NSTimer>| {
            ticks.set(ticks.get() + 1);
            if let Ok(s) = kilo_probe::sample(std::process::id()) {
                println!("t={:>2}s footprint {:.1} MB", ticks.get() * 2, s.footprint as f64 / 1048576.0);
            }
            if ticks.get() == 5 {
                std::process::exit(0);
            }
        });
        unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(2.0, true, &block) };
        app.run();
    }
}
