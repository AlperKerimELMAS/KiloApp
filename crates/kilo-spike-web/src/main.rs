//! Spike: what does YouTube Music's own player cost when it runs in a hidden
//! system web view?
//!
//! ```text
//! kilo-spike-web login                         sign in once (visible window)
//! kilo-spike-web play <videoId> [--seconds N] [--lean] [--window]
//! ```
//!
//! `play` measures its own process plus WebKit's helper processes with
//! kilo-probe: while playing, while paused, and after the web view is torn
//! down. Playback only starts on a signed-in account (Premium-only rule).

#[cfg(target_os = "macos")]
mod mac;

#[cfg(target_os = "macos")]
fn main() {
    mac::main();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("kilo-spike-web: macOS only for now; WebView2 and WebKitGTK spikes come next");
    std::process::exit(2);
}
