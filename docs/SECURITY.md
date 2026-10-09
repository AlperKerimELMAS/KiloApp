# Security

Kilo signs in to your Google account, so it has to be as careful as it is
small. This page says what Kilo protects and how, what it doesn't protect
yet, and how to report a problem.

## Reporting a vulnerability

Please report it privately, through the repository's **Security** tab →
**Report a vulnerability**, rather than in a public issue. Say what you
found, how to reproduce it, and what an attacker could do with it. If you
can't use that form, open an issue asking for a way to reach the
maintainer, without any details of the problem.

## Supported versions

Fixes go into the latest version, on the `main` branch. Kilo has no
prebuilt releases yet: update by building the latest code.

## What Kilo protects, and how

**Your password.** You sign in on Google's own page, in a system web view.
Kilo never sees or stores your password.

**The sign-in window.** It has no address bar, so it only shows the pages
Google's sign-in uses (an exact list of hosts), over https, and names the
page's host in its title bar. Any other link you click, including the
rest of google.com, where anyone can publish a page, opens in your
browser, where you can see where it goes. Anything else headed elsewhere
(a redirect, a frame) is refused: its address may carry a sign-in token.

**Your session.**
- Signing in creates your whole Google account session. Kilo keeps it in the
  sign-in window's memory only, and it ends with that window. Kilo stores
  only YouTube's cookies, which is all it needs. (Older versions stored the
  whole session; Kilo deletes the rest at launch.)
- Kilo's own requests send the session only to music.youtube.com, over TLS
  that macOS verifies. Every request, and every redirect, must be https;
  redirects never carry the session along. (The player helper's web view
  sends YouTube's cookies to YouTube, as a browser does.)
- Thumbnails are downloaded without it.
- **Sign Out** (the account button, or the Kilo menu) deletes everything
  WebKit stores for Kilo (the session, site data, caches) and every file
  Kilo writes, copies of the cookie file included. A session that ended
  (see below) is erased the same way. If something can't be deleted, Kilo
  says so and offers to try again. Work still under way at that moment (a page, a thumbnail)
  writes nothing back, and what one sign-in cached is never shown to
  another.
- Sign Out ends the session on this Mac, not at Google: ending it there
  takes Google's own cookies, which Kilo doesn't keep (YouTube's sign-out
  without them was tested, and leaves the session working). To end it
  there too (if the cookie file may have been copied, say), sign that
  device out in your Google Account: **Security → Your devices**. Kilo
  notices and asks you to sign in again.
- The session lasts a week at most: Google gives YouTube's copy of it 7
  days, and only Google's own cookies can renew it. Then Kilo asks you to
  sign in again. Whether Google would still take a copy of the cookies
  after that week hasn't been tested, so don't count on it: signing the
  device out (above) is what's sure.
- Cookie values never appear in Kilo's logs or debug output, and what
  Kilo writes itself (pages, thumbnails, settings from YouTube) only your
  account can read.
- Kilo and its player connect only to Google's servers: YouTube's, and
  the video caches Google places inside internet providers' networks.
- The app is signed with the hardened runtime, so other programs can't
  inject code into it or attach to it to read its memory.

**Your Mac.**
- The app itself never runs a web engine. YouTube's player runs in a
  separate helper process, in a web view with Lockdown Mode (no JIT, no
  WebAssembly), images and fonts blocked, and navigation limited to
  YouTube. New windows are refused. Only the page itself can talk to Kilo,
  not the frames embedded in it.
- What the player page reports is checked before Kilo acts on it: video ids
  are validated before they reach a URL or a script, times must be real
  numbers, and the page can't pretend the player stopped.
- Thumbnails are decoded only if they come from Google's image servers, over
  https (checked again where a redirect leads), and are JPEG, PNG or WebP
  files. Other formats never reach the system's image decoders.
- Kilo adds no telemetry of its own (YouTube's player reports playback to
  YouTube, as on its website), and has no other servers, no update
  channel, no URL handlers.
- Dependencies are few and pinned in `Cargo.lock`. CI checks them against
  the [RustSec advisory database](https://rustsec.org) on every push and
  every week, runs with read-only permissions, and pins its actions.

## Known limitations

- **The YouTube session is stored unencrypted.** WebKit keeps it in its
  cookie file,
  `~/Library/HTTPStorages/io.github.alperkerimelmas.kilo.binarycookies`.
  Other accounts on the Mac can't reach it, but any program running as you
  can read it, and with it act on your YouTube account. Google's own
  cookies (for Gmail, Drive…) aren't in it, but YouTube's still belong to
  your Google account's sign-in: treat the file like a password, and if it
  may have been copied, end the session at Google (above). Chrome encrypts its cookies with a key in the Keychain;
  Safari keeps them in a container macOS protects. Both rely on the app
  having a stable code signature, which ad-hoc builds don't: each build
  would have to ask for your Mac password to read the session. Encrypting
  it comes with Developer ID signing.
- Builds are signed ad hoc, not with a Developer ID, and aren't notarized.
  Build Kilo yourself, or only run a build you trust.
- The measurement lab `kilo-spike-web` (not part of the app) has a `login`
  command that, like the app, keeps YouTube's cookies in its own WebKit
  store, but its sign-in window isn't limited to Google's pages. It's a
  development tool: use it only if you know why, and delete its data
  afterwards.
