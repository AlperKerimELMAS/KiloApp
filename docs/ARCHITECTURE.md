# Architecture

How Kilo is put together, for anyone about to change it, and why each
piece is the way it is: the measurements behind the decisions are at the
end ([Decisions, measured](#decisions-measured)).

## One binary, five roles

The macOS app is a single executable that starts itself again, with an
argument, for the work it keeps out of its own process:

```
Kilo.app/Contents/MacOS/Kilo
├─ (no arguments)     the app: native AppKit UI; never loads WebKit
├─ --player-helper    YouTube's own player in a hidden web view (kilo-player)
├─ --login-helper     Google's sign-in page in a window; exits once signed in
├─ --prune-helper     deletes WebKit's data for Kilo except YouTube's, then exits
└─ --sign-out-helper  deletes all of WebKit's data for Kilo, then exits
```

WebKit costs tens of megabytes that it never gives back to the process
that loaded it. Running it only in helpers is what lets Kilo sit at about
30 MB when nothing plays: the player helper is started on the first play
and shut down five minutes after playback stops, and the others live for
one job.

Every helper ends with Kilo: the player and sign-in helpers read a pipe
from the app on stdin and exit when it closes, and the data helpers give
up after 15 seconds.

## Crates

| Crate | What it holds |
|---|---|
| [`kilo-core`](../crates/kilo-core) | YouTube Music's web API (`client`), HTTP on macOS's TLS (`http`), the sign-in session (`auth`), parsers (`parse`) and models (`model`), the queue, the thumbnail policy (`image`). No UI code |
| [`kilo-player`](../crates/kilo-player) | The player helper's line protocol (`protocol`), the app's side of it (`host`), and the helper itself (`helper.rs`) |
| [`kilo-mac`](../crates/kilo-mac) | The app: state (`app/`), views (`ui/`, with the colors in `ui/theme.rs`), words (`strings.rs`), keyboard shortcuts (`shortcuts.rs`, and AppKit's side in `keys.rs`), images, page cache, sign-in and data helpers (`login.rs`), paths, settings, developer switches |
| [`kilo-probe`](../crates/kilo-probe) | Measures memory and CPU like Activity Monitor; no dependencies |
| [`kilo-spike-web`](../crates/kilo-spike-web), [`kilo-ui-bench`](../crates/kilo-ui-bench) | Measurement labs behind the [decisions below](#decisions-measured); not part of the app |

`kilo-mac` depends on `kilo-core` and `kilo-player`; `kilo-player` and
`kilo-probe` depend on neither. `kilo-core` uses neither AppKit nor
WebKit, so its tests run without a window.

## The app

### State and threads

All of the app's state is one `App` value (`app/mod.rs`) in a
thread-local on the main thread, reached through `app::with(|a| …)`.
`with` hands out a mutable borrow, and returns `None` when the state is
already borrowed, which happens when AppKit calls back into Kilo from
inside a `with`. Two rules follow:

- Don't call AppKit methods that can run the event loop (closing a window,
  opening a menu) inside `with`; AppKit's callbacks step out first with
  `net::later`.
- Don't call `images::load` inside `with` if its callback needs the state:
  an image that's already decoded is handed back before `load` returns.

Background work goes through `net` (`net.rs`):

- `net::run(Pool::Api | Pool::Images, job, done)`: `job` runs on one of two
  small worker pools (two threads each, so thumbnails never delay a click),
  and `done` gets its result on the main thread, through GCD.
- `net::spawn(job, done)`: the same on a thread of its own, for work that
  waits on another process (the sign-in window, a helper quitting, the
  data helpers), which must neither wait behind page loads nor hold them
  up.
- `net::later(f)`: runs `f` on the main thread after the current event.

Late answers are the main source of bugs in an app like this: a page that
arrives after the user moved on, a queue that loads after another one
started, a request from an account that has since signed out. Each kind of
work carries a token taken when it started, and its completion does
nothing if the token has moved on:

| Token | Bumped when | Guards |
|---|---|---|
| `generation` | a page load starts | page answers for a page that's no longer wanted |
| `play_intent` | the user asks for something to play | of two queues still loading, only the last one asked for starts |
| `queue_id` | a new queue starts | more of a list, radio or mix arriving for an old queue |
| `play_token` | the track changes | a slow song-version lookup, or the load watchdog, for an old track (a slow cover is checked against the current track's) |
| `player_serial` | a helper is started, or the account is let go of | events from a helper that has been replaced |
| `idle_token` | playback starts | an idle shutdown that's no longer due |
| `session` | the account is let go of (`session::forget`) | connecting, the account's name and photo, more of a page |
| `paths::epoch` | the account is let go of | disk writes from work started before (`paths::write_if_current`; deleting the account's files waits for writes under way) |

### Launch

1. `main` reads the Mac's Safari version, which Kilo's browser identity
   names (Google turns away sign-in windows whose claimed Safari doesn't
   match their engine), then runs the app.
2. The app turns itself into a regular app (the bundle is `LSUIElement`, so
   that the helpers never appear in the Dock), applies the appearance and
   language, and builds the menu bar and the window.
3. `session::resume` reads WebKit's cookie file. If an older Kilo left
   Google's account cookies in it, the prune helper deletes them first.
4. `session::start` connects, showing a loading skeleton:
   music.youtube.com's page config (client version, visitor id, region)
   comes from its cache, however old, and is refreshed in the background
   once a day; only the very first launch waits for it. Then Home opens
   (from the page cache, if it's there), and the account's name and photo
   load. Without a session, the welcome screen offers to sign in.

### Pages

A `Route` (Home, Explore, Library, a browse id, a search) is loaded by
`app/browse.rs`:

- **The page cache** (`pagecache.rs`) keeps YouTube's answers as they came,
  one file per page, language and sign-in, 16 MB at most, for a week. A
  page fetched in the last half hour is shown again without a request; an
  older one is shown at once and replaced by a fresh copy if the user
  hasn't scrolled it and it changed. Reload (⌘R) always asks YouTube.
- **Parsing** (`kilo_core::parse`) deserializes the JSON straight into
  narrow structs that name only what Kilo shows: no JSON tree is built, and
  a page takes about a millisecond. The result is a `Page`: an optional
  header and sections (`Cards`, `List`, `Grid`, `Text`) of `Entry`s, each
  with a `Target` (play, play a playlist, open a page).
- **The page view** (`ui/page.rs`) is laid out by hand, without Auto
  Layout: the page is cut into blocks (header, titles, shelves, rows) with
  known heights, and only blocks within half a screen of the visible area
  exist as views. Shelves create their cards the same way as they scroll
  sideways. A width change re-frames what exists instead of rebuilding it.
- **More of a page** loads as its end comes into view: the next shelves
  (`More::Page`) or the next rows of a long list (`More::List`), by
  continuation token. The grown page replaces the old one, keeping the
  scroll position and the views that didn't move.

### Thumbnails

`model::Thumb::sized(px)` turns a thumbnail into a URL for exactly the
pixels a view draws (Google's image servers resize on request). Then
`images::load(url, px, done)`:

1. Returns the decoded image at once if it's in the memory cache (keyed by
   URL and size), and otherwise joins the request already under way for it,
   if any.
2. On an image worker: refuses URLs that aren't Google's image servers over
   https (`kilo_core::image::trusted_url`), reads the disk cache (64 MB,
   trimmed as it grows), or downloads the image without the session, and
   checks where any redirect led. Only JPEG, PNG and WebP bytes go further
   (`image::known_format`).
3. ImageIO decodes it at `px` straight into an IOSurface, which Core
   Animation shows without copying (a `CGImage` as layer contents is
   copied). The black bars of YouTube's 4:3 video frames are cropped away.
4. Decoded images that no view shows are marked purgeable: they don't count
   toward Kilo's memory, and macOS can take them back (Kilo then decodes
   again). Closing the window drops them all.

### Playback

**The queue** belongs to Kilo (`kilo_core::queue`, `app/queue.rs`):
YouTube's player only ever plays the one track it's told to, so next,
previous and autoplay are local and instant. A queue starts from:

- a row of a list: the list from that row, with the rest of a long list
  joining as the queue gets to it (`Follow`);
- Play or Shuffle on an album or playlist page, or a card's play button;
- a playlist or mix, through YouTube's "up next" (`Client::next_playlist`);
- a single song, with its radio (`Client::next`).

When three tracks or fewer are left, `queue::extend` asks for more: the
list's next part, or more of the radio or mix the queue came from
(`Endless`); once there's neither, the last track's own radio. A music video is played
as its song version when it has one (`Client::next_single`): Kilo never
shows video, and a song's "video" is a still picture, 41% less data.

**The player helper** is started on the first play (`app/player.rs`).
The app sends it one command per line and reads one event per line
(`kilo_player::protocol`):

| App → helper | Helper → app |
|---|---|
| `load <video id> <seconds>` | `ready` |
| `play`, `pause` | `playing`, `paused`, `buffering`, `ended` `<seconds> <duration> <video id>` |
| `seek <seconds>` | `next`, `previous` (media keys, Control Center) |
| `volume <0–100>` | `signed-out` (it exits), `ad` (it pauses) |
| `quit` | `error <text>` |

The host adds `Exited` itself when the helper's output ends, so nothing
the page says can fake it. Video ids are validated to 11 safe characters
before they reach a URL or a script, and times must be finite.

While a track plays the helper sends nothing: the app extrapolates the
position, and moves the progress bar once a second only while the window
is visible. A track that hasn't started 30 seconds after it was asked for
is given up with a message. Five minutes after playback stops, the helper
is shut down; the next play starts a fresh one where playback left off.

**Inside the helper** (`kilo-player/src/helper.rs`), a `WKWebView`
runs m.youtube.com's player, the lightest page that runs YouTube's full
player, set up for the smallest footprint measured:

- in Lockdown Mode (no JIT, no WebAssembly), at 128×72 (the 144p stream),
  in a borderless window off screen (WebKit plays media only for a view in
  a window), with the page's own interface hidden by CSS, and images and
  fonts blocked; nothing else is;
- with a small bridge script that reports the `<video>` element's events
  and drives YouTube's player through its own API (`loadVideoById`,
  `playVideo`…); only the page's main frame is heard, and only short
  messages;
- commands wait until the page says YouTube's player is there, then run in
  order, the volume first;
- anything that starts playing without having been asked for (the mobile
  site's autoplay) is paused, and an ad pauses playback and is reported;
- main-frame navigation is limited to YouTube's and Google's hosts, and
  new windows are refused;
- it exits when the app closes its stdin, when the page fails to load, or
  when WebKit's content process dies; the app starts a fresh one on the
  next play.

### Accounts

**Signing in** (`app/session.rs`, `login.rs`): the login helper opens
Google's sign-in page in a window with no address bar, so it only shows
the hosts Google's sign-in uses (`kilo_core::auth::is_sign_in_host`, an
exact list), over https (anything else opens in the browser), and names
the page's host in its title bar. The window's
WebKit store lives in memory only, so the Google account session that
signing in creates never touches the disk. Once music.youtube.com says the
user is signed in, the helper copies YouTube's cookies, and only those,
into WebKit's persistent store for the player helper, prints them for the
app, and exits. WebKit writes its cookie file as the process exits, so the
app waits for the file (`login::wait_until_saved`) before it connects.

**The session** (`kilo_core::auth::Session`) is the YouTube cookies that
apply to music.youtube.com. Each API request carries them and the
`SAPISIDHASH` signature YouTube's own web client sends, and goes only to
music.youtube.com. Redirects never carry them. `Session::fingerprint`
tells sign-ins apart for the page cache's file names. It lasts a week at
most: Google gives youtube.com's copies of the session cookies 7 days
(its own get 400), and renews them only from its own, which Kilo doesn't
keep.

**Letting go of an account** happens two ways. When YouTube stops accepting
the session, `session::session_ended` forgets the account and shows "Sign
in again". YouTube rarely says so with a 401 or 403: it answers as to a
guest, `logged_in` 0 in the answer's `responseContext`, which the parsers
turn into `Error::SignedOut` (browse, search, and the account menu, asked
at every connect); the player page's `LOGGED_IN` false makes the helper
say `signed-out` instead of `player`. To forget the account,
`session::forget` clears everything of it in memory, bumps every token above, and starts a new
`paths::epoch`. **Sign Out** does the same, then stops the player helper,
has the sign-out helper empty WebKit's stores for Kilo, and deletes Kilo's
own files (`paths::remove_own_data`). If anything couldn't be deleted, the
screen says so and offers to try again.

### The window

The window exists only while it's open (`app/mod.rs`): closing it
releases every view, layer and decoded image, and reopening builds a fresh
one from the app's state (`Screen` says what the page area showed). A new
appearance or language rebuilds the window's views in place, keeping the
page and how far it was scrolled.

The frame (sidebar, player bar, the panel the page sits on) uses Auto
Layout (`ui/shell.rs`); pages use frame layout. Sliders are two plain
layers (`ui/bar.rs`): `NSSlider` would start a 40 MB system service. An
invisible `FocusSink` holds the keyboard focus, so the search field (whose
focus starts the 11 MB AutoFill service) is focused only when asked for.

Keyboard shortcuts come from one table (`shortcuts.rs`): menu items carry
the ones they show, and the focus sink hears the rest (Space, M, the
arrows; `keys.rs`).
While the search field has the focus, plain keys and the arrows go to the
text (`validateMenuItem:` in `ui/actions.rs`).

## On disk

Everything is keyed by the running bundle id (`paths.rs`), which is what
WebKit keys its stores by too, so a copy with another bundle id is a
separate install.

| What | Where | Written by |
|---|---|---|
| The session | `~/Library/HTTPStorages/<bundle id>.binarycookies` | WebKit (the login and player helpers) |
| WebKit's other data | `~/Library/WebKit/<bundle id>`, `~/Library/Caches/<bundle id>` | WebKit |
| music.youtube.com's page config | `~/Library/Application Support/<bundle id>/innertube.txt` | the app, once a day |
| Thumbnails, 64 MB at most | `~/Library/Caches/<bundle id>/images` | the app |
| Pages, 16 MB at most | `~/Library/Caches/<bundle id>/pages` | the app |
| Settings (appearance, language, sidebar) | the defaults store | the app |

Files are written atomically (to a temporary file, then renamed), and only
if the account they belong to is still the current one.

## Decisions, measured

Every choice above was measured with `kilo-probe` on one Mac (Apple
Silicon, macOS 27), launched as an `.app` with `open` so macOS charges its
system services to it. Footprint is what Activity Monitor shows.

### Playing: YouTube's player, hidden

Kilo doesn't decode media itself (a ground rule), so the question was
which of YouTube's own pages plays music for the least. The page alone, in
a hidden `WKWebView` (`crates/kilo-spike-web`), playing:

| Page | Footprint |
|---|---|
| music.youtube.com | 518 MB |
| music.youtube.com, in Lockdown Mode | 345 MB |
| m.youtube.com | 200 MB |
| m.youtube.com, in Lockdown Mode, in a 128×72 view (the 144p stream) | **113–122 MB** |
| the same without Lockdown Mode (JavaScript JIT on) | 200 MB |

- **In a helper process.** After its web view is gone, WebKit leaves about
  48 MB in the process that loaded it. The helper quits five minutes after
  playback stops, and then Kilo is back to about 30 MB; playing again
  takes about 2 s.
- **Song versions of music videos.** m.youtube.com always streams a video
  track, even hidden (music.youtube.com streams audio only, but costs
  398 MB). A song's "video" is a still picture: playing a music video's
  song version took 3.0 MB/min down to 1.8 MB/min (−41%).
- **Hiding the page's own interface** (CSS) saves about 10% of its memory;
  hiding the video element saves compositing (17 → 10 wakeups a second),
  though WebKit still decodes it.
- Doesn't help: WebKit's "suspend" scheduling (a page that has played
  media keeps running) and hiding the window (same CPU).

### The UI

| Decision | Measured |
|---|---|
| AppKit, not a cross-platform toolkit | The same window: AppKit 27 MB, Slint's software renderer 84 MB, Slint's GPU renderer 171 MB (`crates/kilo-ui-bench`) |
| No `NSSlider` (`ui/bar.rs` instead) | On macOS 27 it starts Metal's shader compiler service: +40 MB, and +5 MB in the app |
| An invisible `FocusSink` holds the focus | A focused text field starts the AutoFill service: +11 MB |
| Lazy pages, laid out by hand | Views for whole pages cost about 24 MB: creating only what's near the screen took the UI from 61 to 37 MB |
| Thumbnails decoded into IOSurfaces | A `CGImage` as layer contents is copied: Home's thumbnails took 6.2 MB, now 3.4 MB. Images nothing shows are purgeable (up to 16 MB, not counted) |
| The window is released when closed | 29–38.5 MB → 26.6–27.6 MB with the window closed |
| The progress timer runs only while the window is visible | Playing with the window closed: 0.3% → 0.0% CPU, 3.5 → 0.2 wakeups a second |
| One plain frame color: no blur, no Liquid Glass, no shadows | Behind-window blur +0.2 MB, `NSGlassEffectView` +1.2 MB per element, layer shadows +0.4 MB, a clipping panel +0.4 MB. Without the blur and the system search bezel, an album page went from 29.2–29.7 to 28.2–28.5 MB |
| A width change re-frames the page | Per change: Home 3.9 → 0.8 ms, an album 3.9 → 0.2 ms |

### Network and startup

| Decision | Measured |
|---|---|
| A field mask on "up next" requests (`X-Goog-FieldMask`) | A radio: 800 KB → 110 KB of JSON (36 → 8 KB on the wire); the account: 5.6 KB → 0.4 KB |
| Thumbnails at their exact size, JPEG at quality 75 or WebP | 33–55% smaller, no visible difference |
| The page config read from the first 64 KB of music.youtube.com | Instead of 549 KB, once a day |
| Pages cached on disk as YouTube sent them | Home on screen 0.93–1.05 s → 0.15–0.17 s after launch |
| Fresh cached pages (under 30 minutes) aren't fetched again | Swapping in a refetched page right after the cached one cost 2–4 MB of heap fragmentation |
| HTTP buffers of 16 KB | ureq's default, 128 KB each way per connection, kept 1.5 MB alive |
| `opt-level = "z"` | The binary 1,624 → 1,165 KB, with the same startup and memory |

The comparison with YouTube Music in Chrome is in
[`COMPARISON.md`](COMPARISON.md).
