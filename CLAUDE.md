# Kilo

Kilo is an ultra-lean, unofficial YouTube Music desktop client: a native UI
over a shared Rust core. It's macOS-first, and Windows and Linux front ends
come later. Motto: **every single kilobyte counts.**

**Start by reading `docs/HANDOFF.md`** for the story, the decisions, the
architecture, the status and the next steps. Details and measurements are in
`docs/PLAN.md` and `docs/COMPARISON.md`.

## Ground rules

- **Kilo never touches media.** Audio plays only through YouTube's own player
  in a hidden system web view, in a helper process. Never add code that
  downloads, processes or captures YouTube media streams. This was decided
  for legal and safety reasons and isn't up for revisiting.
- **Playback is Premium-only.** The helper stops if an ad appears.
- **No downloads, no ad blocking, no YouTube branding.**
- **Back every efficiency change with `kilo-probe` numbers.** Keep a change
  only if it measurably wins.
- **Ask the owner first** before touching their data (cookie stores),
  pushing or publishing, or anything sensitive under YouTube's ToS.
- **Keep the security boundaries** (`SECURITY.md`): the sign-in window and
  the player only show allowlisted https hosts, images pass
  `kilo_core::image` before ImageIO, page messages are validated, and
  `bundle.sh` signs with the hardened runtime.

## Layout

| Crate | What it is |
|---|---|
| `crates/kilo-core` | Portable: HTTP on the OS's TLS, auth from WebKit's cookie file, YouTube Music web API client, lean serde parsers, models, queue; and what every front end shares: words (`strings.rs`), colors (`style.rs`), keyboard shortcuts (`shortcuts.rs`) |
| `crates/kilo-player` | The player helper (WKWebView on macOS) and its one-line text protocol; `examples/session.rs` is a scripted, measured session |
| `crates/kilo-mac` | The macOS app (AppKit via objc2). The binary `Kilo` also runs as `--player-helper`, `--login-helper`, `--prune-helper` and `--sign-out-helper`. `src/app/` holds the state (`mod.rs`), pages (`browse.rs`), queue, player and sign-in; `src/ui/` the views (`theme.rs` light and dark); `keys.rs` the shortcuts; `pagecache.rs` pages on disk |
| `crates/kilo-probe` | Measures footprint and CPU like the OS task managers, over the process tree plus the XPC services macOS charges to the app |
| `crates/kilo-spike-web`, `crates/kilo-ui-bench` | Measurement labs |

## Commands

```sh
./packaging/macos/bundle.sh --install     # build Kilo.app and copy it to /Applications
cargo fmt --all
cargo clippy --release --workspace --all-targets -- -D warnings      # also run with --target x86_64-pc-windows-msvc
cargo test --workspace
./target/release/kilo-probe <pid> -d 30 -i 5 --breakdown
scripts/measure.sh dist/Kilo.app wait:15           # launch, run a scenario, measure
scripts/startup.sh dist/Kilo.app 4                 # time the window, Home and the first image
```

Developer switches live in `crates/kilo-mac/src/debug.rs`: `KILO_SNAPSHOT`,
`KILO_OPEN`, `KILO_NO_ACTIVATE`, `KILO_SCENARIO`. There's no Screen
Recording permission, so use `KILO_SNAPSHOT` to see the UI. `KILO_SCENARIO`
can also browse, scroll, play (`volume:0` first to stay silent), switch
`theme:` and `lang:`, take a `snap`, press keys (`key:cmd+right`; needs
the app active), send scroll gestures to a shelf (`wheel:`), and print the
page and queue state, which is how lazy loading, the queue and the
shortcuts are tested.

## Things that bit us

- **Measure `.app` bundles launched with `open`.** When started from a
  terminal, a process's XPC services are charged to the terminal.
- **Don't use `NSSlider`.** On macOS 27 it starts a 40 MB Metal compiler
  service.
- **Don't let a text field auto-focus.** It starts the 11 MB AutoFill
  service. `FocusSink` takes focus instead.
- **Pages use frame layout and lazy blocks:**
  - Use `frame_label` there: it adds no constraints of its own. On macOS 27
    AppKit still gives every label about 9 autoresizing constraints, even in
    a window with no constraints at all (`kilo-ui-bench --bin engine`).
  - Only views near the screen exist.
