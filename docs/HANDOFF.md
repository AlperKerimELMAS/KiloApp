# Kilo: handoff

This is the project's story and current status, written for a new working
session. `CLAUDE.md` at the repo root is the short version (loaded every
session). For depth:
- `docs/PLAN.md`: design and every measurement.
- `docs/COMPARISON.md`: the measured comparison with Chrome.

**Last updated:** 2026-10-07 (cleanup before publishing) · **Version:** 0.2 ·
**Branch:** `main` (no remote yet)

---

## 1. What Kilo is

Kilo is the most resource-efficient YouTube Music desktop client we can
build. The owner's motto is **"every single kilobyte counts."** It was
inspired by Spotifast, a native Rust Spotify client that claims 100–250 MB.

**Goals:**
- Professional, legal, and open source.
- macOS first. The code must stay portable: Windows and Linux front ends
  need machines to build and measure on.
- Kilo's own UI text is in English. YouTube's content comes in the
  account's own language and region (`hl`, `gl`, read from
  music.youtube.com's page config).

**Today:** `Kilo.app` is installed in `/Applications` and works:
- Home, Explore, Library, search, and album, playlist and artist pages.
- Sign-in through Google's page.
- Playback with a queue and radio continuation.
- Media keys.
- Music keeps playing with the window closed.

## 2. Ground rules

1. **Kilo never touches media.** Audio is played only by YouTube's own player
   inside a system web view (the player helper). The app never downloads,
   processes or captures audio or video itself. The owner chose this
   deliberately, early on, to stay legal and publishable; it's not up for
   revisiting.
2. **Playback is Premium-only.** The helper refuses to load anything without
   a signed-in session, and **stops and reports if an ad appears**: hidden
   ads would be fake impressions.
3. **No downloads, no ad blocking.** The hidden page may only have images
   and fonts blocked, nothing else.
4. **No YouTube branding.** Kilo is described as an unofficial client, not
   affiliated with YouTube or Google.
5. **Measure, don't guess.** Every efficiency decision is backed by
   `kilo-probe` numbers.
6. **Ask the owner first** before touching their data (cookie stores),
   pushing or publishing anything, or anything sensitive under YouTube's ToS.

## 3. How we got here (key decisions)

1. **The legal route.** After a legal review, playback goes through YouTube's
   own player in a hidden web view, the same approach as Kaset. This keeps
   the project in "ToS gray area" territory and out of anti-circumvention
   law. Browsing (Home, search, Library) uses YouTube Music's web API with
   the user's own signed-in session, the way the official web client does.
2. **Picking the player page.** We measured each option in a hidden
   WKWebView (`crates/kilo-spike-web`):

   | Page | Playing |
   |---|---|
   | music.youtube.com, desktop | 518 MB |
   | + Lockdown Mode | 345 MB |
   | m.youtube.com | 200 MB |
   | **m.youtube.com + Lockdown Mode + 128×72 view + hidden page UI** | **~113–122 MB** |

   The trade-off: m.youtube.com's player also streams a 144p video track,
   because the mobile web player has no audio-only mode.
3. **Player in a helper process** (`crates/kilo-player`). The app never loads
   WebKit. It spawns itself as `--player-helper`, drives it over a one-line
   text protocol, and kills it after 5 minutes paused, which frees about
   110 MB. Resuming takes about 2 s.
4. **UI toolkit: native AppKit,** chosen by measurement. Same window: AppKit
   27 MB, Slint software renderer 84 MB, Slint GPU renderer 171 MB. So each
   OS gets a native front end over a shared core (`crates/kilo-core`).
5. **System controls that quietly cost memory** (found by bisecting with
   `crates/kilo-ui-bench`):
   - **`NSSlider` starts Apple's Metal shader compiler service (40 MB) on
     macOS 27.** Kilo uses its own two-layer bar (`ui/bar.rs`).
   - **A focused text field starts the AutoFill service (11 MB).** An
     invisible `FocusSink` view takes focus instead of the search field.
   - **Creating views for whole pages cost about 24 MB.** Pages now use
     hand-made frame layout, and only blocks within half a screen of view
     exist.
6. **Comparison** (`docs/COMPARISON.md`; Kilo after the efficiency pass in
   item 7):

   | | Chrome | Kilo |
   |---|---|---|
   | Playing | 1,088 MB | **147.5 MB** |
   | Window open, not playing | | **31 MB** |
   | Window closed / paused 5+ minutes | | **27–31 MB, 0% CPU** |
   | CPU while playing | 8.9% | 8.2% |
   | Data while playing a song | 1.2 MB/min | about 1.9 MB/min (mostly audio; not fully attributed) |
   | Data while playing a music video | | 1.8 MB/min (its song version; was 3.0) |
   | On disk | 1.4 GB | 2.4 MB |

