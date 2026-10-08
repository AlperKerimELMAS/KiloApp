//! Pages are laid out by hand, without Auto Layout, and only what's near the
//! screen exists as views: each header, shelf, row or text block is created
//! when it scrolls within half a screen of view and destroyed when it
//! leaves, and so is each card of a shelf as the shelf scrolls sideways.
//! Thumbnails load only while visible. Memory follows the screen, not the
//! length of the page.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use kilo_core::model::{Entry, Header, Page, PageKind, Section, Target};
use objc2::rc::Retained;
use objc2::{MainThreadMarker, sel};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSLayoutAttribute, NSScrollElasticity, NSScrollView, NSTextAlignment, NSView};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use super::{
    FlippedView, HOVER_PLAY, ItemView, ShelfScrollView, Weight, circle_button, fill, frame_label, image_layer_view, image_view, label,
    paragraph, size, square_px, stack, theme,
};
use crate::images;
use crate::strings::{S, t};

const SIDE: f64 = 28.0;
const TOP: f64 = 4.0;
const BOTTOM: f64 = 32.0;
const GAP: f64 = 28.0;
const HEADER: f64 = 220.0;
const TITLE: f64 = 40.0;
/// Card art, square or 16:9.
const ART: f64 = 164.0;
const ART_WIDE: f64 = 292.0;
/// Around a card's art and text, inside its hover highlight.
const PAD: f64 = 10.0;
const CARD_GAP: f64 = 4.0;
const SHELF: f64 = PAD + ART + 8.0 + 18.0 + 2.0 + 17.0 + PAD;
const PILLS: f64 = 44.0;
const ROW: f64 = 56.0;
const ROW_THUMB: f64 = 40.0;
/// Grids ("Quick picks"): columns of rows, this wide, this many rows tall.
const GRID_COLUMN: f64 = 360.0;
const GRID_ROWS: usize = 4;
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
    shelf: Option<Shelf>,
}

/// A shelf on screen, of cards or of grid rows: its items exist only near
/// the visible part.
struct Shelf {
    si: usize,
    grid: bool,
    scroll: Retained<ShelfScrollView>,
    document: Retained<FlippedView>,
    /// Every item's entry, and where it goes.
    items: Vec<ShelfItem>,
    /// The items that exist, by index into `items`.
    live: HashMap<usize, (Retained<NSView>, Vec<Slot>)>,
}

