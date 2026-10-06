//! Dev tool: `dump <cookies.binarycookies> <home|explore|library|liked|browse ID|search QUERY|next VIDEO_ID>`
//! prints the raw JSON response, for checking parsers against live data.

use kilo_core::auth::{Session, parse_binary_cookies};
use kilo_core::client::{Client, Config};
use kilo_core::http::Http;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cookies = parse_binary_cookies(&std::fs::read(&args[0]).expect("cookie file")).expect("cookie format");
    let session = Session::from_cookies(&cookies).expect("signed in");
    let http = Http::new();
    let t = std::time::Instant::now();
    let config = Config::fetch(&http, Some(&session)).expect("config");
    eprintln!("config {:?} in {:?}", config, t.elapsed());
    let client = Client::new(http, session, config);
    let t = std::time::Instant::now();
    let body = match args[1].as_str() {
        "home" => client.browse("FEmusic_home", None),
        "explore" => client.browse("FEmusic_explore", None),
        "library" => client.browse("FEmusic_liked_playlists", None),
        "liked" => client.browse("VLLM", None),
        "browse" => client.browse(&args[2], None),
        "cont" => client.continuation(&args[2]),
        "search" => client.search(&args[2], None),
        "next" => client.next(&args[2], None),
        other => panic!("unknown request {other}"),
    }
    .expect("request");
    eprintln!("{} bytes in {:?}", body.len(), t.elapsed());
    std::io::Write::write_all(&mut std::io::stdout(), &body).unwrap();
}
