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

## Layout

| Crate | What it is |
|---|---|
| `crates/kilo-core` | Portable: HTTP on the OS's TLS, auth from WebKit's cookie file, YouTube Music web API client, lean serde parsers, models, queue |
| `crates/kilo-player` | The player helper (WKWebView on macOS) and its one-line text protocol |
| `crates/kilo-mac` | The macOS app (AppKit via objc2). The binary `Kilo` also runs as `--player-helper` and `--login-helper` |
| `crates/kilo-probe` | Measures footprint and CPU like the OS task managers, over the process tree plus the XPC services macOS charges to the app |
| `crates/kilo-spike-web`, `crates/kilo-ui-bench` | Measurement labs |

## Commands

```sh
./packaging/macos/bundle.sh --install     # build Kilo.app and copy it to /Applications
cargo clippy --release --workspace -- -D warnings      # also run with --target x86_64-pc-windows-msvc
cargo test --workspace
./target/release/kilo-probe <pid> -d 30 -i 5 --breakdown
```

Developer switches live in `crates/kilo-mac/src/debug.rs`: `KILO_SNAPSHOT`,
`KILO_OPEN`, `KILO_NO_ACTIVATE`, `KILO_SCENARIO`. There's no Screen
Recording permission, so use `KILO_SNAPSHOT` to see the UI.

## Things that bit us

- **Measure `.app` bundles launched with `open`.** When started from a
  terminal, a process's XPC services are charged to the terminal.
- **Don't use `NSSlider`.** On macOS 27 it starts a 40 MB Metal compiler
  service.
- **Don't let a text field auto-focus.** It starts the 11 MB AutoFill
  service. `FocusSink` takes focus instead.
- **Pages use frame layout and lazy blocks:**
  - Use `frame_label` there: it never touches Auto Layout.
  - Only views near the screen exist.
- **A web view only plays media when it's in a window,** so the helper keeps
  it in an offscreen 128×72 window.

## Style

- Match the surrounding code: concise doc comments, and a `SAFETY:` comment
  on every `unsafe` block.
- Keep clippy at `-D warnings` on macOS and Windows.
- End commit messages with the Co-Authored-By line.
