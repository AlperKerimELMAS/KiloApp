# Kilo: handoff

This is the project's story and current status, written for a new working
session. `CLAUDE.md` at the repo root is the short version (loaded every
session). For depth:
- `docs/PLAN.md`: design and every measurement.
- `docs/COMPARISON.md`: the measured comparison with Chrome.

**Last updated:** 2026-10-07 · **Version:** 0.1 · **Branch:** `main` (no remote
yet) · 5 commits

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
6. **Final comparison** (`docs/COMPARISON.md`):

   | | Chrome | Kilo |
   |---|---|---|
   | Playing | 1,088 MB | **153 MB** |
   | Paused 5+ minutes | | **34 MB, 0% CPU** |
   | CPU while playing | 8.9% | 8.9% (same) |
   | Data while playing | 1.2 MB/min | 1.9 MB/min (+59%, the 144p video) |
   | On disk | 1.4 GB | 2.3 MB |

## 4. Architecture

```
Kilo.app/Contents/MacOS/Kilo  (one binary, three roles)
├─ (no args)          the app: native AppKit UI, never loads WebKit
├─ --player-helper    hidden WKWebView running m.youtube.com's player (kilo-player)
└─ --login-helper     visible WKWebView with Google's sign-in page (kilo-mac/login.rs)
```

| Crate | Role |
|---|---|
| `kilo-core` (portable) | `http.rs` (ureq on the OS's TLS via native-tls), `auth.rs` (reads WebKit's cookie file, builds the request signature YouTube's web client uses), `client.rs` (API calls: browse, continuation, search, next, page config cached 24 h in `~/Library/Application Support/Kilo/innertube.txt`), `parse/` (serde structs naming only the fields Kilo shows, no JSON tree, under 2 ms per page), `model.rs` (Page, Section::{Cards, List, Text}, Entry, Target, Thumb with exact-size URLs), `queue.rs` |
| `kilo-player` | `protocol.rs`: commands `load <id> <start>`, `play`, `pause`, `seek`, `volume`, `quit`; events `ready`, `playing`/`paused`/`buffering`/`ended <t> <dur> <id>`, `signed-out`, `ad`, `next`, `previous`, `error`. `host.rs`: spawn, send, and emit `error helper exited` when it dies. `helper/mac.rs`: the WKWebView setup, the page bridge, ad and unexpected-video guards, media-key routing. `main.rs`: a scripted test session. |
| `kilo-mac` | `app.rs` (state, navigation, sign-in, queue, player control, idle shutdown), `ui/` (`mod.rs` helpers and views, `page.rs` lazy pages, `shell.rs` window, sidebar, player bar and menus, `bar.rs` slider), `images.rs` (disk cache + ImageIO decode + 8 MB LRU), `net.rs` (two small worker pools with main-thread completions), `login.rs`, `paths.rs`, `debug.rs` (developer switches) |
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
cargo clippy --release --workspace -- -D warnings
cargo clippy --release --workspace --target x86_64-pc-windows-msvc -- -D warnings
cargo clippy --release -p kilo-probe -p kilo-player --target x86_64-unknown-linux-gnu -- -D warnings
cargo test --workspace                    # 11 tests
./target/release/kilo-probe <pid> -d 30 -i 5 --breakdown [--csv f.csv]
```

**Developer switches** (environment variables, see `debug.rs`):
- `KILO_SNAPSHOT=/path/x` writes the window to `x-N.png`. This terminal has
  no Screen Recording permission, so this is the only way to see the UI.
  Don't measure memory during snapshot runs; the render inflates it.
- `KILO_OPEN=search:Q` or `browse:ID` opens that page after Home.
- `KILO_NO_ACTIVATE=1` launches without taking focus.
- `KILO_SCENARIO=wait:3,play:ID,wait:95,close,wait:65,pause,wait:345` runs a
  scripted session and logs each step to stderr.

**Testing without disturbing the owner:** copy the app to the scratchpad and
launch it with `open -g -n -o log --stderr log --env ... Copy.app`.

**Gotchas:**
- **Launch through `open` on the `.app` when measuring.** Processes started
  from a terminal get their XPC services charged to the terminal.
- Linux clippy of `kilo-core` fails on this Mac (openssl-sys needs headers).
  That's expected.
- zsh doesn't split `$var`; use `${=var}`.
- The harness blocks chained `sleep`s; use `until` loops or background
  tasks.

## 6. Status and next steps

**Works and is measured:** everything listed in section 1. UI with Home
loaded: 36–40 MB total, 0% CPU when idle.

**Open items, in priority order:**
1. **Drop the 144p video.** Re-test music.youtube.com as served to phones as
   the player page: it likely plays audio-only and may be light. It didn't
   start in one quick test, probably because of how playback was started.
   Fallback: measure the desktop music.youtube.com page with its UI hidden.
2. **Session hardening:**
   - **Delete the development copies of the session** in
     `~/Library/HTTPStorages/kilo-spike-web*` and
     `~/Library/WebKit/kilo-spike-web`, **only with the owner's OK.**
   - Add **Sign out**, which clears Kilo's WebKit data.
   - Consider storing the session in the Keychain and keeping only
     youtube.com cookies. Today WebKit's file holds the full Google session
     unencrypted, in a folder only the user can read.
3. **Before publishing:**
   - Choose a license (MIT suggested; not decided yet).
   - Get the owner's go-ahead before creating a GitHub remote.
   - Optionally, Developer ID signing and notarization.
4. **Features:** a Now Playing view (big art plus Up next and Lyrics), a
   queue panel, like/dislike, add to playlist.
5. **Unverified:**
   - Media keys and Control Center next/previous; implemented, but I didn't
     press them myself.
   - Whether helper plays show up in YouTube Music history.
6. **Windows and Linux front ends** over `kilo-core` and `kilo-player`, once
   machines are available:
   - Windows: Win32/Direct2D with a WebView2 helper.
   - Linux: GTK with a WebKitGTK helper.

**Leftovers:**
- `dist/` is git-ignored.
- `crates/kilo-spike-web` and the dev `KiloPlayer.app` share the
  `kilo-spike-web` cookie store.
