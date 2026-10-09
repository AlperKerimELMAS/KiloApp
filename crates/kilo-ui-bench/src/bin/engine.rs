//! Bench: does one Auto Layout constraint anywhere in a window pull
//! hand-placed views into the layout engine? `engine [constraint]` shows a
//! window of 200 frame-placed labels (plus, with `constraint`, one view
//! sized by a constraint elsewhere), then sleeps so `heap` can count
//! `NSAutoresizingMaskLayoutConstraint`s.

fn main() {
    use objc2::MainThreadMarker;
    use objc2::MainThreadOnly;
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSTextField, NSView, NSWindow, NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

    let mtm = MainThreadMarker::new().unwrap();
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    // SAFETY: standard NSWindow initializer on the main thread.
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(100.0, 100.0), NSSize::new(800.0, 600.0)),
            NSWindowStyleMask::Titled | NSWindowStyleMask::Resizable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    let root = NSView::new(mtm);
    window.setContentView(Some(&root));
    let page = NSView::new(mtm);
    page.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(800.0, 600.0)));
    root.addSubview(&page);
    for i in 0..200 {
        let l = NSTextField::labelWithString(&NSString::from_str(&format!("Label {i}")), mtm);
        l.setFrame(NSRect::new(NSPoint::new((i % 4) as f64 * 200.0, (i / 4) as f64 * 12.0), NSSize::new(190.0, 12.0)));
        page.addSubview(&l);
    }
    if std::env::args().nth(1).as_deref() == Some("constraint") {
        let v = NSView::new(mtm);
        root.addSubview(&v);
        v.setTranslatesAutoresizingMaskIntoConstraints(false);
        v.widthAnchor().constraintEqualToConstant(10.0).setActive(true);
        v.heightAnchor().constraintEqualToConstant(10.0).setActive(true);
    }
    window.orderFrontRegardless();
    window.displayIfNeeded();
    let when = dispatch2::DispatchTime::NOW.time(30_000_000_000);
    let _ = dispatch2::DispatchQueue::main().after(when, || std::process::exit(0));
    app.run();
}