7. **Efficiency pass (v0.2, 2026-10-07),** every step measured (details in
   `docs/PLAN.md`, "Efficiency pass"):
   - **The video track:** m.youtube.com always streams one, and hiding it
     doesn't stop WebKit decoding it. music.youtube.com streams audio only
     but costs 398 MB. So Kilo plays a music video's **song version**
     whenever there is one, as YouTube Music's Song/Video switch does: a
     song's "video" is a 144×144 still, 7% of the bytes, against 38% for a
     music video (−41% data).
   - **Images were held twice** (a `CGImage` as layer contents gets copied).
     They're now decoded into IOSurfaces, and cached ones nothing shows are
     purgeable, so they don't count toward the footprint.
   - **Closing the window now releases it** (it used to free almost
     nothing), and the progress timer stops whenever the window isn't
     visible.
   - **Less network:** "up next" answers carry a field mask (800 KB → 110 KB
     of JSON), Home and long playlists load as you scroll, thumbnails are
     q75 JPEG or WebP (33–55% smaller), the daily config read stops after
     64 KB, and HTTP buffers are 16 KB instead of 128 KB.
8. **Cleanup before publishing (2026-10-07).** No behavior changes beyond the
   fixes below; footprint (30.9 MB, window open) and binary size (+16 bytes)
   measured unchanged.
   - **Structure:** `kilo-mac`'s 1,200-line `app.rs` is now `app/` (`mod.rs`
     state and window, `browse.rs`, `queue.rs`, `player.rs`, `session.rs`).
     The player's scripted session moved to `kilo-player/examples/`, so the
     player library no longer depends on `kilo-probe`. One version of each
     objc2 crate in `[workspace.dependencies]`; `rustfmt.toml` (width 140);
     clippy's `undocumented_unsafe_blocks` lint enforces `SAFETY:` comments;
     CI in `.github/workflows/ci.yml`.
   - **Fixes:** a late "ended" from the previous track no longer skips a
     track; events from a helper that's been replaced are ignored (a dying
     helper's "exited" could take the new one); if the helper dies while
     playing, the bar stops and play resumes from there; the volume slider
     keeps its value when the window reopens; a slow "play playlist" or
     radio answer can't replace or extend a queue started after it; a radio
     or list part that failed to load is asked for again when the queue
     needs it; the radio also takes over after a long list ends;
     Previous/seek work after the idle shutdown; the helper's guard against
     unrequested videos no longer pauses the next track on a stale event;
     quitting the helper after an ad or sign-out no longer blocks the UI.

## 4. Architecture

```
Kilo.app/Contents/MacOS/Kilo  (one binary, three roles)
├─ (no args)          the app: native AppKit UI, never loads WebKit
├─ --player-helper    hidden WKWebView running m.youtube.com's player (kilo-player)
└─ --login-helper     visible WKWebView with Google's sign-in page (kilo-mac/login.rs)
```

| Crate | Role |
|---|---|
| `kilo-core` (portable) | `http.rs` (ureq on the OS's TLS via native-tls), `auth.rs` (reads WebKit's cookie file, builds the request signature YouTube's web client uses), `client.rs` (API calls: browse, continuation, search, next, `next_single` for a track's song version, page config cached 24 h in `~/Library/Application Support/Kilo/innertube.txt`; "up next" requests carry a field mask), `parse/` (serde structs naming only the fields Kilo shows, no JSON tree, under 2 ms per page; the same structs generate the field mask), `model.rs` (Page, Section::{Cards, List, Text}, Entry, Target, Thumb with exact-size URLs), `queue.rs` |
| `kilo-player` | `protocol.rs`: commands `load <id> <start>`, `play`, `pause`, `seek`, `volume`, `quit`; events `ready`, `playing`/`paused`/`buffering`/`ended <t> <dur> <id>`, `signed-out`, `ad`, `next`, `previous`, `error`. `host.rs`: spawn, send, and emit `error helper exited` (`host::EXITED`) when it dies. `helper/mac.rs`: the WKWebView setup, the page bridge, ad and unexpected-video guards, media-key routing. `examples/session.rs`: a scripted test session. |
| `kilo-mac` | `app/` (`mod.rs`: state, launch, the window, which exists only while open; `browse.rs`: navigation, pages and long lists loading as they scroll; `queue.rs`: queues, a queue started from a long list taking the rest as it arrives, the radio; `player.rs`: the helper and its events, the player bar, idle shutdown; `session.rs`: sign-in and connecting), `ui/` (`mod.rs` helpers and views, `page.rs` lazy pages, `shell.rs` window, sidebar, player bar and menus, `bar.rs` slider), `images.rs` (disk cache, ImageIO decode into IOSurfaces, 16 MB purgeable cache), `net.rs` (two small worker pools with main-thread completions), `login.rs` (the sign-in helper and its output format), `paths.rs`, `debug.rs` (developer switches) |
| `kilo-probe` | Memory and CPU like the OS task managers show, summed over the whole process tree, including the macOS XPC services charged to the app. macOS, Windows and Linux. |
| `kilo-spike-web`, `kilo-ui-bench` | The measurement labs behind the decisions above |

**Bundle id:** `io.github.alperkerimelmas.kilo`. WebKit keys its cookie store
by it.

**Paths:**
- Cookies: `~/Library/HTTPStorages/<bundle id>.binarycookies`
- Image cache: `~/Library/Caches/<bundle id>/images`

