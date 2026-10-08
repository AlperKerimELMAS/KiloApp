//! Developer switches' entry points into the app (`debug` runs them): state
//! summaries, snapshots, and measurements.

use std::time::{Duration, Instant};

use kilo_core::model::{Entry, Section};

use super::{set_appearance, set_language, with, with_shell};

/// Developer switch: renders the window to `KILO_SNAPSHOT`, soon or `now`.
pub fn snapshot(now: bool) {
    with_shell(|s| if now { crate::debug::snapshot_now(&s.window) } else { crate::debug::schedule_snapshot(&s.window) });
}

/// Developer switch: picks the appearance or language as their menus do.
pub fn choose(setting: &str, value: &str) {
    let tag = match value {
        "light" | "en" => 1,
        "dark" | "tr" => 2,
        _ => 0,
    };
    match setting {
        "theme" => set_appearance(tag),
        "lang" => set_language(tag),
        _ => {}
    }
}

/// Developer switch: the volume, what has the keyboard focus, and the search
/// field's text.
pub fn describe_keys() -> String {
    with(|a| {
        let Some(s) = &a.shell else { return format!("volume {} (no window)", a.volume) };
        let focus = s.window.firstResponder().map(|r| r.class().name().to_string_lossy().into_owned()).unwrap_or_default();
        format!(
            "volume {} muted {} playing {} focus {focus} search {:?}",
            a.volume,
            a.unmute_to.is_some(),
            a.playing,
            s.search.stringValue()
        )
    })
    .unwrap_or_default()
}

/// Developer switch: changes the page's width `n` times (by resizing the
/// window) and says how long the page took to follow, on average.
pub fn bench_relayout(n: usize) -> String {
    let Some(window) = with_shell(|s| s.window.clone()) else { return "no window".into() };
    let frame = window.frame();
    let mut spent = Duration::ZERO;
    for i in 0..n {
        let width = frame.size.width - if i % 2 == 0 { 40.0 } else { 0.0 };
        window.setFrame_display(objc2_foundation::NSRect::new(frame.origin, objc2_foundation::NSSize::new(width, frame.size.height)), true);
        window.layoutIfNeeded();
        let start = Instant::now();
        with(|a| {
            if let Some(v) = &a.view {
                v.refresh();
            }
        });
        spent += start.elapsed();
    }
    window.setFrame_display(frame, true);
    format!("page relayout: {:.2} ms per width change ({n} changes)", spent.as_secs_f64() * 1000.0 / n.max(1) as f64)
}

/// Developer switch: hands `event` to the page's first shelf, and says
/// where the page and that shelf are scrolled to then.
pub fn scroll_shelf(event: &objc2_app_kit::NSEvent) -> String {
    let Some(Some((page, shelf))) = with(|a| a.view.as_ref().map(|v| (v.root.clone(), v.first_shelf()))) else { return "no page".into() };
    let Some(shelf) = shelf else { return "no shelf on screen".into() };
    shelf.scrollWheel(event);
    format!(
        "page at {:.0}, shelf at {:.0} (event dx {} dy {})",
        page.contentView().bounds().origin.y,
        shelf.contentView().bounds().origin.x,
        event.scrollingDeltaX(),
        event.scrollingDeltaY()
    )
}

/// Developer switch: a one-line summary of the page and the queue.
pub fn describe() -> String {
    with(|a| {
        let lists: Vec<String> = a
            .page
            .iter()
            .flat_map(|p| p.sections.iter())
            .map(|s| match s {
                Section::List { entries, continuation, .. } => {
                    format!("list {}{}", entries.len(), if continuation.is_some() { "+" } else { "" })
                }
                Section::Cards { entries, .. } => format!("cards {}", entries.len()),
                Section::Grid { entries, .. } => format!("grid {}", entries.len()),
                Section::Text { .. } => "text".into(),
            })
            .collect();
        let more = a.page.as_ref().is_some_and(|p| p.continuation.is_some());
        format!(
            "page [{}]{} queue {}/{} ({}) following {}",
            lists.join(", "),
            if more { " +more" } else { "" },
            a.queue.index() + 1,
            a.queue.entries().len(),
            a.queue.current().and_then(Entry::video_id).unwrap_or("-"),
            a.follow.is_some()
        )
    })
    .unwrap_or_default()
}
