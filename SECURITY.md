# Security

Kilo signs in to your Google account, so it has to be as careful as it is
small. This page says what Kilo protects and how, what it doesn't protect
yet, and how to report a problem.

## Reporting a vulnerability

Please report it privately, through the repository's **Security** tab →
**Report a vulnerability**, rather than in a public issue. Say what you
found, how to reproduce it, and what an attacker could do with it.

## What Kilo protects, and how

**Your password.** You sign in on Google's own page, in a system web view.
Kilo never sees or stores your password.

**The sign-in window.** It has no address bar, so it only shows Google's and
YouTube's pages, over https, and names the page's host in its title bar.
Any other link opens in your browser, where you can see where it goes.

**Your session.**
- Kilo sends it only to music.youtube.com, over TLS that macOS verifies.
  Every request, and every redirect, must be https; redirects never carry
  the session along.
- Thumbnails are downloaded without it.
- **Sign Out** (in the Kilo menu) deletes everything WebKit stores for Kilo
  (the session, site data, caches) and every file Kilo writes.
- The app is signed with the hardened runtime, so other programs can't
  inject code into it or attach to it to read its memory.

**Your Mac.**
- The app itself never runs a web engine. YouTube's player runs in a
  separate helper process, in a web view with Lockdown Mode (no JIT, no
  WebAssembly), images and fonts blocked, and navigation limited to
  YouTube. New windows are refused.
- What the player page reports is checked before Kilo acts on it: video ids
  are validated before they reach a URL or a script, times must be real
  numbers, and the page can't pretend the player stopped.
- Thumbnails are decoded only if they come from Google's image servers, over
  https, and are JPEG, PNG or WebP files. Other formats never reach the
  system's image decoders.
- No telemetry, no other servers, no update channel, no URL handlers.
- Dependencies are few and pinned in `Cargo.lock`, and checked against the
  [RustSec advisory database](https://rustsec.org). CI runs with read-only
  permissions and pinned actions.

## Known limitations

- **The session is stored unencrypted.** WebKit keeps it in its cookie file,
  `~/Library/HTTPStorages/io.github.alperkerimelmas.kilo.binarycookies`.
  Other accounts on the Mac can't reach it, but any program running as you
  can read it. (Chrome encrypts its cookies with a key in the Keychain;
  Safari keeps them in a container macOS protects.)
- **That file holds the whole Google session** that signing in created,
  though Kilo only needs its YouTube part.
- Builds are signed ad hoc, not with a Developer ID, and aren't notarized.
  Build Kilo yourself, or only run a build you trust.

Fixing the first two (keeping only YouTube's cookies, protected by the
Keychain or the App Sandbox) is planned.