## 5. Build, run, measure

```sh
./packaging/macos/bundle.sh --install    # build dist/Kilo.app and copy it to /Applications
cargo fmt --all -- --check               # rustfmt.toml: width 140
cargo clippy --release --workspace --all-targets -- -D warnings
cargo clippy --release --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
cargo clippy --release -p kilo-probe -p kilo-player --all-targets --target x86_64-unknown-linux-gnu -- -D warnings
cargo test --workspace                    # 17 tests
./target/release/kilo-probe <pid> -d 30 -i 5 --breakdown [--csv f.csv]
```

**Developer switches** (environment variables, see `debug.rs`):
- `KILO_SNAPSHOT=/path/x` writes the window to `x-N.png`. This terminal has
  no Screen Recording permission, so this is the only way to see the UI.
  Don't measure memory during snapshot runs; the render inflates it.
- `KILO_OPEN=search:Q` or `browse:ID` opens that page after Home.
- `KILO_NO_ACTIVATE=1` launches without taking focus.
- `KILO_SCENARIO=wait:3,play:ID,wait:95,close,wait:65,pause,wait:345` runs a
  scripted session and logs each step to stderr. Other steps: `playvideo:ID`
  (as a click on a music video's card), `browse:ID`, `scroll:Y` or
  `scroll:end`, `playall`, `shuffleall`, `next`, `open`, `volume:N`
  (`volume:0` first to test playback silently), and `state`, which logs
  the page's sections and the queue.

**Measuring:** `scripts/measure.sh dist/Kilo.app wait:15` (window open),
`... wait:12,close,wait:20` (closed), `... --play VIDEO_ID` (playing, with
network bytes). `cargo run --release -p kilo-core --example masks <cookies>`
checks the field mask against live answers.

**Testing without disturbing the owner:** copy the app to the scratchpad and
launch it with `open -g -n -o log --stderr log --env ... Copy.app`.

**Gotchas:**
- **Launch through `open` on the `.app` when measuring.** Processes started
  from a terminal get their XPC services charged to the terminal.
- Linux clippy of `kilo-core` fails on this Mac (openssl-sys needs headers).
  That's expected.
- zsh doesn't split `$var`; use `${=var}`.
- `log` is a zsh builtin: read the system log with `/usr/bin/log show`.
- A scenario's `KILO_IDLE=SECS` shortens the idle shutdown, and player
  events (and how the helper ended) are logged during a scenario.
- The harness blocks chained `sleep`s; use `until` loops or background
  tasks.

## 6. Status and next steps

**Works and is measured:** everything listed in section 1. UI with Home
loaded: 31 MB total, 27 MB with the window closed, 0% CPU when idle.

**Open items, in priority order:**
1. **The video track (done as far as it legally goes).** m.youtube.com has no
   audio-only mode, and music.youtube.com, which does, costs 398 MB.
   Kilo plays song versions instead (section 3, item 7). The remaining
   options touch YouTube's player itself (pretending the browser can't
   play video, or building m.youtube.com's Shorts-sound player, which has
   `deviceIsAudioOnly`) and need the owner's decision; neither is
   recommended.
2. **More UI memory, if wanted:** draw row and card text by hand instead
   of `NSTextField`s (about 6 KB and 9 constraints each, maybe 1 MB in all;
   bench the layer cost first). Consider shutting the helper down sooner
   while paused with the window closed (owner's call: resume then takes 2 s).
3. **Session hardening:**
   - **Delete the development copies of the session** in
     `~/Library/HTTPStorages/kilo-spike-web*` and
     `~/Library/WebKit/kilo-spike-web`, **only with the owner's OK.**
   - Add **Sign out**, which clears Kilo's WebKit data.
   - Consider storing the session in the Keychain and keeping only
     youtube.com cookies. Today WebKit's file holds the full Google session
     unencrypted, in a folder only the user can read.
4. **Before publishing:**
   - Choose a license (MIT suggested; not decided yet). Then add `LICENSE`,
     `license` in `Cargo.toml`, and update the README's License section.
   - Decide whether `docs/HANDOFF.md` and `CLAUDE.md` (working notes,
     including details about the owner's account) go public as they are.
   - Get the owner's go-ahead before creating a GitHub remote. CI
     (`.github/workflows/ci.yml`) runs once it's pushed.
   - Optionally, Developer ID signing and notarization.
5. **Features:** a Now Playing view (big art plus Up next and Lyrics), a
   queue panel, like/dislike, add to playlist.
6. **Unverified:**
   - Media keys and Control Center next/previous; implemented, but I didn't
     press them myself.
   - Whether helper plays show up in YouTube Music history.
7. **Windows and Linux front ends** over `kilo-core` and `kilo-player`, once
   machines are available:
   - Windows: Win32/Direct2D with a WebView2 helper.
   - Linux: GTK with a WebKitGTK helper.

**Leftovers:**
- `dist/` is git-ignored.
- `crates/kilo-spike-web` and the dev `KiloPlayer.app` (wrapping
  `target/release/examples/session`) share the `kilo-spike-web` cookie
  store.
