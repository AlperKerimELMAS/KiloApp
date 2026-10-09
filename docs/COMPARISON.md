# Kilo vs. YouTube Music in Chrome

Measured on 2026-10-07 on one machine: MacBook (Apple Silicon, 16 GB),
macOS 27.0.1, Retina display, YouTube Music Premium. Both apps were playing
music. Kilo was measured twice the same day: as first built (0.1), and after
the efficiency pass (0.2, [`CHANGELOG.md`](CHANGELOG.md)). Chrome wasn't
re-measured.

## Results

| | YouTube Music (Chrome app) | Kilo 0.1 | Kilo 0.2 | Kilo 0.2 uses |
|---|---|---|---|---|
| **Memory, playing, window open** | **1,088 MB** (1,035–1,152) | 153 MB (152–154) | **147.5 MB** (145–152)¹ | **7.4× less** |
| Processes | 18–20 | 8 | 8 | |
| CPU, playing (% of one core) | 8.9% | 8.9% (8.5% re-measured) | **8.2%** | slightly less |
| Wakeups per second, playing | 258 | 121 | 114 | 2.3× fewer |
| Playing with the window closed | not possible (closing the window stops music) | 147 MB, 7.6% CPU | **143 MB, 6.5% CPU** | |
| Paused, first 5 minutes | not measured | 143 MB, 0.08% CPU, 2 wakeups/s | **142 MB, 0.0% CPU, 0 wakeups/s** | |
| Paused for 5+ minutes | not measured | 34 MB, 0.00% CPU, 0 wakeups/s | **31 MB, 0.00% CPU, 0 wakeups/s** | |
| Not playing, window open (Home) | | 36–37.5 MB | **31 MB** | |
| Not playing, window closed | | 29–38.5 MB | **27 MB** | |
| Network while playing a song | 1.2 MB/min | 1.9 MB/min | about the same² | |
| Network while playing a music video | | 3.0 MB/min | **1.8 MB/min** (plays its song version) | |
| App on disk | 1.4 GB (Chrome) | 2.3 MB | 2.4 MB | 580× less |
| Launch to ready | not measured | 0.2 s | 0.2 s | |

¹ While YouTube's player is still buffering, WebKit's GPU process adds about
37 MB for under a second each time a media segment arrives (about every
10 s; peak 185 MB). Both builds do this; the first measurement's samples
didn't land on one.

² Over a minute, network use is mostly audio and depends on how far ahead the
player is buffering, so short measurements vary by ±10% (1.86 vs 2.04 MB/min
back to back). For a song, the video track is a still picture: 7% of the
media bytes. The gap to Chrome's figure isn't fully attributed yet.

## What drives the difference

- **Chrome, 1,088 MB:**
  - Most of it is Chrome itself: the GPU process (261 MB), the YouTube Music
    page (248 MB), the browser process (117 MB), three other renderers
    (196 MB), and the network, audio and storage services.
  - Also macOS services Chrome triggers: a video decoder (39 MB), Metal's
    shader compiler (37 MB) and AutoFill (11 MB).
  - **Even leaving out the three other renderers, Chrome uses about 890 MB.**
- **Kilo 0.2, 147.5 MB:**
  - The native UI is 31 MB (39 MB in 0.1: images were held twice, and HTTP
    buffers were 8 times larger than needed).
  - The rest is YouTube's own player running in a hidden web view, inside a
    helper process: page 58 MB, helper 21 MB, WebKit GPU 18 MB, networking
    7 MB, audio 5 MB, plus 4 MB of system services.
  - After 5 minutes paused, Kilo shuts the helper down. Only the UI is left,
    at 31 MB including the system service every app gets. Pressing play
    restarts the helper in about 2 s, where playback stopped.

## Where Kilo is not better (yet)

- **CPU while playing is about the same.** Both run YouTube's own player
  script, which does most of the work. Kilo deliberately doesn't decode
  audio itself, because that would mean circumventing YouTube's protections.
- **Network use for songs is about the same in 0.2 as in 0.1,** and above Chrome's
  figure. YouTube's mobile web player, the lightest page that runs YouTube's
  player, has no audio-only mode, so it also streams a video track; for a
  song that's a still picture (7% of the media bytes). YouTube Music's own
  page streams audio only, but measured 398 MB instead of 147.
- **Startup has a short peak.** Memory reaches about 185 MB for a second
  while the player page loads.

## How it was measured

- **Tool:** `kilo-probe`, from this repository. It sums each app's whole
  process tree, plus the macOS services the system charges to that app (the
  "responsible" process), using `phys_footprint`, the number Activity
  Monitor shows. CPU is in percent of one core; wakeups are what macOS counts
  for energy use.
- **Chrome:** the user's YouTube Music app in Chrome, as normally used and
  playing a song, sampled every 5 s for 90 s. Network is the bytes received
  by Chrome's network process over the same 90 s.
- **Kilo:** the app bundle, run through a scripted session with
  `KILO_SCENARIO=wait:3,play:lYBUbBu4W08,wait:95,close,wait:65,pause,wait:345`
  and sampled every 2 s for 9 minutes (medians per phase). Network is the
  bytes received by WebKit's networking process over 60 s of playback. The
  0.1 and current builds were also measured back to back with
  `scripts/measure.sh` (same track, same minute-long windows) for the CPU
  and network figures.
- **Limitations:**
  - The two apps played different songs.
  - For part of Kilo's run, both apps were playing at once. Each app's
    processes were measured separately, so the totals don't mix.
  - Chrome's pause and window-closed states weren't measured, because Chrome
    couldn't be controlled from the test.
  - One machine, one run each.

## Since then

Kilo 0.3 (same Mac, `scripts/measure.sh`, launches alternating with the
build before it): **28.7–28.8 MB** with the window open on Home, 0% CPU
(2026-10-09); playing, 142.6–144 MB (2026-10-08). Chrome wasn't measured
again.
