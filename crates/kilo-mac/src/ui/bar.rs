//! A minimal slider: a track layer and a fill layer. AppKit's `NSSlider`
//! (Liquid Glass) starts Apple's Metal shader compiler service — 40 MB — and
//! adds ~5 MB to the app; this costs two layers.

use std::cell::Cell;

use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSEvent, NSView};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use objc2_quartz_core::{CALayer, CATransaction};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Seeks when released.
    Progress,
    /// Changes volume continuously.
    Volume,
}

pub struct BarIvars {
    kind: Kind,
    value: Cell<f64>,
    enabled: Cell<bool>,
    dragging: Cell<bool>,
    track: Retained<CALayer>,
    fill: Retained<CALayer>,
    thickness: f64,
}

define_class!(
    // SAFETY: NSView subclass; ivars are plain values and layers created in
    // `new`; overrides match AppKit's signatures.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "KiloBar"]
    #[ivars = BarIvars]
    pub struct Bar;

    impl Bar {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _e: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            // SAFETY: forwarding to NSView's implementation.
            let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
            self.relayout();
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, e: &NSEvent) {
            if self.ivars().enabled.get() {
                self.ivars().dragging.set(true);
                self.track_mouse(e);
            }
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, e: &NSEvent) {
            if self.ivars().dragging.get() {
                self.track_mouse(e);
            }
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, e: &NSEvent) {
            if self.ivars().dragging.replace(false) {
                self.track_mouse(e);
                if self.ivars().kind == Kind::Progress {
                    let v = self.ivars().value.get();
                    crate::net::later(move || crate::app::seek(v));
                }
            }
        }
    }
);

impl Bar {
    pub fn new(kind: Kind, thickness: f64, fill: &objc2_app_kit::NSColor, mtm: MainThreadMarker) -> Retained<Self> {
        let track = CALayer::new();
        track.setBackgroundColor(Some(&super::theme::palette().track.CGColor()));
        track.setCornerRadius(thickness / 2.0);
        let fill_layer = CALayer::new();
        fill_layer.setBackgroundColor(Some(&fill.CGColor()));
        fill_layer.setCornerRadius(thickness / 2.0);
        let this = Self::alloc(mtm).set_ivars(BarIvars {
            kind,
            value: Cell::new(if kind == Kind::Volume { 1.0 } else { 0.0 }),
            enabled: Cell::new(kind == Kind::Volume),
            dragging: Cell::new(false),
            track: track.clone(),
            fill: fill_layer.clone(),
            thickness,
        });
        // SAFETY: NSView's designated initializer.
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        let root = CALayer::new();
        root.addSublayer(&track);
        root.addSublayer(&fill_layer);
        view.setLayer(Some(&root));
        view.setWantsLayer(true);
        view
    }

    /// Sets the value (0–1) unless the user is dragging it.
    pub fn set_value(&self, v: f64) {
        if !self.ivars().dragging.get() {
            // Never NaN: Core Animation throws on a NaN frame.
            self.ivars().value.set(if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 });
            self.relayout();
        }
    }

    pub fn set_enabled(&self, on: bool) {
        self.ivars().enabled.set(on);
    }

    fn track_mouse(&self, e: &NSEvent) {
        let p: NSPoint = self.convertPoint_fromView(e.locationInWindow(), None);
        let w = self.bounds().size.width.max(1.0);
        let v = (p.x / w).clamp(0.0, 1.0);
        self.ivars().value.set(v);
        self.relayout();
        if self.ivars().kind == Kind::Volume {
            let volume = (v * 100.0).round() as u8;
            crate::net::later(move || crate::app::set_volume(volume));
        }
    }

    fn relayout(&self) {
        let b = self.bounds();
        let t = self.ivars().thickness;
        let y = (b.size.height - t) / 2.0;
        // No implicit animations: the bar moves once a second while playing,
        // and animating that would only cost CPU.
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        self.ivars().track.setFrame(NSRect::new(NSPoint::new(0.0, y), NSSize::new(b.size.width, t)));
        let value = self.ivars().value.get();
        self.ivars().fill.setFrame(NSRect::new(NSPoint::new(0.0, y), NSSize::new(b.size.width * value, t)));
        // Empty, its rounded ends would still show as a dot.
        self.ivars().fill.setHidden(value <= 0.0);
        CATransaction::commit();
    }
}