struct ShelfItem {
    ei: usize,
    x: f64,
    y: f64,
    w: f64,
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

/// Cards that play straight from their hover button: tracks, mixes, albums
/// and playlists.
fn playable(e: &Entry) -> bool {
    matches!(
        e.target,
        Target::Play { .. } | Target::PlayPlaylist { .. } | Target::Browse { kind: PageKind::Album | PageKind::Playlist, .. }
    )
}

fn art_width(e: &Entry) -> f64 {
    if e.thumb.as_ref().is_some_and(|t| t.wide) { ART_WIDE } else { ART }
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
        root.setHorizontalScrollElasticity(NSScrollElasticity::None);
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
                Section::Grid { .. } => push(Kind::Shelf(si), entries.min(GRID_ROWS) as f64 * ROW, &mut y),
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

    /// Developer switch: the page's first sideways shelf, if it exists.
    pub fn first_shelf(&self) -> Option<Retained<ShelfScrollView>> {
        let live = self.live.borrow();
        live.iter().filter_map(|(i, l)| Some((*i, l.shelf.as_ref()?))).min_by_key(|(i, _)| *i).map(|(_, s)| s.scroll.clone())
    }

    /// How far down the page is scrolled, in points.
    pub fn scrolled(&self) -> f64 {
        self.root.contentView().bounds().origin.y
    }

    /// Scrolls by `points`, plus `screens` screenfuls (less a little, so a
    /// line carries over).
    pub fn scroll_by(&self, points: f64, screens: f64) {
        let clip = self.root.contentView().bounds();
        let screen = (clip.size.height - 40.0).max(0.0);
        self.scroll_to(Some(clip.origin.y + points + screens * screen));
    }

    /// Scrolls to `y` points from the top, or to the end.
    pub fn scroll_to(&self, y: Option<f64>) {
        let clip = self.root.contentView();
        let end = (self.document.frame().size.height - clip.bounds().size.height).max(0.0);
        clip.scrollToPoint(NSPoint::new(0.0, y.unwrap_or(end).clamp(0.0, end)));
        self.root.reflectScrolledClipView(&clip);
    }

    /// Creates views for blocks (and shelf cards) near the visible area,
    /// destroys the rest, then loads or releases thumbnails. Called after
    /// scrolling and resizing.
    pub fn refresh(&self) {
        let clip = self.root.contentView().bounds();
        let width = clip.size.width;
        if width <= 0.0 {
            return;
        }
        let content = width - 2.0 * SIDE;
        let mut live = self.live.borrow_mut();
        if (width - self.width.get()).abs() > 0.5 {
            // New width (a resize, the sidebar): blocks keep their heights,
            // so what's live is only widened or narrowed, and its parts
            // follow by their autoresizing masks. Rebuilding instead cost
            // 3.9 ms per change, on every frame of a resize.
            self.width.set(width);
            let doc = self.document.frame();
            self.document.setFrame(NSRect::new(doc.origin, NSSize::new(width, doc.size.height)));
            for (i, l) in live.iter() {
                l.view.setFrameSize(NSSize::new(content, self.blocks[*i].h));
            }
        }
        let margin = clip.size.height / 2.0;
        let (lo, hi) = (clip.origin.y - margin, clip.origin.y + clip.size.height + margin);
        for (i, b) in self.blocks.iter().enumerate() {
            let near = b.y + b.h >= lo && b.y <= hi;
            if near && !live.contains_key(&i) {
                let mut slots = Vec::new();
                let (view, shelf) = self.make(b, content, &mut slots);
                view.setFrame(NSRect::new(NSPoint::new(SIDE, b.y), NSSize::new(content, b.h)));
                self.document.addSubview(&view);
                if b.kind == Kind::Header {
                    // The header's stacks use Auto Layout: lay them out now,
                    // or its art has no size yet and seems off screen.
                    view.layoutSubtreeIfNeeded();
                }
                live.insert(i, Live { view, slots, shelf });
            } else if !near && let Some(gone) = live.remove(&i) {
                gone.view.removeFromSuperview();
            }
        }
        for l in live.values_mut() {
            if let Some(shelf) = &mut l.shelf {
                self.refresh_shelf(shelf);
                for (_, slots) in shelf.live.values() {
                    slots.iter().for_each(refresh_slot);
                }
            }
            l.slots.iter().for_each(refresh_slot);
        }
    }

    /// Creates the items of `shelf` near its visible part, destroys the rest.
    fn refresh_shelf(&self, shelf: &mut Shelf) {
        let clip = shelf.scroll.contentView().bounds();
        let margin = clip.size.width / 2.0;
        let (lo, hi) = (clip.origin.x - margin, clip.origin.x + clip.size.width + margin);
        for (i, item) in shelf.items.iter().enumerate() {
            let near = item.x + item.w >= lo && item.x <= hi;
            if near && !shelf.live.contains_key(&i) {
                let mut slots = Vec::new();
                let view = if shelf.grid {
                    let row = self.row(shelf.si, item.ei, item.w, &mut slots);
                    row.setFrameSize(NSSize::new(item.w, ROW));
                    row
                } else {
                    self.card(shelf.si, item.ei, &mut slots)
                };
                view.setFrameOrigin(NSPoint::new(item.x, item.y));
                shelf.document.addSubview(&view);
                shelf.live.insert(i, (view, slots));
            } else if !near && let Some((gone, _)) = shelf.live.remove(&i) {
                gone.removeFromSuperview();
            }
        }
    }

    fn make(&self, b: &Block, width: f64, slots: &mut Vec<Slot>) -> (Retained<NSView>, Option<Shelf>) {
        let mtm = self.mtm;
        let view = match b.kind {
            Kind::Header => {
                let header = self.page.header.as_ref().expect("header block implies a header");
                let playable = self.page.sections.iter().any(|s| matches!(s, Section::List { .. }));
                header_view(header, playable, self.scale, slots, mtm)
            }
            Kind::Title(si) => {
                let l = frame_label(self.page.sections[si].title(), 22.0, Weight::Bold, false, mtm);
                l.setFrame(NSRect::new(NSPoint::new(PAD, 6.0), NSSize::new(width - PAD, 28.0)));
                l.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
                let holder = FlippedView::new(mtm);
                holder.addSubview(&l);
                Retained::into_super(holder)
            }
            Kind::Shelf(si) => match &self.page.sections[si] {
                Section::Cards { entries, .. } if entries.iter().all(|e| e.thumb.is_none()) => self.pills(si),
                section => {
                    let (view, shelf) = self.shelf(si, matches!(section, Section::Grid { .. }), b.h);
                    return (view, Some(shelf));
                }
            },
            Kind::Row(si, ei) => self.row(si, ei, width, slots),
            Kind::Text(si) => {
                let Section::Text { body, .. } = &self.page.sections[si] else { unreachable!() };
                let p = paragraph(body, 13.0, true, 6, width - PAD, mtm);
                p.setFrame(NSRect::new(NSPoint::new(PAD, 0.0), NSSize::new(width - PAD, TEXT)));
                p.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
                let holder = FlippedView::new(mtm);
                holder.addSubview(&p);
                Retained::into_super(holder)
            }
        };
        (view, None)
    }

    /// A horizontally scrolling shelf of cards, or of rows in columns (a
    /// grid), which `refresh_shelf` fills.
    fn shelf(&self, si: usize, grid: bool, h: f64) -> (Retained<NSView>, Shelf) {
        let mtm = self.mtm;
        let document = FlippedView::new(mtm);
        let mut items = Vec::new();
        let mut x = 0.0;
        for (ei, e) in self.page.sections[si].entries().iter().enumerate() {
            if self.ids[si][ei] == u32::MAX {
                continue;
            }
            if grid {
                let n = items.len();
                let column = (n / GRID_ROWS) as f64 * (GRID_COLUMN + CARD_GAP);
                items.push(ShelfItem { ei, x: column, y: (n % GRID_ROWS) as f64 * ROW, w: GRID_COLUMN });
                x = column + GRID_COLUMN + CARD_GAP;
            } else {
                let w = art_width(e) + 2.0 * PAD;
                items.push(ShelfItem { ei, x, y: 0.0, w });
                x += w + CARD_GAP;
            }
        }
        document.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new((x - CARD_GAP).max(0.0), h)));
        let scroll = ShelfScrollView::new(mtm);
        scroll.setDocumentView(Some(&document));
        scroll.contentView().setPostsBoundsChangedNotifications(true);
        let shelf = Shelf { si, grid, scroll: scroll.clone(), document, items, live: HashMap::new() };
        (Retained::into_super(Retained::into_super(scroll)), shelf)
    }

    /// One card: art, title, subtitle, and a play button on hover.
    fn card(&self, si: usize, ei: usize, slots: &mut Vec<Slot>) -> Retained<NSView> {
        let mtm = self.mtm;
        let e = &self.page.sections[si].entries()[ei];
        let w = art_width(e);
        let card = ItemView::new(self.ids[si][ei], 8.0, mtm);
        card.setFrameSize(NSSize::new(w + 2.0 * PAD, SHELF));
        let art = image_layer_view(if is_artist(e) { ART / 2.0 } else { 6.0 }, mtm);
        art.setFrame(NSRect::new(NSPoint::new(PAD, PAD), NSSize::new(w, ART)));
        if let Some(t) = &e.thumb {
            let px = (w * self.scale) as u32;
            slots.push(Slot { view: art.clone(), url: t.sized(px), px, state: Rc::new(Cell::new(0)) });
        }
        let title = frame_label(&e.title, 14.0, Weight::Semibold, false, mtm);
        title.setFrame(NSRect::new(NSPoint::new(PAD, PAD + ART + 8.0), NSSize::new(w, 18.0)));
        let subtitle = frame_label(&e.subtitle, 13.0, Weight::Regular, true, mtm);
        subtitle.setFrame(NSRect::new(NSPoint::new(PAD, PAD + ART + 28.0), NSSize::new(w, 17.0)));
        card.addSubview(&art);
        card.addSubview(&title);
        card.addSubview(&subtitle);
        if playable(e) {
            card.set_play_at(NSPoint::new(PAD + w - HOVER_PLAY - 8.0, PAD + ART - HOVER_PLAY - 8.0));
        }
        Retained::into_super(card)
    }

    /// Mood, genre and navigation buttons: a row of pills.
    fn pills(&self, si: usize) -> Retained<NSView> {
        let mtm = self.mtm;
        let p = theme::palette();
        let document = FlippedView::new(mtm);
        let mut x = PAD;
        for (ei, e) in self.page.sections[si].entries().iter().enumerate() {
            let id = self.ids[si][ei];
            if id == u32::MAX {
                continue;
            }
            let pill = ItemView::new(id, 18.0, mtm);
            let text = frame_label(&e.title, 14.0, Weight::Medium, false, mtm);
            let tw = text.cell().map_or(120.0, |c| c.cellSize().width).ceil().min(260.0);
            text.setFrame(NSRect::new(NSPoint::new(16.0, 9.0), NSSize::new(tw, 18.0)));
            pill.addSubview(&text);
            pill.setFrame(NSRect::new(NSPoint::new(x, 4.0), NSSize::new(tw + 32.0, 36.0)));
            // A resting fill under the hover one.
            let rest = NSView::new(mtm);
            fill(&rest, Some(&p.raised), 18.0);
            rest.setFrame(pill.frame());
            document.addSubview(&rest);
            document.addSubview(&pill);
            x += tw + 32.0 + 8.0;
        }
        document.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(x, PILLS)));
        let scroll = ShelfScrollView::new(mtm);
        scroll.setDocumentView(Some(&document));
        Retained::into_super(Retained::into_super(scroll))
    }

    /// One row: thumbnail, title, subtitle, duration.
    fn row(&self, si: usize, ei: usize, width: f64, slots: &mut Vec<Slot>) -> Retained<NSView> {
        let mtm = self.mtm;
        let e = &self.page.sections[si].entries()[ei];
        let row = ItemView::new(self.ids[si][ei], 6.0, mtm);
        let x = 60.0;
        if let Some(t) = &e.thumb {
            let art = image_layer_view(if is_artist(e) { ROW_THUMB / 2.0 } else { 4.0 }, mtm);
            art.setFrame(NSRect::new(NSPoint::new(8.0, 8.0), NSSize::new(ROW_THUMB, ROW_THUMB)));
            let px = square_px(ROW_THUMB, self.scale, t.wide);
            slots.push(Slot { view: art.clone(), url: t.sized(px), px, state: Rc::new(Cell::new(0)) });
            row.addSubview(&art);
        } else {
            // An album's tracks come without art: their numbers instead.
            let n = frame_label(&(ei + 1).to_string(), 14.0, Weight::Regular, true, mtm);
            n.setAlignment(NSTextAlignment::Center);
            n.setFrame(NSRect::new(NSPoint::new(8.0, 19.0), NSSize::new(ROW_THUMB, 18.0)));
            row.addSubview(&n);
        }
        let right = if e.duration.is_empty() { 12.0 } else { 84.0 };
        let title = frame_label(&e.title, 14.0, Weight::Medium, false, mtm);
        title.setFrame(NSRect::new(NSPoint::new(x, 9.0), NSSize::new(width - x - right, 18.0)));
        let subtitle = frame_label(&e.subtitle, 13.0, Weight::Regular, true, mtm);
        subtitle.setFrame(NSRect::new(NSPoint::new(x, 29.0), NSSize::new(width - x - right, 17.0)));
        for l in [&title, &subtitle] {
            // When the page's width changes, the row stretches its text.
            l.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
            row.addSubview(l);
        }
        if !e.duration.is_empty() {
            let d = frame_label(&e.duration, 13.0, Weight::Regular, true, mtm);
            d.setAlignment(NSTextAlignment::Right);
            d.setFrame(NSRect::new(NSPoint::new(width - 72.0, 19.0), NSSize::new(60.0, 18.0)));
            d.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
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
    let (w, hgt) = if wide { (390.0, HEADER) } else { (HEADER, HEADER) };
    let art = image_view(w, hgt, 8.0, mtm);
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
        info.push(Retained::into_super(Retained::into_super(paragraph(&h.description, 13.0, true, 3, 560.0, mtm))));
    }
    if playable {
        let play = circle_button("play.fill", sel!(playAll:), 52.0, 20.0, true, mtm);
        size(&play, Some(52.0), Some(52.0));
        play.setToolTip(Some(&NSString::from_str(t(S::Play))));
        let shuffle = circle_button("shuffle", sel!(shuffleAll:), 40.0, 15.0, false, mtm);
        size(&shuffle, Some(40.0), Some(40.0));
        shuffle.setToolTip(Some(&NSString::from_str(t(S::Shuffle))));
        let buttons = stack(&[&play, &shuffle], false, 14.0, mtm);
        buttons.setAlignment(NSLayoutAttribute::CenterY);
        info.push(Retained::into_super(buttons));
    }
    let refs: Vec<&NSView> = info.iter().map(|v| &**v).collect();
    let text = stack(&refs, true, 8.0, mtm);
    text.setAlignment(NSLayoutAttribute::Leading);
    if playable && refs.len() >= 2 {
        // A little more room above the buttons.
        text.setCustomSpacing_afterView(16.0, refs[refs.len() - 2]);
    }
    let row = stack(&[&art, &text], false, 28.0, mtm);
    row.setAlignment(NSLayoutAttribute::CenterY);
    let holder = FlippedView::new(mtm);
    holder.addSubview(&row);
    super::pin(&row, &holder, 0.0, PAD, 0.0, 0.0);
    Retained::into_super(holder)
}

/// What shows while a page loads: its shape in placeholder gray (static,
/// so it costs nothing while it waits).
pub fn skeleton(mtm: MainThreadMarker) -> Retained<NSView> {
    let p = theme::palette();
    let holder = FlippedView::new(mtm);
    let block = |x: f64, y: f64, w: f64, h: f64, r: f64| {
        let v = NSView::new(mtm);
        fill(&v, Some(&p.placeholder), r);
        v.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(w, h)));
        holder.addSubview(&v);
    };
    let mut y = TOP + 10.0;
    for _ in 0..2 {
        block(SIDE + PAD, y, 220.0, 22.0, 6.0);
        y += TITLE;
        for i in 0..6 {
            let x = SIDE + PAD + f64::from(i) * (ART + 2.0 * PAD + CARD_GAP);
            block(x, y, ART, ART, 6.0);
            block(x, y + ART + 10.0, ART * 0.8, 12.0, 4.0);
            block(x, y + ART + 30.0, ART * 0.5, 12.0, 4.0);
        }
        y += SHELF + GAP;
    }
    Retained::into_super(holder)
}
