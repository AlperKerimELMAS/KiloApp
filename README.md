# Kilo

A very lightweight YouTube Music desktop client. Every single kilobyte counts.

| | YouTube Music in Chrome | Kilo |
|---|---|---|
| Memory while playing | 1,088 MB | **147.5 MB** |
| Memory, window open, not playing | | **31 MB** |
| Paused for 5+ minutes | | **31 MB, 0% CPU** |
| CPU while playing (one core) | 8.9% | 8.2% |
| Home on screen after launch | | **0.15 s** |
| On disk | 1.4 GB (Chrome) | **1.4 MB** |

Measured on one Mac (Apple Silicon, macOS 27); see
[`docs/COMPARISON.md`](docs/COMPARISON.md) for how.

Kilo is an unofficial client. It isn't affiliated with, or endorsed by,
YouTube or Google. YouTube and YouTube Music are trademarks of Google LLC.

## How it works

- **Native UI.** AppKit on macOS, laid out by hand: only what's near the
  screen exists, and images are decoded at exactly the size they're drawn.
  Light and dark, following the system or your choice. Windows and Linux
  front ends are planned, over the same portable core, which already holds
  what they'll share: the words, the colors and the keyboard shortcuts.
- **In English or Turkish,** YouTube's content included: following your Mac,
  or chosen from the account button (top right) or the View menu.
- **Instant pages.** Pages you've seen open from a small cache on disk, so
  Home is on screen 0.15 s after launch; ones older than half an hour
  refresh in the background.
- **YouTube's own player plays the audio,** in a hidden system web view, in a
  helper process. Kilo never downloads, captures or decodes media, and never
  blocks ads. Music videos play as their song version when there is one,
  which needs 41% less data.
- **The helper goes when you stop.** Five minutes after you pause, the
  helper and all of WebKit are shut down; pressing play starts it again
  where you left off (about 2 s).
- **Browsing** uses YouTube Music's web API with your own signed-in session,
  the way music.youtube.com does.

## Keyboard shortcuts

One table for every platform (`kilo_core::shortcuts`; the Windows and
Linux front ends to come share it): ⌘ on macOS is Ctrl there, and the
sidebar follows each platform's habit. Plain keys (Space, M, the arrows)
act whenever you're not typing in the search field.

| | macOS | Windows, Linux |
|---|---|---|
| Play or pause | Space | Space |
| Next, previous track | ⌘→, ⌘← | Ctrl+→, Ctrl+← |
| 10 seconds forward, back | → ← (or ⇧⌘→, ⇧⌘←) | → ← (or Ctrl+Shift+→, Ctrl+Shift+←) |
| Volume up, down | ⌘↑, ⌘↓ | Ctrl+↑, Ctrl+↓ |
| Mute | M | M |
| Search | ⌘F, ⌘L, ⌘K or / | Ctrl+F, Ctrl+L, Ctrl+K or / |
| Leave the search field | Esc (clears it first) | Esc |
| Back | ⌘[, ⌥← or the mouse's back button | Alt+←, the mouse's back button |
| Home, Explore, Library | ⌘1, ⌘2, ⌘3 | Ctrl+1, Ctrl+2, Ctrl+3 |
| Reload the page | ⌘R | Ctrl+R |
| Collapse or expand the sidebar (also ☰) | ⌃⌘S | Ctrl+B |
| Scroll | ↑ ↓, Page Up, Page Down, Home, End | the same |

## Requirements

- macOS 14 or later.
- A YouTube Music Premium account to play music. Free accounts can browse
  and search. If an ad ever appears, Kilo stops playback rather than play
  it unseen.

## Build

The Rust toolchain is pinned in `rust-toolchain.toml`; rustup installs it.

```sh
./packaging/macos/bundle.sh            # builds dist/Kilo.app
./packaging/macos/bundle.sh --install  # also copies it to /Applications
```

The app is signed ad hoc, which is enough to run it on the Mac that built it.

## Privacy and security

- You sign in on Google's own page, in a system web view. Kilo never sees
  your password, and stores only YouTube's cookies, not your Google account
  session.
- Kilo talks only to YouTube and Google, and adds no telemetry of its own.
- **Sign Out** (the account button, top right, or the Kilo menu) deletes
  your session and everything Kilo stored.
- How Kilo protects your account, its known limitations, and how to report
  a vulnerability: [`SECURITY.md`](SECURITY.md).
- On disk: WebKit's cookie store for Kilo (your session,
  `~/Library/HTTPStorages/io.github.alperkerimelmas.kilo.binarycookies`),
  music.youtube.com's page config
  (`~/Library/Application Support/io.github.alperkerimelmas.kilo`), and
  caches of thumbnails (up to 64 MB) and pages (16 MB), per sign-in
  (`~/Library/Caches/io.github.alperkerimelmas.kilo`).

## Layout

| Crate | What it is |
|---|---|
| [`kilo-core`](crates/kilo-core) | Cross-platform YouTube Music API client, parsers, models, queue |
| [`kilo-player`](crates/kilo-player) | The player helper and its line protocol |
| [`kilo-mac`](crates/kilo-mac) | The macOS app |
| [`kilo-probe`](crates/kilo-probe) | Measures memory and CPU like the OS task managers do |
| [`kilo-spike-web`](crates/kilo-spike-web), [`kilo-ui-bench`](crates/kilo-ui-bench) | Measurement labs behind the numbers in `docs/PLAN.md` |

`packaging/macos` holds the app bundle's files and build script, and
`scripts/measure.sh` launches a build, runs a scripted session and measures it.

## Development

```sh
cargo test --workspace
cargo clippy --release --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Every efficiency change is measured with `kilo-probe` before it's kept.
Developer switches (scripted sessions, snapshots without screen recording)
are listed in [`crates/kilo-mac/src/debug.rs`](crates/kilo-mac/src/debug.rs).

- [`docs/PLAN.md`](docs/PLAN.md): the design, and every measurement behind it.
- [`docs/COMPARISON.md`](docs/COMPARISON.md): the measured comparison with
  YouTube Music in Chrome.
- [`docs/HANDOFF.md`](docs/HANDOFF.md): status, decisions and next steps.

## License

[MIT](LICENSE).
