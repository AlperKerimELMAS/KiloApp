# Contributing to Kilo

Thanks for wanting to help. Kilo has a narrow goal, the smallest YouTube
Music client we can build, and a few rules that come with it. Please read
this page before you start: a change that breaks a ground rule won't be
merged, however good it is otherwise.

For how the code fits together, read [`ARCHITECTURE.md`](ARCHITECTURE.md).
For why it is the way it is, [`PLAN.md`](PLAN.md) has every
decision with the measurement behind it.

## Ground rules

1. **Kilo never touches media.** Audio plays only through YouTube's own
   player, in a hidden system web view, in a helper process. No code that
   downloads, records, decodes or otherwise processes YouTube's media
   streams. This was decided for legal and safety reasons, and isn't up
   for debate.
2. **Playback is Premium-only.** The player helper stops if an ad appears:
   an ad must never play unseen.
3. **No downloads, no ad blocking, no YouTube branding** (no logos, no
   "YouTube" in Kilo's name, no YouTube Sans). Kilo describes itself as an
   unofficial client.
4. **Efficiency changes come with numbers.** A change made to save memory,
   CPU, network or disk is kept only if `kilo-probe` shows it measurably
   wins (see [Measuring](#measuring)). New features have their cost
   measured too.
5. **Security boundaries stay where they are** ([`SECURITY.md`](SECURITY.md)):
   the sign-in window and the player only show allowlisted https hosts;
   thumbnails pass `kilo_core::image` before any image decoder sees them;
   whatever the player page says is validated; only YouTube's cookies are
   ever stored; the app is signed with the hardened runtime.

## Getting started

You need macOS 14 or later, the Xcode Command Line Tools
(`xcode-select --install`) and [rustup](https://rustup.rs). The Rust
version is pinned in `rust-toolchain.toml`; rustup installs it on the first
build.

```sh
./packaging/macos/bundle.sh            # builds dist/Kilo.app
open dist/Kilo.app
./packaging/macos/bundle.sh --install  # also copies it to /Applications
```

Run Kilo as an app bundle, with `open`: a binary started from a terminal
works, but macOS then charges its system services to the terminal, and
some things (activation, the Dock) behave differently.

### Testing sign-in and sign-out without losing your own session

Kilo, its helpers and WebKit key everything by the app's bundle id: the
cookie store, the caches, the settings. To try signing in or out, use a
copy with a bundle id of its own:

```sh
cp -R dist/Kilo.app /tmp/KiloTest.app
plutil -replace CFBundleIdentifier -string io.github.example.kilotest /tmp/KiloTest.app/Contents/Info.plist
codesign --force --options runtime --sign - /tmp/KiloTest.app
open -n /tmp/KiloTest.app
```

Afterwards, remove what it stored:

```sh
rm -rf ~/Library/{WebKit,Caches,HTTPStorages}/io.github.example.kilotest* \
       ~/Library/Application\ Support/io.github.example.kilotest
defaults delete io.github.example.kilotest
```

## Before you open a pull request

```sh
cargo fmt --all
cargo clippy --release --workspace --all-targets -- -D warnings
rustup target add x86_64-pc-windows-msvc   # once
cargo clippy --release --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace
```

CI runs the same checks, plus the portable crates on Linux and a check of
the dependencies against the [RustSec advisory database](https://rustsec.org).
The Windows clippy run keeps the portable crates (`kilo-core`,
`kilo-player`, `kilo-probe`) building for the front ends to come.

In the pull request, say what you changed and why, how you tested it, and,
for anything that could affect memory, CPU, network or disk, the numbers
before and after.

## Measuring

`kilo-probe` measures memory and CPU the way each OS's task manager does
(Activity Monitor's "Memory" on macOS), summed over a process tree, plus
the system services macOS charges to the app.

```sh
cargo build --release -p kilo-probe
./target/release/kilo-probe <pid|name> -d 30 -i 5 --breakdown   # 30 s, a sample every 5 s
./target/release/kilo-probe <pid> --list                        # the processes it counts
```

The scripts launch a copy of Kilo of their own (never touching one that's
already running), drive it, and measure it:

```sh
scripts/measure.sh dist/Kilo.app wait:15                    # window open, Home loaded
scripts/measure.sh dist/Kilo.app wait:12,close,wait:20      # window closed
scripts/measure.sh dist/Kilo.app --play lYBUbBu4W08         # playing (also counts network bytes)
scripts/startup.sh dist/Kilo.app 4                          # when the window, Home and the first image appear
```

`--play` plays the track on your account, out loud: a muted player could
measure differently.

Some habits that keep numbers honest:

- **Measure app bundles launched with `open`.** A process started from a
  terminal has its XPC services (WebKit's, AutoFill, Metal's shader
  compiler) charged to the terminal, and they disappear from the numbers.
- **Compare like with like:** the same page, window size and display, and
  several launches each (alternating builds), because what Home shows
  changes from run to run. Report the medians or the range.
- **Don't measure a run that takes snapshots** (`KILO_SNAPSHOT`): rendering
  the window inflates the footprint.
- `heap` and `malloc_history` can't read a hardened app. To diagnose memory,
  re-sign a copy without `--options runtime`.

## Developer switches

Kilo can be driven without a mouse or a screen, through environment
variables: `KILO_SCENARIO` runs a script of steps (play, browse, scroll,
close, key presses, snapshots…) and logs each one, `KILO_SNAPSHOT` renders
the window to PNG files (no Screen Recording permission needed), and a few
others. They're listed in
[`crates/kilo-mac/src/debug.rs`](../crates/kilo-mac/src/debug.rs). For
example:

```sh
open -n -g -o /tmp/kilo.log --stderr /tmp/kilo.log \
  --env KILO_NO_ACTIVATE=1 --env KILO_SNAPSHOT=/tmp/shot \
  --env KILO_SCENARIO=wait:3,state,scroll:end,wait:2,snap:now dist/Kilo.app
```

## Code style

- **Match the code around you:** short, plain doc comments that say what a
  thing is for; one idea per line (`rustfmt.toml` allows 140 columns).
- **Every `unsafe` block has a `// SAFETY:` comment** saying why it's sound
  (clippy enforces it).
- **What every front end shares lives in `kilo-core`:** new UI text goes in
  `kilo_core::strings`, in every language Kilo speaks (English and Turkish
  today); colors in `kilo_core::style`; keyboard shortcuts in
  `kilo_core::shortcuts`.
- **Dependencies:** few, and each one has to justify its size. Prefer the
  OS's own facilities (Kilo uses the system's TLS, ImageIO, WebKit).
- **Apple's bindings are trimmed:** the `objc2-*` crates have their default
  features off (otherwise each would compile every header). If a type or
  method is missing ("cannot find …", "no method named …"), add its header
  to the crate's `Cargo.toml`: the file in the binding's `src/generated/`
  that defines it, or the `cfg(feature = …)` above the method.

## Pitfalls

Things that cost us time, mostly macOS behavior that isn't obvious:

- **Some system controls are expensive.** `NSSlider` starts a 40 MB Metal
  shader compiler service on macOS 27 (Kilo draws its own, `ui/bar.rs`); a
  focused text field starts the 11 MB AutoFill service (an invisible
  `FocusSink` takes the focus instead); `NSGlassEffectView` is +1.2 MB per
  element and layer shadows +0.4 MB.
- **Pages use frame layout and lazy blocks.** Only views near the screen
  exist. Use `frame_label` there: AppKit gives every label about 9
  autoresizing constraints even in a window without Auto Layout. A width
  change re-frames the page (`PageView::refresh`) rather than rebuilding
  it, so a block's contents need autoresizing masks, and the block its
  final size before they're added.
- **Never set a `CGImage` as layer contents:** Core Animation copies it.
  Thumbnails are decoded into IOSurfaces (`images.rs`).
- **Image views need `clipsToBounds`,** or rounded corners stay square and
  wide images spill out of square slots.
- **App state lives in one place, reached through `app::with`,** which
  silently does nothing if the state is already borrowed. So don't call
  AppKit methods that can run the event loop (`NSWindow.close`, a menu)
  inside it, and don't call `images::load` inside it when the callback
  needs the state: a cached image is handed back before `load` returns.
- **Work that waits on another process** (a helper, the sign-in window)
  runs on a thread of its own (`net::spawn`), never on the two
  page-loading workers (`net::run`).
- **An account's data has boundaries.** Signing out, or a session YouTube
  stopped accepting, goes through `session::forget`; disk writes from
  background work go through `paths::write_if_current` with the epoch taken
  when the work started; completions check that the session is still the
  same.
- **WebKit writes cookies to disk only when its process exits.** A helper
  that sets cookies must exit before anyone reads the file.
- **A web view plays media only while it's in a window,** so the player
  helper keeps it in an offscreen 128×72 one.
- **The player helper holds commands until YouTube's player is on the
  page** (`player`); anything that drives the player goes through `handle`.
- **YouTube's API honors `X-Goog-FieldMask`, but strictly:** an unknown
  field name is a 400, and nested `*` wildcards make it take seconds or
  time out. Only "up next" and account requests carry a mask.
- **The bundle is `LSUIElement`, and the app makes itself a regular app**
  at launch, so the helpers never flash in the Dock. Since macOS 14,
  `activate()` is only a request: use `app::bring_to_front`.
- **Light/dark and the language are read when views are made;** a change
  rebuilds the window's views (`app::rebuild_window`).
- **Plain-key shortcuts must not reach the search field:** Space, M and the
  arrows act only when it isn't focused (`validateMenuItem:` in
  `ui/actions.rs`).
- **In `define_class!`, a method returning `bool` can't `return` early:**
  only its last expression is converted to Objective-C's `BOOL`.
- **macOS 27 quits an idle helper by itself** ("quiet safe quit") a few
  minutes after playback stops. That's fine: the next play starts a fresh
  one.

## Adding a language

Kilo's words are in `crates/kilo-core/src/strings.rs`, one line per phrase
with a column per language. To add one, add a column to that table (and to
the `strings!` macro that reads it) and a variant to `Language`, then teach
the macOS front end to offer it:
`settings::Language`, the language menu in `ui/menus.rs`, and
`strings::init`. YouTube's own content follows through the `hl` Kilo asks
for (`settings::content_language`).

## Reporting bugs and vulnerabilities

Open an issue for bugs and ideas. Please don't paste the contents of
Kilo's cookie file or caches: they hold your session and your library.

Report vulnerabilities privately, as [`SECURITY.md`](SECURITY.md) explains.

By contributing, you agree that your contributions are licensed under the
[MIT license](../LICENSE).
