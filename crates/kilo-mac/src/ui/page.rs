//! Pages are laid out by hand, without Auto Layout, and only what's near the
//! screen exists as views: each header, shelf, row or text block is created
//! when it scrolls within half a screen of view and destroyed when it
//! leaves. Thumbnails load only while visible. Memory follows the screen,
//! not the length of the page.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use kilo_core::model::{Entry, Header, Page, PageKind, Section, Target};
use objc2::rc::Retained;
use objc2::{MainThreadMarker, sel};
use objc2_app_kit::{NSScrollView, NSTextAlignment, NSView};
use objc2_foundation::{NSPoint, NSRect, NSSize};

use super::{FlippedView, ItemView, Weight, frame_label, image_layer_view, image_view, label, paragraph, stack, text_button};
use crate::images;

const SIDE: f64 = 32.0;
const TOP: f64 = 24.0;
const BOTTOM: f64 = 40.0;
const GAP: f64 = 28.0;
const HEADER: f64 = 216.0;
const TITLE: f64 = 36.0;
const CARD: f64 = 160.0;
const CARD_WIDE: f64 = 284.0;
const CARD_GAP: f64 = 16.0;
const SHELF: f64 = 208.0;
const PILLS: f64 = 44.0;
const ROW: f64 = 56.0;
const ROW_THUMB: f64 = 40.0;
const TEXT: f64 = 110.0;

/// Item ids handed to clickable views map back to (section, entry) here.
pub type Items = Vec<(usize, usize)>;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Header,
    Title(usize),
    Shelf(usize),
    Row(usize, usize),
    Text(usize),
}

struct Block {
    y: f64,
    h: f64,
    kind: Kind,
}

/// A thumbnail that should show `url` while it's on screen.
struct Slot {
    view: Retained<NSView>,
    url: String,
    px: u32,
    /// 0 = empty, 1 = loading, 2 = shown.
    state: Rc<Cell<u8>>,
}

struct Live {
    view: Retained<NSView>,
    slots: Vec<Slot>,
}

pub struct PageView {
    pub root: Retained<NSScrollView>,
    document: Retained<FlippedView>,
    page: Rc<Page>,
    blocks: Vec<Block>,
    /// Per section, per entry: its item id, or `u32::MAX` if not clickable.
    ids: Vec<Vec<u32>>,
    live: RefCell<HashMap<usize, Live>>,
    width: Cell<f64>,
    scale: f64,
    mtm: MainThreadMarker,
}

fn clickable(e: &Entry) -> bool {
    !matches!(e.target, Target::None)
}

fn is_artist(e: &Entry) -> bool {
    matches!(e.target, Target::Browse { kind: PageKind::Artist, .. })
}

/// More of a page to load: its next shelves, or the next rows of a long list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum More {
    Page,
    List,
}

