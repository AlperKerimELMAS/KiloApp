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
| `crates/kilo-core` | Portable: HTTP on the OS's TLS, auth from WebKit's cookie file, YouTube Music web API client, lean serde parsers, models, queue |
| `crates/kilo-player` | The player helper (WKWebView on macOS) and its one-line text protocol; `examples/session.rs` is a scripted, measured session |
| `crates/kilo-mac` | The macOS app (AppKit via objc2). The binary `Kilo` also runs as `--player-helper`, `--login-helper` and `--sign-out-helper`. `src/app/` holds the state (`mod.rs`), pages (`browse.rs`), queue, player and sign-in |
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
```

Developer switches live in `crates/kilo-mac/src/debug.rs`: `KILO_SNAPSHOT`,
`KILO_OPEN`, `KILO_NO_ACTIVATE`, `KILO_SCENARIO`. There's no Screen
Recording permission, so use `KILO_SNAPSHOT` to see the UI. `KILO_SCENARIO`
can also browse, scroll, play (`volume:0` first to stay silent) and print
the page and queue state, which is how lazy loading and the queue are
tested.

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
  Only "up next" requests carry a mask.
- **macOS 27 quits an idle helper itself** ("quiet safe quit", SIGTERM) a
  few minutes after playback stops, before or after Kilo's own 5-minute
  idle shutdown. Both are fine: the next play starts a fresh helper.
- **Test sign-in and sign-out in a copy with another bundle id** (see
  `docs/HANDOFF.md`, section 6). The real app's helpers use the owner's
  session.
- **`app::with` silently skips when the state is already borrowed.** Don't
  call AppKit methods that can run the event loop (like `NSWindow.close`)
  inside it.

## Style

- Match the surrounding code: concise doc comments, and a `SAFETY:` comment
  on every `unsafe` block (clippy enforces it).
- Keep clippy at `-D warnings` on macOS and Windows, and `cargo fmt` clean
  (CI checks all three).
- End commit messages with the Co-Authored-By line.
