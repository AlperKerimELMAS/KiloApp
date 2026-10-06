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
| Window open, idle | ≤ 40 MB Retina, ≤ 22 MB at 1× | UI not built yet |
| Playing, everything included | ≤ 125 MB | **113–118 MB measured** |
| Paused for more than a few minutes | main process only | **2.3 MB measured** (helper killed) |

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
`ad`, `error`). A scripted session (`kilo-player <idA> <idB>`, run as an
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

## Architecture

```
main process (never loads WebKit)
├─ UI thread: layout, CPU rasterizer that redraws only changed areas, image cache
├─ engine thread: InnerTube client for browse, search, library and likes
└─ OS media controls: MPRIS / SMTC / MPNowPlayingInfoCenter, media keys

player helper (the same binary in `--player` mode; JSON lines over stdin/stdout)
└─ hidden web view running YouTube's own player
   started on play, killed after N minutes paused, which frees all of WebKit

login helper: a visible web view for Google sign-in, then exits
```

- **Language:** Rust.
- **UI:** Slint with its software renderer, or a custom CPU renderer
  (vello_cpu or tiny-skia, with parley for text). Phase 0 decides by
  measurement.
- **Web views:** WKWebView on macOS, WebView2 on Windows (it has a public
  memory-target API to try), WebKitGTK on Linux.
- **Queue:** Kilo owns the queue and switches tracks with the player's own
  `loadVideoById`, which avoids reloading the page.

## Memory rules

1. Nothing runs unless something changed: no render loop, no polling.
2. Thumbnails are requested at exact pixel size. Decoded pixels stay in RAM
   only while visible.
3. JSON is parsed straight into small structs and then dropped. Going back to
   a page re-requests it rather than keeping it in memory.
4. Lists render only their visible rows.
5. A closed window means no UI in memory at all.
6. WebKit only ever runs in the player helper, which exits when idle.
7. Every subsystem has a byte budget, shown in a debug overlay and enforced
   in CI.
8. Every dependency has to justify its size (checked with cargo-bloat in CI).

## What we learned about the web client (2026-10-06)

- `WEB_REMIX`, version `1.20261004.17.00`. The version is read from the page
  config at runtime and cached.
- Each track's player response is 170–200 KB of JSON. Browse responses will be
  parsed into small typed structs that skip everything else.
- Anonymous responses carry ad payloads, which is one more reason playback is
  Premium-only.

## Next steps

1. **UI renderer bake-off:** the main process's budget is the remaining
   unknown.
2. **Windows (WebView2) and Linux (WebKitGTK) helpers,** measured on real
   machines. WebView2 has a public memory-target API to try.
3. **OS media controls:** check what WKWebView's own Now Playing integration
   does in the helper, and route media keys through Kilo.
4. **Verify** that plays from the helper show up in YouTube Music history.
5. **Measure** pear-desktop and Kaset on the same track.

## Tools

- `cargo run --release -p kilo-probe -- <pid|name> [-i SECS] [-d SECS] [--csv FILE]`
  counts the whole process tree, including macOS XPC services attributed to
  the app. A binary started from a terminal has its XPC services attributed to
  the terminal, so measure `.app` bundles launched with `open`.
- `crates/kilo-player`: `kilo-player <idA> <idB>` runs the scripted session
  above. Run it as `target/release/KiloPlayer.app` through `open -W -o out.txt
  ... --args`. Its dev bundle id reuses the spike's signed-in data store.
- `kilo-probe <pid> --breakdown` lists every process with its footprint and
  CPU.
- `crates/kilo-spike-web`: `login`, then
  `play <videoId> --host offscreen --lean --lockdown --site mweb --size 128x72`.
  Run it as `target/release/KiloSpike.app` through `open -W -o out.txt ... --args`.