impl PageView {
    pub fn new(page: Rc<Page>, scale: f64, items: &mut Items, mtm: MainThreadMarker) -> Self {
        let document = FlippedView::new(mtm);
        document.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(800.0, 0.0)));
        let root = NSScrollView::new(mtm);
        root.setHasVerticalScroller(true);
        root.setAutohidesScrollers(true);
        root.setDrawsBackground(false);
        root.setDocumentView(Some(&document));
        root.contentView().setPostsBoundsChangedNotifications(true);
        let mut view = PageView {
            root,
            document,
            page: page.clone(),
            blocks: Vec::new(),
            ids: Vec::new(),
            live: RefCell::new(HashMap::new()),
            width: Cell::new(0.0),
            scale,
            mtm,
        };
        view.set_page(page, items);
        view
    }

    /// Shows `page`, typically this page with more of it loaded, keeping the
    /// scroll position. Clickable entries get fresh ids in `items`.
    pub fn set_page(&mut self, page: Rc<Page>, items: &mut Items) {
        items.clear();
        let mut ids = Vec::with_capacity(page.sections.len());
        for (si, s) in page.sections.iter().enumerate() {
            ids.push(
                s.entries()
                    .iter()
                    .enumerate()
                    .map(|(ei, e)| {
                        if clickable(e) {
                            items.push((si, ei));
                            (items.len() - 1) as u32
                        } else {
                            u32::MAX
                        }
                    })
                    .collect(),
            );
        }

        let mut blocks = Vec::new();
        let mut y = TOP;
        let mut push = |kind, h, y: &mut f64| {
            blocks.push(Block { y: *y, h, kind });
            *y += h;
        };
        if page.header.is_some() {
            push(Kind::Header, HEADER, &mut y);
            y += GAP;
        }
        for (si, s) in page.sections.iter().enumerate() {
            let entries = s.entries().iter().filter(|e| clickable(e)).count();
            if entries == 0 && !matches!(s, Section::Text { .. }) {
                continue;
            }
            if !s.title().is_empty() {
                push(Kind::Title(si), TITLE, &mut y);
            }
            match s {
                Section::Cards { entries, .. } => {
                    let h = if entries.iter().any(|e| e.thumb.is_some()) { SHELF } else { PILLS };
                    push(Kind::Shelf(si), h, &mut y);
                }
                Section::List { entries, .. } => {
                    for (ei, e) in entries.iter().enumerate() {
                        if clickable(e) {
                            push(Kind::Row(si, ei), ROW, &mut y);
                        }
                    }
                }
                Section::Text { .. } => push(Kind::Text(si), TEXT, &mut y),
            }
            y += GAP;
        }

        // Blocks that didn't move keep their views (and a shelf its sideways
        // scroll): more shelves or rows only add blocks after them.
        self.live.borrow_mut().retain(|&i, live| {
            let kept = self.blocks.get(i).zip(blocks.get(i)).is_some_and(|(a, b)| a.kind == b.kind && a.y == b.y);
            if !kept {
                live.view.removeFromSuperview();
            }
            kept
        });
        let doc = self.document.frame();
        self.document.setFrame(NSRect::new(doc.origin, NSSize::new(doc.size.width, y + BOTTOM)));
        self.page = page;
        self.blocks = blocks;
        self.ids = ids;
    }

    /// What to load next, once the end of what's loaded is within a screen
    /// of view: the rest of a long list first, then more shelves.
    pub fn wants_more(&self) -> Option<(More, Box<str>)> {
        let clip = self.root.contentView().bounds();
        let horizon = clip.origin.y + 2.0 * clip.size.height;
        for (si, s) in self.page.sections.iter().enumerate() {
            if let Section::List { continuation: Some(token), .. } = s {
                let end =
                    self.blocks.iter().filter(|b| matches!(b.kind, Kind::Row(s, _) if s == si)).map(|b| b.y + b.h).fold(0.0, f64::max);
                if end <= horizon {
                    return Some((More::List, token.clone()));
                }
            }
        }
        let token = self.page.continuation.as_ref()?;
        (self.document.frame().size.height <= horizon).then(|| (More::Page, token.clone()))
    }

    /// Scrolls to `y` points from the top, or to the end.
    pub fn scroll_to(&self, y: Option<f64>) {
        let clip = self.root.contentView();
        let end = (self.document.frame().size.height - clip.bounds().size.height).max(0.0);
        clip.scrollToPoint(NSPoint::new(0.0, y.unwrap_or(end).clamp(0.0, end)));
        self.root.reflectScrolledClipView(&clip);
    }

    /// Creates views for blocks near the visible area, destroys the rest,
    /// then loads or releases thumbnails. Called after scrolling/resizing.
    pub fn refresh(&self) {
        let clip = self.root.contentView().bounds();
        let width = clip.size.width;
        if width <= 0.0 {
            return;
        }
        if (width - self.width.get()).abs() > 0.5 {
            // New width: rebuild what's live at the new size.
            self.width.set(width);
            let doc = self.document.frame();
            self.document.setFrame(NSRect::new(doc.origin, NSSize::new(width, doc.size.height)));
            for (_, live) in self.live.borrow_mut().drain() {
                live.view.removeFromSuperview();
            }
        }
        let margin = clip.size.height / 2.0;
        let (lo, hi) = (clip.origin.y - margin, clip.origin.y + clip.size.height + margin);
        let content = width - 2.0 * SIDE;
        let mut live = self.live.borrow_mut();
        for (i, b) in self.blocks.iter().enumerate() {
            let near = b.y + b.h >= lo && b.y <= hi;
            if near && !live.contains_key(&i) {
                let mut slots = Vec::new();
                let view = self.make(b, content, &mut slots);
                view.setFrame(NSRect::new(NSPoint::new(SIDE, b.y), NSSize::new(content, b.h)));
                self.document.addSubview(&view);
                live.insert(i, Live { view, slots });
            } else if !near && let Some(gone) = live.remove(&i) {
                gone.view.removeFromSuperview();
            }
        }
        for l in live.values() {
            for slot in &l.slots {
                refresh_slot(slot);
            }
        }
    }

    fn make(&self, b: &Block, width: f64, slots: &mut Vec<Slot>) -> Retained<NSView> {
        let mtm = self.mtm;
        match b.kind {
            Kind::Header => {
                let header = self.page.header.as_ref().expect("header block implies a header");
                let playable = self.page.sections.iter().any(|s| matches!(s, Section::List { .. }));
                header_view(header, playable, self.scale, slots, mtm)
            }
            Kind::Title(si) => {
                let l = frame_label(self.page.sections[si].title(), 22.0, Weight::Bold, false, mtm);
                l.setFrame(NSRect::new(NSPoint::new(0.0, 2.0), NSSize::new(width, 28.0)));
                let holder = FlippedView::new(mtm);
                holder.addSubview(&l);
                Retained::into_super(holder)
            }
            Kind::Shelf(si) => self.shelf(si, b.h, slots),
            Kind::Row(si, ei) => self.row(si, ei, width, slots),
            Kind::Text(si) => {
                let Section::Text { body, .. } = &self.page.sections[si] else { unreachable!() };
                let p = paragraph(body, 13.0, true, 6, mtm);
                p.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(width, TEXT)));
                let holder = FlippedView::new(mtm);
                holder.addSubview(&p);
                Retained::into_super(holder)
            }
        }
    }

    /// A horizontally scrolling shelf of cards (or pills, without art).
    fn shelf(&self, si: usize, h: f64, slots: &mut Vec<Slot>) -> Retained<NSView> {
        let mtm = self.mtm;
        let document = FlippedView::new(mtm);
        let mut x = 0.0;
        for (ei, e) in self.page.sections[si].entries().iter().enumerate() {
            let id = self.ids[si][ei];
            if id == u32::MAX {
                continue;
            }
            let card = ItemView::new(id, mtm);
            if let Some(t) = &e.thumb {
                let w = if t.wide { CARD_WIDE } else { CARD };
                let art = image_layer_view(if is_artist(e) { CARD / 2.0 } else { 4.0 }, mtm);
                art.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(w, CARD)));
                let px = (w * self.scale) as u32;
                slots.push(Slot { view: art.clone(), url: t.sized(px), px, state: Rc::new(Cell::new(0)) });
                let title = frame_label(&e.title, 14.0, Weight::Medium, false, mtm);
                title.setFrame(NSRect::new(NSPoint::new(0.0, CARD + 6.0), NSSize::new(w, 18.0)));
                let subtitle = frame_label(&e.subtitle, 13.0, Weight::Regular, true, mtm);
                subtitle.setFrame(NSRect::new(NSPoint::new(0.0, CARD + 26.0), NSSize::new(w, 17.0)));
                card.addSubview(&art);
                card.addSubview(&title);
                card.addSubview(&subtitle);
                card.setFrame(NSRect::new(NSPoint::new(x, 0.0), NSSize::new(w, h)));
                x += w + CARD_GAP;
            } else {
                // Mood/genre and navigation buttons: a pill.
                let text = frame_label(&e.title, 14.0, Weight::Medium, false, mtm);
                let tw = text.cell().map_or(120.0, |c| c.cellSize().width).ceil().min(260.0);
                text.setFrame(NSRect::new(NSPoint::new(16.0, 11.0), NSSize::new(tw, 18.0)));
                card.addSubview(&text);
                card.setFrame(NSRect::new(NSPoint::new(x, 2.0), NSSize::new(tw + 32.0, 40.0)));
                if let Some(layer) = card.layer() {
                    layer.setBackgroundColor(Some(&super::placeholder().CGColor()));
                    layer.setCornerRadius(8.0);
                }
                x += tw + 32.0 + 10.0;
            }
            document.addSubview(&card);
        }
        document.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new((x - CARD_GAP).max(0.0), h)));
        let scroll = NSScrollView::new(mtm);
        scroll.setHasHorizontalScroller(false);
        scroll.setHasVerticalScroller(false);
        scroll.setDrawsBackground(false);
        scroll.setDocumentView(Some(&document));
        scroll.contentView().setPostsBoundsChangedNotifications(true);
        Retained::into_super(scroll)
    }

    /// One row: thumbnail, title, subtitle, duration.
    fn row(&self, si: usize, ei: usize, width: f64, slots: &mut Vec<Slot>) -> Retained<NSView> {
        let mtm = self.mtm;
        let e = &self.page.sections[si].entries()[ei];
        let row = ItemView::new(self.ids[si][ei], mtm);
        let mut x = 12.0;
        if let Some(t) = &e.thumb {
            let art = image_layer_view(if is_artist(e) { ROW_THUMB / 2.0 } else { 4.0 }, mtm);
            art.setFrame(NSRect::new(NSPoint::new(8.0, 8.0), NSSize::new(ROW_THUMB, ROW_THUMB)));
            let px = (ROW_THUMB * self.scale) as u32;
            slots.push(Slot { view: art.clone(), url: t.sized(px), px, state: Rc::new(Cell::new(0)) });
            row.addSubview(&art);
            x = 60.0;
        }
        let right = if e.duration.is_empty() { 12.0 } else { 84.0 };
        let title = frame_label(&e.title, 14.0, Weight::Medium, false, mtm);
        title.setFrame(NSRect::new(NSPoint::new(x, 9.0), NSSize::new(width - x - right, 18.0)));
        let subtitle = frame_label(&e.subtitle, 13.0, Weight::Regular, true, mtm);
        subtitle.setFrame(NSRect::new(NSPoint::new(x, 29.0), NSSize::new(width - x - right, 17.0)));
        row.addSubview(&title);
        row.addSubview(&subtitle);
        if !e.duration.is_empty() {
            let d = frame_label(&e.duration, 13.0, Weight::Regular, true, mtm);
            d.setAlignment(NSTextAlignment::Right);
            d.setFrame(NSRect::new(NSPoint::new(width - 72.0, 19.0), NSSize::new(60.0, 18.0)));
            row.addSubview(&d);
        }
        Retained::into_super(row)
    }
}

