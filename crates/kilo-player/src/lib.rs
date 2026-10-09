//! Playback through YouTube's own player, in a helper process.
//!
//! The main app never loads a web engine. It spawns itself in helper mode
//! (`--player-helper`), talks to it with one-line text messages over
//! stdin/stdout, and kills it after a while paused so every byte the web
//! engine used goes back to the system.
//!
//! The helper hosts a hidden system web view running YouTube's mobile web
//! player. Kilo never fetches, captures or decodes media itself.

mod helper;
pub mod host;
pub mod protocol;

/// Runs the helper side. Never returns.
pub fn run_helper() -> ! {
    helper::run()
}
