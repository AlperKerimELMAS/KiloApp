# Kilo — plan

The most resource-efficient YouTube Music desktop client possible, for macOS,
Windows and Linux. Same YouTube Music structure (sidebar, Home / Explore /
Library, search, bottom player bar, Now Playing with Up next / Lyrics /
Related), drawn natively. Every kilobyte counts.

## Ground rules

- **Publishable as open source.** No circumvention: audio is only ever played
  by YouTube's own player, inside a system web view. Kilo never fetches,
  captures or decodes the media itself.
- **Premium-only playback.** Free accounts can browse and search but can't
  play. A hidden player would turn ads into fake impressions, so the player
  stops if an ad ever shows.
- **No downloads, no ad blocking.**
- **No YouTube branding.** No logos, no "YouTube" in the name, and no YouTube
  Sans font. Kilo is described as an unofficial client, not affiliated with
  YouTube or Google.
- **Audio-only v1.**
- **Measure, don't guess.** Every decision is backed by `kilo-probe` numbers.

## Targets

Footprint means what each OS's task manager shows: `phys_footprint` on macOS,
private working set on Windows, PSS on Linux, with every helper process
included (on macOS that also means WebKit's XPC services).

| State | Target | Status (macOS) |
|---|---|---|
| Window open, idle | ≤ 40 MB Retina, ≤ 22 MB at 1× | **31 MB measured** (Home loaded, Retina, 0% CPU); 27 MB with the window closed |
| Playing, everything included | ≤ 125 MB | **147.5 MB measured** for the app with its window open (`docs/COMPARISON.md`); the player alone, under a 2 MB test host, is 113–118 MB |
| Paused for more than a few minutes | main process only | **31 MB measured** (helper shut down; the UI is what's left); 2.3 MB for the test host |

Baseline (same Mac, 2026-10-06): the YouTube Music PWA in Chrome, playing a
song, uses **1,082–1,161 MB** across 28 processes at about 9.5% CPU. Of that:
- 284 MB is the page's renderer.
- 235 MB is the GPU process.
- 127 MB is the browser process.
- About 30 MB is Google Updater.

Also: binary ≤ 8 MB, under 150 ms to the first frame, zero wakeups while
paused, and the lowest CPU we can reach while playing. That's currently about
10.5% of one core:
- 7.7% is YouTube's player script, interpreted because Lockdown Mode has no
  JIT.
- 2.5% is media decoding.
- 0.7% is networking.
- 0.4% is our two processes.

This is about the same as, or a little more than, Chrome's YouTube Music. With
the JIT on, the script costs less, but the total doesn't go down and memory
goes up by about 80 MB. That could become an opt-in "lower CPU" setting.

## Playback: YouTube's player in a hidden web view

Measured on 2026-10-06 (MacBook, Apple Silicon, macOS 27.0.1, Premium
account), playing a 30–180 s track, with the spike measuring itself
(`crates/kilo-spike-web`):

| Configuration | Avg while playing | Peak | WebContent | GPU | CPU (1 core) |
|---|---|---|---|---|---|
| music.youtube.com, no window | doesn't play: stalls buffering | | | | |
| music.youtube.com, 1×1 offscreen window | 518 MB | 538 MB | 348 MB | 93 MB | 6.7% |
| + images and fonts blocked | 516 MB | 530 MB | 355 MB | 91 MB | 6.3% |
| + Lockdown Mode | 345 MB | 361 MB | 217 MB | 88 MB | 11.8% |
| music.youtube.com as served to phones | doesn't start | | | | |
| m.youtube.com | 200 MB | 234 MB | 132 MB | 18 MB | 10.0% |
| **m.youtube.com + Lockdown Mode, 128×72 view** | **122 MB** | **176 MB** | **68 MB** | **18 MB** | **11.4%** |
| same, without Lockdown Mode (JIT on) | 200 MB | 259 MB | 140 MB | 18 MB | 12.7% |

Notes on the winning configuration:
- Over 3 minutes it settles at 120–122 MB and stays flat (no leak). The
  175 MB peak happens only while the page loads.
- The player chooses the 144p video stream for the tiny view, and `audible()`
  asks for "tiny" quality explicitly.
- **WebKit's background scheduling setting has no effect:** none, throttle and
  suspend all measure the same, because a page that's playing audio counts as
  active.
- **CPU breakdown:** YouTube's player script 7.1%, media decoding in the GPU
  process 2.5%, networking 0.3%. The spike's own once-per-second measuring adds
  about 1.3%, which the real app won't have.
- **After the web view is destroyed, about 48 MB remains:** WebKit's code in
  our process plus its GPU and Networking processes. That's why the player
  must live in its own helper process (below).
- **Lockdown Mode** is Apple's public hardened browsing mode: no JIT and no
  WebAssembly. YouTube's player works under it, and it saves roughly
  80–170 MB.
- The web view identifies as Safari, since WKWebView is Safari's engine.
  Google sign-in works in it.

Configuration (macOS): a `WKWebView` at 128×72 in a borderless window off
screen, no user action required for media, `fraudulentWebsiteWarningEnabled`
off, Lockdown Mode on, and a content rule list that blocks only images and
fonts. It loads `https://m.youtube.com/watch?v=ID` with the Mobile Safari user
agent. Never block scripts, the network requests that report plays, or
attestation.

## Player helper: measured end to end

`crates/kilo-player` is the real architecture, not a spike. The app spawns
itself with `--player-helper` and talks to it over a one-line text protocol
(`load <id> <start>`, `play`, `pause`, `seek`, `volume`, `quit` →
`ready`, `playing|paused|buffering|ended <t> <duration> <id>`, `signed-out`,
`ad`, `error`). A scripted session (`session <idA> <idB>`, run as an
`.app` bundle), from 2026-10-06:

| Phase | Result |
|---|---|
| App process alone (never loads WebKit) | 2.0 MB |
| Cold start to audio | 1.6–1.9 s |
| Playing, everything included | 113–118 MB, peak ~155 MB while the page loads |
| Events sent by the helper while playing | **0**: the app extrapolates position instead of polling |
| Switching tracks (`loadVideoById`, same page) | 0.45–0.85 s |
| Paused | 110–114 MB, 1.8% CPU (the page still runs 1.2–1.4%) |
| Resume from pause | 0.01–0.05 s |
| Helper killed after idling | 2.3 MB |
| Resume after the kill | 1.7–2.3 s, starting about 1 s back |

Where the memory goes while playing:

| Process | Footprint |
|---|---|
| WebContent | 57–61 MB |
| helper (AppKit + WebKit's app side) | 21 MB |
| GPU | 17 MB |
| Networking | 7 MB |
| CoreAudio sandbox helper | 5 MB |
| SetStoreUpdateService | 2 MB |
| app | 2.3 MB |

The helper's own 21 MB is about 5.5 MB of live allocations plus framework
data that every AppKit process has. Releasing free malloc memory recovered
0 KB, so there's nothing left to trim there.

Safety built into the helper:
- Video ids are validated before they reach a URL or script.
- Main-frame navigation is limited to YouTube and Google hosts.
- Anything starting to play that the app didn't ask for, such as the mobile
  site's autoplay, is paused.
- An ad showing means the account isn't Premium, so the helper pauses and
  reports it.
- If the WebContent process dies, the helper exits so the app can start a
  clean one.
- The helper exits when the app closes its stdin, so it never outlives the
  app.

Also measured:
- Hiding the window after playback starts doesn't stop audio, but it doesn't
  save CPU either.
- WebKit's Suspend scheduling policy doesn't suspend a page that has played
  media.

## The macOS app (v0.1, 2026-10-07)

`crates/kilo-mac` builds `Kilo.app` (`packaging/macos/bundle.sh --install`).
The app is a native AppKit UI with no web engine. The same binary also runs
as the player helper and as the sign-in window.

**Features:**
- Home, Explore, Library, search, and album, playlist and artist pages.
- Sign-in through Google's own page, opened in a separate helper.
- A queue that keeps going with each track's radio when it runs out.
- Play/Shuffle buttons.
- Media keys and Control Center routed through Kilo's queue.
- Music keeps playing when the window is closed.

**Memory, measured with Home loaded, Retina, 1240×820 window:**

| Process | Footprint |
|---|---|
| Kilo (the UI) | 36–38 MB |
| SetStoreUpdateService (every macOS app gets one) | 2.2 MB |
| **Total, not playing** | **≈ 40 MB** |
| **Total, playing** (adds the player helper) | **≈ 150 MB** |

For comparison, the YouTube Music PWA in Chrome uses about 1.1 GB.

How it gets there, each step measured:
- **Native AppKit, not Slint.** The same window measured 27 MB in AppKit
  versus 84 MB with Slint's software renderer (about 5 full-window buffers)
  and 171 MB with its GPU renderer.
- **No `NSSlider`.** On macOS 27 it starts Apple's Metal shader compiler
  service, which costs 40 MB, plus about 5 MB in-process. Kilo's sliders are
  two plain layers (`ui/bar.rs`).
- **A focus target, not an auto-focused search field.** When the window
  becomes key, AppKit focuses the search field, and a focused text field
  starts macOS's AutoFill service (11 MB). An invisible view takes focus
  instead (`FocusSink`).
- **Lazy pages.** Pages are laid out by hand with no Auto Layout. Only blocks
  within half a screen of the view exist, and thumbnails are attached only
  while visible. This took the UI from 61 MB to 37 MB.
- **Images:**
  - Requested from the server at exact pixel size.
  - Cached on disk, capped at 64 MB.
  - Decoded off the main thread by ImageIO, into an IOSurface that Core
    Animation shows without a copy (see "Efficiency pass" below).
  - Images nothing shows stay cached as purgeable memory, up to 16 MB.
- **Small dependencies.** The OS's TLS (`native-tls`) instead of bundling
  rustls, and parsers that deserialize only the fields Kilo shows (sub-ms per
  page).

**Size:** the binary is 1.4 MB and `Kilo.app` is 2.3 MB (1 MB of that is the
icon).

**Developer switches** (`crates/kilo-mac/src/debug.rs`):
- `KILO_SNAPSHOT=path` renders the window to PNG, with no Screen Recording
  permission needed.
- `KILO_OPEN=search:q` or `browse:ID` opens that page.
- `KILO_NO_ACTIVATE=1` launches without taking focus.

## Efficiency pass (v0.2, 2026-10-07)

Every change below was measured with `kilo-probe`, the same Mac, Retina,
1240×820 window, against the build before it. Totals include
SetStoreUpdateService (2.1 MB), as Activity Monitor would.

**UI process:**

| State | Before | After | What did it |
|---|---|---|---|
| Window open, Home loaded | 36.0–37.5 MB | **30.5–31.0 MB** (up to 34 when Home shows more video thumbnails) | IOSurface images, smaller HTTP buffers |
| Window closed (20 s after) | 29.1–38.5 MB | **26.6–27.6 MB** | the whole window is released on close |
| Home after six scrolls | 52 MB | **35–39 MB** (+ up to 16 MB purgeable, not counted) | IOSurface images, purgeable cache |
| Playing, window closed: app CPU / wakeups | 0.3%, 3.5/s | **0.0%, 0.2/s** | progress timer stops when the window isn't visible |

- **Images were held twice.** A `CGImage` set as layer contents gets copied
  by Core Animation: Home's thumbnails cost 3.1 MB decoded plus a 3.1 MB
  copy. Decoding into IOSurfaces, which the window server reads in place,
  removed the copy (6.2 → 3.4 MB).
- **Purgeable cache.** Cached images that no layer shows are marked
  volatile. macOS doesn't count volatile memory in the footprint and takes
  it back when it needs to; Kilo then decodes again. The cache can hold
  16 MB without costing anything.
- **Closing the window released almost nothing:** the window and its views
  were kept for reopening. Now the window is dropped on close and rebuilt
  from the app's state when the Dock icon is clicked.
- **HTTP buffers:** ureq keeps 128 KB each way per pooled connection (twelve
  buffers, 1.5 MB, were live). 16 KB is plenty for a few KB of headers.
- **Measured and dropped:** `malloc_zone_pressure_relief` after scrolling
  frees nothing (the heap is live data, not fragmentation). Text labels
  would save about 6 KB each if drawn by hand, because on macOS 27 every
  `NSTextField` gets about 9 autoresizing constraints even in a window
  without constraints (`kilo-ui-bench --bin engine`); not worth it yet.

**Player (m.youtube.com in the helper):**

| | Measured |
|---|---|
| Media bytes, a song (art track) | audio 2.75 MB, video 0.19 MB (7%): the "video" is a 144×144 still |
| Media bytes, a music video | audio 2.3 MB, video 1.4 MB (38%); 3.0 MB/min on the wire |
| A music video's id, now played as its song version | **1.77 MB/min (−41%)**, same CPU |
| `video{display:none}` in the page CSS | same bytes and CPU (WebKit still decodes); helper wakeups 17 → 10/s |
| music.youtube.com as the player page | audio only (one audio buffer), but **398 MB**, 11% CPU, and Metal's compiler service (17.5 MB) |
| Paused | 0.14% CPU, about 1 wakeup/s; hiding the window while paused changes nothing |
| While buffering | WebKit's GPU process adds ~37 MB for under a second per media segment (about every 10 s), in both builds |
| Paused for minutes | macOS 27 itself quits the idle helper ("quiet safe quit", SIGTERM), sometimes before Kilo's 5-minute shutdown |

- **The video track can't be dropped legally on m.youtube.com:** the mobile
  player has no audio-only mode, and hiding the element only saves
  compositing. YouTube Music's own page is audio-only but 2.6× the memory.
  So Kilo plays a music video's song version whenever there is one, as
  YouTube Music does with its Song/Video switch set to Song. Signed in,
  "up next" answers list the song version next to the video (44 of 50 radio
  tracks had one).
- **The GPU process's ~47 wakeups/s** are audio output: AAC's 1024-sample
  frames at 48 kHz are 21 ms apart.

**Network:**

| | Before | After |
|---|---|---|
| A radio ("up next", JSON) | 800–875 KB (36 KB on the wire) | **110 KB (8 KB)**, field mask |
| Song version of one track | 28 KB | **2 KB** |
| Album/artist art (320 px) | JPEG q90 | **q75: 38–55% smaller**, no visible difference |
| Video thumbnails | JPEG | **WebP: 33–50% smaller** (JPEG fallback) |
| Daily config refresh | 549 KB page | **first 64 KB** (the fields are in the first 25 KB) |
| Home before first paint | 3 requests | **1**; more shelves load as you scroll |
| Opening a long playlist | up to 21 requests | **1**; the rest loads on scroll, or into the queue on Play/Shuffle |

- **Field masks:** YouTube's API honors `X-Goog-FieldMask`, so Kilo can ask
  for exactly the fields it parses. The mask is generated from the parser's
  own structs (`parse/raw.rs`), and a masked answer parses to the same queue
  as a full one (`cargo run --example masks`). The server rejects unknown
  field names with a 400 (Kilo then retries without), and nested `*`
  wildcards make it take seconds or time out. Browse and search masks came
  out 45 KB long without wildcards, so only "up next" uses one (and the
  account menu: 5,616 → 371 bytes).

## UI redesign (v0.3, 2026-10-08)

A Spotify / YouTube Music-like look, light and dark, English and Turkish,
and an account button with Sign Out. Every ingredient was priced first in
`kilo-ui-bench` (as an `.app` launched with `open`, against the same window
without it):

| Ingredient | Cost | Used? |
|---|---|---|
| `NSVisualEffectView`, behind-window blur, as the window's root | +0.2 MB | at first; dropped in the second round (below) |
| `NSGlassEffectView` (Liquid Glass) | +1.2 MB per element | no |
| Layer shadows on cards | +0.4 MB, more CPU on scroll | no |
| A rounded panel that clips its content | +0.4 MB | no: laid out so nothing needs clipping |
| Hover play button | one, shared, moved to the card under the mouse | yes |

**Startup** (`scripts/startup.sh`, seconds since `main`, four launches):

| | Before | After |
|---|---|---|
| Window on screen | 0.12–0.15 s | 0.13–0.15 s |
| Home on screen | 0.93–1.05 s | **0.148–0.165 s** |
| First thumbnail | after Home | **~0.17 s** |

- Home comes from the page cache (`pagecache.rs`): YouTube's answer as sent,
  keyed by route and language, 16 MB at most. Under 30 minutes old it's
  shown with no request; up to 7 days old it's shown at once and refetched,
  and the new one replaces it if the user hasn't scrolled and it differs.
- The page config is used even when it's stale, and refreshed for the next
  launch, so only the very first launch waits for it.
- What's left is AppKit: `main` to a ready app is ~78 ms, and Kilo's window
  ~55 ms of it (creating the `NSWindow` 23–27 ms, sidebar 6, player bar 4,
  menu bar 4–7).

**Memory** (`kilo-probe`, medians of several launches):

| State | Before | After |
|---|---|---|
| A fixed playlist page | 31.4 MB | **31.2 MB** |
| Home | 33.7 MB (30.4–36.6: Home's content changes run to run) | 35.5 MB, within that spread |
| Playing | 141–143 MB | 142.6–144 MB |

- Showing a cached page and then parsing the fresh one raised the footprint
  2–4 MB (the heap fragments; `malloc_zone_pressure_relief` freed nothing).
  Hence the 30-minute rule: a page that fresh is never parsed twice.
- Card shelves now create only the cards near view sideways too, as lists
  already did vertically.
- Light/dark and language changes rebuild the window's views: 30–50 ms.

**Size:**

| | Before | After |
|---|---|---|
| Binary | 1,516 KB (v0.2) | **1,165 KB** (`opt-level = "z"`; the v0.3 code at `opt-level = 3` was 1,624 KB) |
| App icon (`.icns`) | 995 KB, gradient | **318 KB**, flat dark |
| `Kilo.app` | 2.4 MB | **1.42 MB** |

`opt-level = "z"` kept the same startup and memory; parsing a 412 KB page
went from 0.6 ms to 0.6–1.0 ms, which nobody can see.

**Second round, after the owner's review (same day).** The blurred frame
took the wallpaper's colors, so the sidebar looked like a separate part:
the frame is now one plain color (the window's own background, as in
Spotify), and the search field is a plain text field in Kilo's own pill
instead of the system's bezel. Measured against the first round's build,
alternating launches on the same cache:

| State | First round | Second round |
|---|---|---|
| A fixed album page | 29.2, 29.4, 29.5, 29.7 MB | **28.2, 28.4, 28.5, 28.5 MB** |
| Home | 34.4, 34.4, 36.9 MB | **32.8, 32.9, 33.7 MB** |
| Binary | 1,158 KB | 1,175 KB (shortcuts, search pill, test steps) |

- Image views now clip (`clipsToBounds`; AppKit had been turning their
  layers' masking off), so rounded corners and circles cost what Core
  Animation charges for them: included in the numbers above.
- Shelves pass vertical scroll gestures to the page, and keyboard
  shortcuts come from one table in `kilo-core` for every platform.

**Width changes** (window resizing, the sidebar collapsing), main-thread time
per change, 40 changes × 3 runs (`relayout:40`):

| Page | Rebuilding what's live | Re-framing it |
|---|---|---|
| Home | 3.83–3.90 ms | **0.76–0.79 ms** |
| An album | 3.84–3.89 ms | **0.16–0.17 ms** |

That's what makes the sidebar's 0.22 s slide affordable: 11–15 frames, the
slowest 3–8 ms of work including AppKit's layout.

## Architecture

As built (the crates are described in `docs/HANDOFF.md`, section 4):

```
Kilo (one binary, five roles)
├─ the app (no arguments): native UI on the main thread, network and image
│  decoding on two small worker pools; never loads WebKit
├─ --player-helper: hidden web view running YouTube's own player, driven by
│  one-line text messages over stdin/stdout; started on play, shut down
│  after 5 minutes paused, which frees all of WebKit
├─ --login-helper: a visible web view for Google sign-in, then exits
├─ --prune-helper: deletes WebKit's data for Kilo except YouTube's
└─ --sign-out-helper: empties WebKit's stores for Kilo, then exits
```

- **Language:** Rust.
- **UI:** native per OS over a shared core (`kilo-core`). On macOS that's
  AppKit, chosen by measurement over Slint's software and GPU renderers
  (the first plan also considered a custom CPU renderer).
- **Web views:** WKWebView on macOS; WebView2 on Windows (it has a public
  memory-target API to try) and WebKitGTK on Linux are planned.
- **Queue:** Kilo owns the queue and switches tracks with the player's own
  `loadVideoById`, which avoids reloading the page.
- **Media controls:** the media keys and Control Center go through the
  hidden page's Media Session, routed to Kilo's queue. MPRIS (Linux) and
  SMTC (Windows) come with those front ends.

## Memory rules

1. Nothing runs unless something changed: no render loop, no polling.
2. Thumbnails are requested at exact pixel size. Decoded pixels count
   toward the footprint only while visible; the rest are purgeable.
3. JSON is parsed straight into small structs and then dropped. Going back to
   a page reads it again (from the disk cache when fresh) rather than
   keeping it in memory.
4. Lists render only their visible rows.
5. A closed window means no UI in memory at all.
6. WebKit only ever runs in the player helper, which exits when idle.
7. Every dependency has to justify its size.

Planned, not in place yet: a byte budget per subsystem, shown in a debug
overlay and enforced in CI, and a cargo-bloat check in CI.

## What we learned about the web client (2026-10-06)

- `WEB_REMIX`, version `1.20261004.17.00`. The version is read from the page
  config at runtime and cached.
- Each track's player response is 170–200 KB of JSON. Browse responses will be
  parsed into small typed structs that skip everything else.
- Anonymous responses carry ad payloads, which is one more reason playback is
  Premium-only.

## Next steps

The current, prioritized list is in `docs/HANDOFF.md`, section 6.

## Tools

- `cargo run --release -p kilo-probe -- <pid|name> [-i SECS] [-d SECS] [--csv FILE]`
  counts the whole process tree, including macOS XPC services attributed to
  the app. A binary started from a terminal has its XPC services attributed to
  the terminal, so measure `.app` bundles launched with `open`.
- `crates/kilo-player`: `cargo build --release -p kilo-player --example
  session` builds the scripted session above (`session <idA> <idB>`). Run it
  as a `KiloPlayer.app` wrapping `target/release/examples/session`, through
  `open -W -o out.txt ... --args`. Its dev bundle id reuses the spike's
  signed-in data store.
- `kilo-probe <pid> --breakdown` lists every process with its footprint and
  CPU.
- `crates/kilo-spike-web`: `login`, then
  `play <videoId> --host offscreen --lean --lockdown --site mweb --size 128x72`.
  Run it as `target/release/KiloSpike.app` through `open -W -o out.txt ... --args`.