fn refresh_slot(slot: &Slot) {
    let r = slot.view.visibleRect();
    let visible = r.size.width > 0.0 && r.size.height > 0.0;
    match (visible, slot.state.get()) {
        (true, 0) => {
            slot.state.set(1);
            let (view, state) = (slot.view.clone(), slot.state.clone());
            images::load(slot.url.clone(), slot.px, move |img| {
                if state.get() == 1 {
                    super::set_image(&view, img.as_ref());
                    state.set(2);
                }
            });
        }
        (false, 2) => {
            // The decoded image stays in the small cache for scrolling back;
            // the view lets go of it.
            super::set_image(&slot.view, None);
            slot.state.set(0);
        }
        (false, 1) => slot.state.set(0),
        _ => {}
    }
}

fn header_view(h: &Header, playable: bool, scale: f64, slots: &mut Vec<Slot>, mtm: MainThreadMarker) -> Retained<NSView> {
    let wide = h.thumb.as_ref().is_some_and(|t| t.wide);
    let (w, hgt) = if wide { (384.0, HEADER) } else { (HEADER, HEADER) };
    let art = image_view(w, hgt, if wide { 8.0 } else { 6.0 }, mtm);
    if let Some(t) = &h.thumb {
        let px = (w * scale) as u32;
        slots.push(Slot { view: art.clone(), url: t.sized(px), px, state: Rc::new(Cell::new(0)) });
    }
    let mut info: Vec<Retained<NSView>> = vec![Retained::into_super(Retained::into_super(label(&h.title, 36.0, Weight::Bold, false, mtm)))];
    for line in [&h.subtitle, &h.detail] {
        if !line.is_empty() {
            info.push(Retained::into_super(Retained::into_super(label(line, 14.0, Weight::Regular, true, mtm))));
        }
    }
    if !h.description.is_empty() {
        info.push(Retained::into_super(Retained::into_super(paragraph(&h.description, 13.0, true, 3, mtm))));
    }
    if playable {
        let play = text_button("Play", sel!(playAll:), mtm);
        let shuffle = text_button("Shuffle", sel!(shuffleAll:), mtm);
        info.push(Retained::into_super(stack(&[&play, &shuffle], false, 8.0, mtm)));
    }
    let refs: Vec<&NSView> = info.iter().map(|v| &**v).collect();
    let text = stack(&refs, true, 8.0, mtm);
    text.setAlignment(objc2_app_kit::NSLayoutAttribute::Leading);
    let row = stack(&[&art, &text], false, 24.0, mtm);
    row.setAlignment(objc2_app_kit::NSLayoutAttribute::CenterY);
    let holder = FlippedView::new(mtm);
    holder.addSubview(&row);
    super::pin(&row, &holder, 0.0, 0.0, 0.0, 0.0);
    Retained::into_super(holder)
}