- **Never set a `CGImage` as layer contents.** Core Animation copies it, so
  every image on screen costs twice its size. Thumbnails are decoded into
  IOSurfaces, and the ones nothing shows are made purgeable (`images.rs`).
- **A web view only plays media when it's in a window,** so the helper keeps
  it in an offscreen 128×72 window.
- **m.youtube.com always streams a video track,** even when hidden. So Kilo
  plays a music video's song version when it has one: a song's video is a
  still image, 7% of the bytes. music.youtube.com streams audio only, but
  costs 398 MB.
- **YouTube's API honors `X-Goog-FieldMask`,** but rejects any unknown field
  name with a 400, and nested `*` wildcards make it take seconds or time out.
  Only "up next" and account requests carry a mask.
- **macOS 27 quits an idle helper itself** ("quiet safe quit", SIGTERM) a
  few minutes after playback stops, before or after Kilo's own 5-minute
  idle shutdown. Both are fine: the next play starts a fresh helper.
- **WebKit writes cookies to its file only when its process exits.** A
  helper that sets cookies must exit before anyone reads the file
  (`login::wait_until_saved`).
- **Only YouTube's cookies are ever stored.** The sign-in window's store is
  in memory; never give it (or the player) a persistent store that could
  keep Google's account session.
- **Test sign-in and sign-out in a copy with another bundle id** (see
  `docs/HANDOFF.md`, section 6). The real app's helpers use the owner's
  session.
- **Light/dark and the language are read when views are made.** A change
  rebuilds the window (`app::rebuild_window`); nothing tracks it live. New
  UI text goes in `kilo_core::strings` in both languages; colors come from
  `kilo_core::style` (through `theme::palette()`), shortcuts from
  `kilo_core::shortcuts`, so every platform shares them.
- **One plain frame color, no blur.** A behind-window `NSVisualEffectView`
  tints with the wallpaper, so the sidebar looked like a separate part.
  Don't use `NSGlassEffectView` (+1.2 MB per element) or layer shadows
  (+0.4 MB), and lay containers out so nothing needs clipping.
- **Image views need `clipsToBounds`.** AppKit sets a hosted layer's
  `masksToBounds` from the view's `clipsToBounds` (off by default since
  macOS 14), so without it rounded corners stay square and wide images
  spill out of square slots.
- **Plain-key shortcuts must not reach the search field.** Space, M and the
  arrows act only when it isn't focused; menu items with them let the key
  through while it is (`validateMenuItem:` in `ui/mod.rs`).
- **Apple's bindings compile only the headers each crate lists.** The
  `objc2-*` framework crates have default features off (they'd build every
  header: 127 MB per AppKit build). A missing type or method means adding
  its header (the file in the binding's `src/generated/`, or the
  `cfg(feature)` above the method) to that crate's `Cargo.toml`.
- **In `define_class!`, a method returning `bool` can't `return` early:**
  only the last expression is converted to Objective-C's `BOOL`.
- **The bundle is `LSUIElement`; the app makes itself regular.** Otherwise
  macOS registers every helper as a regular app at launch and the Dock
  flashes a second Kilo. So nothing activates Kilo for free: use
  `app::bring_to_front` (`activate()` is only a request since macOS 14 and
  gets turned down). Check with `lsappinfo listen +all`.
- **A width change only re-frames the page** (`PageView::refresh`): new
  block contents need autoresizing masks that keep them right when the
  page is wider or narrower.
- **Fresh cached pages are shown without parsing a second answer.**
  Swapping in a refetched page right after showing the cached one cost
  2–4 MB of heap fragmentation, so pages under 30 minutes old aren't
  refetched (`pagecache.rs`).
- **`app::with` silently skips when the state is already borrowed.** Don't
  call AppKit methods that can run the event loop (like `NSWindow.close`)
  inside it.

## Style

- Match the surrounding code: concise doc comments, and a `SAFETY:` comment
  on every `unsafe` block (clippy enforces it).
- Keep clippy at `-D warnings` on macOS and Windows, and `cargo fmt` clean
  (CI checks all three).
- End commit messages with the Co-Authored-By line.
