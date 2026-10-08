//! Dev tool: `masks <cookies.binarycookies>` fetches "up next" and account
//! answers with and without their field masks, checks that both parse the
//! same, and prints the sizes.

use kilo_core::auth::{Session, parse_binary_cookies};
use kilo_core::client::{Client, Config};
use kilo_core::http::Http;
use kilo_core::parse;

fn main() {
    println!("mask ({} bytes): {}\n", parse::next_mask().len(), parse::next_mask());
    let Some(path) = std::env::args().nth(1) else { return };
    let cookies = parse_binary_cookies(&std::fs::read(path).expect("cookie file")).expect("cookie format");
    let session = Session::from_cookies(&cookies).expect("signed in");
    let http = Http::new();
    let config = Config::fetch(&http, Some(&session)).expect("config");
    let masked = Client::new(http, session, config);
    let full = masked.clone().unmasked();
    let mix = "RDCLAK5uy_nZiG9ehz_MQoWQxY5yElsLHCcG0tv9PRg";
    for (name, a, b) in [
        ("radio (video)", full.next("dQw4w9WgXcQ", None), masked.next("dQw4w9WgXcQ", None)),
        ("radio (song)", full.next("lYBUbBu4W08", None), masked.next("lYBUbBu4W08", None)),
        ("single", full.next_single("dQw4w9WgXcQ"), masked.next_single("dQw4w9WgXcQ")),
        ("mix", full.next_playlist(mix), masked.next_playlist(mix)),
    ] {
        let (a, b) = (a.expect("full"), b.expect("masked"));
        let (qa, qb) = (parse::up_next(&a).expect("parse full").entries, parse::up_next(&b).expect("parse masked").entries);
        // Radios are shuffled on every request: compare the tracks both have.
        let shared: Vec<_> = qa.iter().filter_map(|x| qb.iter().find(|y| y.video_id() == x.video_id()).map(|y| (x, y))).collect();
        let same = !shared.is_empty() && shared.iter().all(|(x, y)| x == y);
        println!(
            "{name:<14} {:>5} KB → {:>4} KB  {} of {} tracks in both, {}",
            a.len() / 1024,
            b.len() / 1024,
            shared.len(),
            qa.len(),
            if same { "identical" } else { "DIFFERENT" }
        );
    }
    // More of the mix: the continuation's own mask.
    let first = parse::up_next(&masked.next_playlist(mix).expect("mix")).expect("parse mix");
    let token = first.continuation.expect("a mix is endless");
    let (a, b) = (full.next_continuation(mix, &token).expect("full"), masked.next_continuation(mix, &token).expect("masked"));
    let (qa, qb) = (parse::up_next(&a).expect("parse full"), parse::up_next(&b).expect("parse masked"));
    println!(
        "{:<14} {:>5} KB → {:>4} KB  {} tracks, {}, more: {}",
        "mix, more",
        a.len() / 1024,
        b.len() / 1024,
        qb.entries.len(),
        if qa.entries == qb.entries { "identical" } else { "DIFFERENT" },
        qb.continuation.is_some()
    );
    let (a, b) = (full.account().expect("full"), masked.account().expect("masked"));
    let same = parse::account(&a).expect("parse full") == parse::account(&b).expect("parse masked");
    println!("{:<14} {:>5} B → {:>4} B  {}", "account", a.len(), b.len(), if same { "identical" } else { "DIFFERENT" });
}
