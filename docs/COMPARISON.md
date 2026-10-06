# Kilo vs. YouTube Music in Chrome

Measured on 2026-10-07 on one machine: MacBook (Apple Silicon, 16 GB),
macOS 27.0.1, Retina display, YouTube Music Premium. Both apps were playing
music.

## Results

| | YouTube Music (Chrome app) | Kilo 0.1 | Kilo uses |
|---|---|---|---|
| **Memory, playing, window open** | **1,088 MB** (1,035–1,152) | **153 MB** (152–154) | **7× less** |
| Processes | 18–20 | 8 | |
| CPU, playing (% of one core) | 8.9% | 8.9% | the same |
| Wakeups per second, playing | 258 | 121 | 2× fewer |
| Playing with the window closed | not possible (closing the window stops music) | 147 MB, 7.6% CPU | |
| Paused, first 5 minutes | not measured | 143 MB, **0.08% CPU**, 2 wakeups/s | |
| Paused for 5+ minutes | not measured | **34 MB, 0.00% CPU, 0 wakeups/s** | |
| Network while playing | 1.2 MB/min | 1.9 MB/min | **59% more** |
| App on disk | 1.4 GB (Chrome) | 2.3 MB | 600× less |
| Launch to ready | not measured | 0.2 s | |

## What drives the difference

- **Chrome, 1,088 MB:**
  - Most of it is Chrome itself: the GPU process (261 MB), the YouTube Music
    page (248 MB), the browser process (117 MB), three other renderers
    (196 MB), and the network, audio and storage services.
  - Also macOS services Chrome triggers: a video decoder (39 MB), Metal's
    shader compiler (37 MB) and AutoFill (11 MB).
  - **Even leaving out the three other renderers, Chrome uses about 890 MB.**
- **Kilo, 153 MB:**
  - The native UI is 39 MB.
  - The rest is YouTube's own player running in a hidden web view, inside a
    helper process: page 58 MB, helper 21 MB, WebKit GPU 18 MB, networking
    7 MB, audio 5 MB, plus 4 MB of system services.
  - After 5 minutes paused, Kilo shuts the helper down. Only the UI is left,
    at 34 MB including the system service every app gets. Pressing play
    restarts the helper in about 2 s, where playback stopped.

## Where Kilo is not better (yet)

- **CPU while playing is the same.** Both run YouTube's own player script,
  which does most of the work. Kilo deliberately doesn't decode audio itself,
  because that would mean circumventing YouTube's protections.
- **It uses about 59% more data.** YouTube's mobile web player, the lightest
  page that runs YouTube's player, has no audio-only mode, so it also streams
  a 144p video track.
- **Startup has a short peak.** Memory reaches 202 MB for a second while the
  player page loads.

## How it was measured

- **Tool:** `kilo-probe`, from this repository. It sums each app's whole
  process tree, plus the macOS services the system charges to that app (the
  "responsible" process), using `phys_footprint`, the number Activity
  Monitor shows. CPU is in percent of one core; wakeups are what macOS counts
  for energy use.
- **Chrome:** the user's YouTube Music app in Chrome, as normally used and
  playing a song, sampled every 5 s for 90 s. Network is the bytes received
  by Chrome's network process over the same 90 s.
- **Kilo:** the installed `/Applications/Kilo.app`, run through a scripted
  session with `KILO_SCENARIO=wait:3,play:lYBUbBu4W08,wait:95,close,wait:65,pause,wait:345`
  and sampled every 2 s for 9 minutes. Network is the bytes received by
  WebKit's networking process over 60 s of playback.
- **Limitations:**
  - The two apps played different songs.
  - For part of Kilo's run, both apps were playing at once. Each app's
    processes were measured separately, so the totals don't mix.
  - Chrome's pause and window-closed states weren't measured, because Chrome
    couldn't be controlled from the test.
  - One machine, one run each.
