//! Dev tool: `parse_check <browse|search|next> FILE` prints what Kilo would show.

use kilo_core::model::{Section, Target};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let data = std::fs::read(&args[1]).expect("file");
    let t = std::time::Instant::now();
    if args[0] == "next" {
        let q = kilo_core::parse::up_next(&data).expect("parse");
        println!("{} queue entries in {:?}", q.len(), t.elapsed());
        for e in q.iter().take(3) {
            println!("  {} — {} [{}] {:?}", e.title, e.subtitle, e.duration, e.video_id());
        }
        return;
    }
    let page = if args[0] == "search" { kilo_core::parse::search(&data) } else { kilo_core::parse::browse(&data) }.expect("parse");
    println!("parsed {} KB in {:?}", data.len() / 1024, t.elapsed());
    if let Some(h) = &page.header {
        println!(
            "HEADER {:?} / {:?} / {:?} thumb={}",
            h.title,
            h.subtitle,
            h.detail,
            h.thumb.as_ref().map(|t| t.sized(400)).unwrap_or_default()
        );
    }
    for s in &page.sections {
        let kind = match s {
            Section::Cards { .. } => "cards",
            Section::List { .. } => "list",
            Section::Text { .. } => "text",
        };
        println!("[{kind}] {:?} ({} entries)", s.title(), s.entries().len());
        for e in s.entries().iter().take(2) {
            let t = match &e.target {
                Target::Play { video_id, .. } => format!("play {video_id}"),
                Target::PlayPlaylist { playlist_id } => format!("play-list {playlist_id}"),
                Target::Browse { id, kind, .. } => format!("{kind:?} {id}"),
                Target::None => "none".into(),
            };
            println!("    {:?} — {:?} [{}] → {t}{}", e.title, e.subtitle, e.duration, if e.thumb.is_some() { " 🖼" } else { "" });
        }
    }
    println!("continuation: {}", page.continuation.is_some());
}
