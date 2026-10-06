# Kilo

A very lightweight YouTube Music desktop client. It uses about **40 MB** with
the window open and about **150 MB** while playing. YouTube Music in Chrome
uses about 1.1 GB.

Kilo is an unofficial client, not affiliated with YouTube or Google.

- **Native UI.** AppKit on macOS. Windows and Linux front ends are planned.
- **YouTube's own player plays the audio,** in a hidden system web view that
  runs in a helper process. Kilo never downloads, captures or decodes media,
  and never blocks ads.
- **Playback needs a YouTube Music Premium account.** Free accounts can browse
  and search.

## Build (macOS 14+)

```sh
./packaging/macos/bundle.sh            # builds dist/Kilo.app
./packaging/macos/bundle.sh --install  # also copies it to /Applications
```

## Layout

| Crate | What it is |
|---|---|
| `kilo-core` | Cross-platform YouTube Music API client, parsers, models, queue |
| `kilo-player` | The player helper and its line protocol |
| `kilo-mac` | The macOS app |
| `kilo-probe` | Measures memory and CPU like the OS task managers do |
| `kilo-spike-web`, `kilo-ui-bench` | Measurement labs behind the numbers in `docs/PLAN.md` |

See `docs/COMPARISON.md` for the measured comparison with YouTube Music in Chrome,
and `docs/PLAN.md` for the design and every measurement behind it.
