# Changelog

What changed in each version of Kilo. The measurements behind each change
are in [`PLAN.md`](PLAN.md), and the story in
[`HANDOFF.md`](HANDOFF.md).

## Unreleased

A final review before the repository goes public.

### Fixed

- **The player bar's cover went missing** for a track whose art Kilo had
  already shown, as with every album track after the first, or a track
  played again. (A regression from the previous review.)
- The idle shutdown could stop the player while a track chosen after a long
  pause was still loading.
- An answer from a signed-out session that arrived late could end the
  session that replaced it.
- A failed request for more of a list the queue was playing could reload an
  unrelated page, and scroll it back to the top.
- Choosing a playlist, mix, song or card to play did nothing at all once
  YouTube stopped accepting the session; now Kilo shows the sign-in screen,
  as it does for pages.
- One request that failed for its own reasons (an expired continuation, say)
  could stop "up next" requests using their field mask for the rest of the
  session, making each about seven times larger.
- Sign Out deleted all of `~/Library/Application Support/Kilo`, where
  versions up to 0.3 kept the page config, though another app could keep
  its files there too. Now it deletes Kilo's file, and the folder only if
  that leaves it empty.
- The sidebar's ☰ button could stop working until relaunch if the window
  was closed, or changed appearance, while the sidebar was sliding.
- The player helper could take a report about the previous track for the
  new one when YouTube's player announced itself late.
- The sign-in window waited forever if Google's page left it without
  YouTube's cookies.
- Arithmetic that could overflow on absurd thumbnail sizes from the server,
  or on tiny letterboxed thumbnails.

### Security

- Redirects never carry the `Authorization` header: stated in Kilo's own
  HTTP setup rather than left to the library's default.
- CI checks every dependency against the RustSec advisory database, on
  every push and weekly.

### Documentation

- A new README, [`CONTRIBUTING.md`](CONTRIBUTING.md),
  [`ARCHITECTURE.md`](ARCHITECTURE.md) and this changelog; issue and pull
  request templates. Contributor and project documents moved into `docs/`,
  and Claude Code's project instructions into `.claude/CLAUDE.md`.

## 0.3.0 (2026-10-08)

### Added

- A new look, like Spotify's and YouTube Music's: one frame color around
  pages on a rounded panel, round play buttons, a player bar with the
  controls over the progress bar. Cards light up on hover and show one
  shared play button.
- Light and dark, following the Mac or chosen in the View menu.
- English and Turkish, YouTube's content included.
- An account button with the account's photo and name, the settings and
  Sign Out.
- Keyboard shortcuts, from one table for every platform, shown in the
  menus. The mouse's back button goes back.
- A collapsible sidebar (☰, or ⌃⌘S).
- A page cache: Home is on screen 0.15 s after launch (was about 1 s).
- VoiceOver can use the sliders, cards and rows.
- Sign Out, which deletes the session and everything Kilo stored.

### Changed

- **Only YouTube's cookies are stored.** The sign-in window keeps Google's
  account session in memory, and Google's cookies stored by older versions
  are deleted at launch.
- The sign-in window presents itself as the Mac's own Safari version.
- A new page width re-frames the page instead of rebuilding it (3.9 ms →
  0.1–0.8 ms per change).
- Smaller: the app is 1.4 MB (was 2.4 MB), with the same speed and memory.
- Licensed under MIT.

### Security

- The sign-in window shows only Google's and YouTube's pages, over https,
  with the page's host in its title bar; other links open in the browser.
- Thumbnails are decoded only from Google's image servers, over https (also
  where redirects lead), and only as JPEG, PNG or WebP.
- What the player page reports is validated, only its main frame is heard,
  and it can't fake the helper exiting.
- Every request, and every redirect, is https. Cookie domains are matched
  exactly.
- The app is signed with the hardened runtime.

### Fixed

- Signing out, or a session YouTube stopped accepting, now forgets
  everything of the account; work started before writes nothing back, and
  cached pages are kept per sign-in.
- Player commands wait until YouTube's player is on its page, volume first:
  a second track chosen while the page loaded was lost, and the first
  started at the page's own volume.
- A track that hasn't started within 30 s is given up with a message;
  buffering stops the progress clock; a slow cover can't land on the next
  track; of two queues still loading, the one asked for last starts.
- Radios and mixes go on with their own continuation; a playlist keeps its
  repeats, and a click starts at that row.
- A second Kilo no longer appears in the Dock while playback starts.
- The sign-in window comes to the front, and closes if Kilo quits.

## 0.2.0 (2026-10-07)

An efficiency pass; every change was measured.

- **Images are held once:** decoded into IOSurfaces instead of `CGImage`s
  (which Core Animation copies), and cached ones nothing shows are
  purgeable.
- **Closing the window releases it,** and the progress timer runs only
  while the window is visible.
- **Music videos play as their song version** when they have one: the same
  music, 41% less data.
- **Less network:** "up next" answers carry a field mask (800 KB → 110 KB
  of JSON), Home and long playlists load as they scroll, thumbnails are
  q75 JPEG or WebP (33–55% smaller), and the daily config read stops after
  64 KB.
- Smaller HTTP buffers (128 KB → 16 KB per connection).

Result: 31 MB with the window open (was 36–37.5 MB), 147.5 MB playing (was
153 MB).

## 0.1.0 (2026-10-07)

The first macOS app.

- Home, Explore, Library, search, and album, playlist and artist pages, in
  native AppKit views created only near the screen.
- Sign-in through Google's own page, in a helper process.
- Playback through YouTube's own player in a hidden web view, in a helper
  process that's shut down five minutes after playback stops.
- A queue that carries on with each track's radio; long playlists load in
  full.
- Media keys and Control Center; music keeps playing with the window
  closed.
- `kilo-probe`, which measures memory and CPU like the OS task managers.
