//! Playback: the player helper and its events, the player bar, and shutting
//! the helper down once playback has stopped for a while.

use std::ptr::NonNull;
use std::time::{Duration, Instant};

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchTime};
use kilo_core::model::Entry;
use kilo_core::parse;
use kilo_player::host::PlayerProcess;
use kilo_player::protocol::{Command, Event, VideoId};
use objc2_app_kit::NSWindowOcclusionState;
use objc2_foundation::{NSString, NSTimer};

use super::{App, queue, session, with, with_shell};
use crate::strings::{S, t};
use crate::ui::shell;
use crate::{images, net, ui};

/// After this long paused, the player helper (and all of WebKit) is shut
/// down; pressing play starts a fresh one where playback left off.
const IDLE_SHUTDOWN: Duration = Duration::from_secs(5 * 60);

/// A track that hasn't started this long after it was asked for won't: the
/// player is let go of, and the bar says so.
const LOAD_TIMEOUT: Duration = Duration::from_secs(30);

/// Plays the queue's current track from `start` seconds.
pub(super) fn play_current(start: f64) {
    let Some(Some((entry, token, client))) = with(|a| {
        a.play_token += 1;
        a.ran_out = false;
        a.queue.current().cloned().map(|e| (e, a.play_token, a.client.clone()))
    }) else {
        return;
    };
    show_now_playing(&entry);
    // Start the helper now; it takes longer than the lookup below.
    ensure_player();
    if entry.is_music_video()
        && start == 0.0
        && let (Some(client), Some(id)) = (client, entry.video_id().map(str::to_owned))
    {
        // Kilo never shows video: a music video plays as its song version
        // when there is one. Same music; the player streams a still
        // picture instead of video (measured: 41% less data).
        net::run(
            net::Pool::Api,
            move || client.next_single(&id).and_then(|j| parse::up_next(&j)),
            move |result| {
                if with(|a| a.play_token) != Some(token) {
                    return; // the user moved on
                }
                if let Some(song) =
                    result.ok().and_then(|q| q.entries.into_iter().next()).filter(|s| !s.is_music_video() && s.video_id().is_some())
                {
                    with(|a| a.queue.replace_current(song.clone()));
                    show_now_playing(&song);
                }
                load_current(start);
            },
        );
        return;
    }
    load_current(start);
}

/// Hands the current track to the player, and watches that it starts.
fn load_current(start: f64) {
    let Some(Some((entry, token))) = with(|a| a.queue.current().cloned().map(|e| (e, a.play_token))) else { return };
    let Some(id) = entry.video_id().and_then(VideoId::parse) else { return };
    ensure_player();
    with(|a| {
        a.playing = false;
        a.stalled = false;
        a.loading = Some(token);
        a.position = start;
        a.duration = 0.0;
        a.position_at = Instant::now();
    });
    send(Command::Load(id, start));
    queue::extend();
    let when = DispatchTime::NOW.time(LOAD_TIMEOUT.as_nanos() as i64);
    let _ = DispatchQueue::main().after(when, move || {
        if with(|a| a.loading == Some(token)).unwrap_or(false) {
            crate::debug::trace(|| "player: the track didn't start".into());
            stop_helper();
            show_status(t(S::PlaybackFailed));
        }
    });
}

fn ensure_player() {
    if with(|a| a.player.is_some()).unwrap_or(true) {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(serial) = with(|a| {
        a.player_serial += 1;
        a.player_serial
    }) else {
        return;
    };
    match PlayerProcess::spawn(&exe, move |event| net::later(move || on_player_event(serial, event))) {
        Ok(player) => {
            let volume = with(|a| {
                a.player = Some(player);
                a.volume
            });
            // The page starts at full volume.
            if let Some(volume) = volume.filter(|v| *v < 100) {
                send(Command::Volume(volume));
            }
        }
        Err(e) => show_status(&format!("{}: {e}", t(S::PlayerFailed))),
    }
}

fn send(cmd: Command) {
    let broken = with(|a| {
        let failed = a.player.as_mut().is_some_and(|p| p.send(&cmd).is_err());
        if failed { a.player.take() } else { None }
    })
    .flatten();
    if let Some(player) = broken {
        crate::debug::trace(|| "player: sending failed; dropping the helper".into());
        dispose(player, Duration::ZERO);
    }
}

/// Asks a helper to quit and kills it if it hasn't within `grace`, off the
/// main thread (and off the workers that load pages).
fn dispose(player: PlayerProcess, grace: Duration) {
    net::spawn(move || player.quit(grace), |()| {});
}

/// Whether `video` is the queue's current track. Events about any other
/// one were sent before the player switched tracks.
fn is_current(a: &App, video: &VideoId) -> bool {
    a.queue.current().and_then(Entry::video_id) == Some(video.as_str())
}

/// Playback stopped without the player saying so (its helper is gone):
/// keeps the position to resume from and shows the play button.
fn halt(a: &mut App) {
    a.position = current_position(a);
    a.position_at = Instant::now();
    a.playing = false;
    a.stalled = false;
    a.loading = None;
    if let Some(s) = &a.shell {
        ui::set_symbol(&s.bar.play, "play.fill");
    }
}

/// Stops playback for good (Premium-only: signed out, or an ad) and lets
/// the helper go.
fn stop_helper() {
    if let Some(player) = with(|a| {
        halt(a);
        a.player.take()
    })
    .flatten()
    {
        dispose(player, Duration::from_millis(500));
    }
    sync_timer();
}

fn on_player_event(serial: u64, event: Event) {
    crate::debug::trace(|| format!("player: {}", event.encode()));
    if with(|a| a.player_serial) != Some(serial) {
        return; // from a helper that has been replaced
    }
    match event {
        Event::Playing(p) => {
            let current = with(|a| {
                if !is_current(a, &p.video) {
                    return false;
                }
                a.playing = true;
                a.stalled = false;
                a.loading = None;
                a.position = p.seconds;
                a.duration = p.duration;
                a.position_at = Instant::now();
                a.idle_token += 1;
                a.idle_armed = false;
                if let Some(s) = &a.shell {
                    ui::set_symbol(&s.bar.play, "pause.fill");
                    s.bar.progress.set_enabled(true);
                }
                true
            });
            if current == Some(true) {
                sync_timer();
                update_progress();
            }
        }
        Event::Paused(ref p) | Event::Buffering(ref p) => {
            let paused = matches!(event, Event::Paused(_));
            let current = with(|a| {
                if !is_current(a, &p.video) {
                    return false;
                }
                a.position = p.seconds;
                a.duration = p.duration.max(a.duration);
                a.position_at = Instant::now();
                a.loading = None;
                // Waiting for data: the clock stops until it plays again.
                a.stalled = !paused;
                if paused {
                    a.playing = false;
                    if let Some(s) = &a.shell {
                        ui::set_symbol(&s.bar.play, "play.fill");
                    }
                }
                true
            });
            if current == Some(true) {
                if paused {
                    sync_timer();
                    schedule_idle_shutdown();
                }
                update_progress();
            }
        }
        Event::Ended(p) => {
            let Some(Some(advanced)) = with(|a| is_current(a, &p.video).then(|| a.queue.advance().is_some())) else { return };
            if advanced {
                play_current(0.0);
                return;
            }
            with(|a| {
                a.playing = false;
                a.ran_out = true;
                if let Some(s) = &a.shell {
                    ui::set_symbol(&s.bar.play, "play.fill");
                }
            });
            sync_timer();
            // Playback carries on if more arrives (`queue::carry_on`);
            // otherwise the helper goes once it has idled long enough.
            queue::extend();
            schedule_idle_shutdown();
        }
        Event::SignedOut => {
            stop_helper();
            session::session_ended();
        }
        Event::AdShowing => {
            stop_helper();
            show_status(t(S::PremiumNeeded));
        }
        Event::Exited => {
            // Quit, crashed, or shut down (by us when idle, or by macOS 27
            // itself, which quits an idle helper with SIGTERM, "quiet safe
            // quit"). Either way the next play starts a fresh one.
            let (player, loading) = with(|a| {
                let loading = a.loading.is_some();
                halt(a);
                (a.player.take(), loading)
            })
            .unwrap_or_default();
            sync_timer();
            if loading {
                // It went before the track started (its page didn't load).
                show_status(t(S::PlaybackFailed));
            }
            if let Some(player) = player {
                net::spawn(
                    move || player.wait(),
                    |status| {
                        crate::debug::trace(|| format!("player: helper ended with {status:?}"));
                    },
                );
            }
        }
        Event::Error(msg) => eprintln!("player: {msg}"),
        Event::Next => next(),
        Event::Previous => previous(),
        Event::Ready => {}
    }
}

pub fn play_pause() {
    let Some((has_player, playing, position)) = with(|a| (a.player.is_some(), a.playing, a.position)) else { return };
    if !has_player {
        // The helper was shut down while idle: start a new one and pick up
        // where playback stopped.
        play_current(position);
    } else if playing {
        send(Command::Pause);
    } else {
        send(Command::Play);
    }
}

pub fn next() {
    if with(|a| a.queue.advance().is_some()).unwrap_or(false) {
        play_current(0.0);
    }
}

/// Back to the start of the track, or to the previous one if it only just
/// started.
pub fn previous() {
    if with(|a| current_position(a)).unwrap_or(0.0) > 3.0 {
        seek_to(0.0);
    } else if with(|a| a.queue.back().is_some()).unwrap_or(false) {
        play_current(0.0);
    }
}

/// Seeks to `fraction` (0–1) of the track.
pub fn seek(fraction: f64) {
    let Some(duration) = with(|a| a.duration) else { return };
    if duration > 0.0 {
        seek_to(fraction.clamp(0.0, 1.0) * duration);
    }
}

/// Moves playback to `seconds`. Without a helper (shut down while idle),
/// playback resumes from there.
fn seek_to(seconds: f64) {
    with(|a| {
        a.position = seconds;
        a.position_at = Instant::now();
    });
    send(Command::Seek(seconds));
    update_progress();
}

/// Moves playback `seconds` forward (or back, if negative).
pub fn seek_by(seconds: f64) {
    let Some(Some(to)) = with(|a| (a.duration > 0.0).then(|| (current_position(a) + seconds).clamp(0.0, a.duration))) else { return };
    seek_to(to);
}

pub fn set_volume(volume: u8) {
    let volume = volume.min(100);
    with(|a| {
        a.volume = volume;
        a.unmute_to = None;
        if let Some(s) = &a.shell {
            s.bar.volume.set_value(f64::from(volume) / 100.0);
            ui::set_symbol(&s.bar.speaker, shell::speaker_symbol(volume));
        }
    });
    send(Command::Volume(volume));
}

/// Turns the volume up or down by `delta` percent.
pub fn change_volume(delta: i16) {
    let Some(volume) = with(|a| a.volume) else { return };
    set_volume((i16::from(volume) + delta).clamp(0, 100) as u8);
}

/// Mutes, or brings back the volume from before muting.
pub fn toggle_mute() {
    let Some((volume, unmute_to)) = with(|a| (a.volume, a.unmute_to)) else { return };
    match unmute_to {
        Some(to) => set_volume(to),
        None if volume > 0 => {
            set_volume(0);
            with(|a| a.unmute_to = Some(volume));
        }
        // Silent without having muted: back to full.
        None => set_volume(100),
    }
}

pub fn is_muted() -> bool {
    with(|a| a.unmute_to.is_some()).unwrap_or(false)
}

fn current_position(a: &App) -> f64 {
    let pos = if a.playing && !a.stalled { a.position + a.position_at.elapsed().as_secs_f64() } else { a.position };
    if a.duration > 0.0 { pos.min(a.duration) } else { pos }
}

/// Brings a newly built player bar up to date: the volume, and what's
/// playing.
pub(super) fn restore_bar() {
    let now = with(|a| {
        let s = a.shell.as_ref()?;
        s.bar.volume.set_value(f64::from(a.volume) / 100.0);
        ui::set_symbol(&s.bar.speaker, shell::speaker_symbol(a.volume));
        let entry = a.queue.current()?.clone();
        ui::set_symbol(&s.bar.play, if a.playing { "pause.fill" } else { "play.fill" });
        s.bar.progress.set_enabled(a.duration > 0.0);
        Some(entry)
    })
    .flatten();
    if let Some(entry) = now {
        show_now_playing(&entry);
        update_progress();
    }
}

fn show_now_playing(entry: &Entry) {
    with(|a| {
        let Some(s) = &a.shell else { return };
        let bar = &s.bar;
        bar.title.setStringValue(&NSString::from_str(&entry.title));
        bar.artist.setStringValue(&NSString::from_str(&entry.subtitle));
        bar.elapsed.setStringValue(&NSString::from_str(""));
        bar.total.setStringValue(&NSString::from_str(""));
        bar.progress.set_value(0.0);
        ui::set_image(&bar.art, None);
        if let Some(t) = &entry.thumb {
            // Square: a music video's 16:9 picture is cropped to fit.
            let px = ui::square_px(56.0, s.window.backingScaleFactor(), t.wide);
            let art = bar.art.clone();
            let url = t.sized(px);
            images::load(url.clone(), px, move |img| {
                // Only if it's still this track's (a slow image for the
                // previous one mustn't land on the next).
                let current = with(|a| a.queue.current().and_then(|e| e.thumb.as_ref()).is_some_and(|t| t.sized(px) == url));
                if current == Some(true) {
                    ui::set_image(&art, img.as_ref());
                }
            });
        }
    });
}

fn show_status(text: &str) {
    with_shell(|s| {
        s.bar.title.setStringValue(&NSString::from_str(text));
        s.bar.artist.setStringValue(&NSString::from_str(""));
    });
}

/// The progress timer runs only while music plays and the window is on
/// screen; a closed, minimized or covered window costs no wakeups.
pub(super) fn sync_timer() {
    let want = with(|a| a.playing && a.shell.as_ref().is_some_and(|s| s.window.occlusionState().contains(NSWindowOcclusionState::Visible)))
        .unwrap_or(false);
    if want {
        start_timer();
        update_progress();
    } else {
        stop_timer();
    }
}

/// Moves the progress bar once a second. The position is extrapolated
/// locally; the player helper sends nothing while playing.
fn start_timer() {
    if with(|a| a.timer.is_some()).unwrap_or(true) {
        return;
    }
    let tick = RcBlock::new(|_t: NonNull<NSTimer>| update_progress());
    // SAFETY: the timer retains the block; runs on the main run loop.
    let timer = unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(1.0, true, &tick) };
    // Let macOS coalesce our wakeups with others.
    timer.setTolerance(0.3);
    with(|a| a.timer = Some(timer));
}

fn stop_timer() {
    if let Some(Some(t)) = with(|a| a.timer.take()) {
        t.invalidate();
    }
}

fn update_progress() {
    with(|a| {
        let pos = current_position(a);
        let Some(s) = &a.shell else { return };
        if a.duration > 0.0 {
            s.bar.progress.set_value(pos / a.duration);
            s.bar.elapsed.setStringValue(&NSString::from_str(&clock(pos)));
            s.bar.total.setStringValue(&NSString::from_str(&clock(a.duration)));
        }
    });
}

/// `m:ss`, or `h:mm:ss` from an hour on.
fn clock(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// Starts the countdown to shutting the helper down once playback stops
/// (paused, or the queue ran out). It runs from the first stop: later
/// "paused" reports from the page don't restart it; only playing again
/// cancels it. With the window closed, App Nap may make it fire a little
/// late, and macOS may quit the idle helper on its own first.
fn schedule_idle_shutdown() {
    let Some(Some(token)) = with(|a| (!std::mem::replace(&mut a.idle_armed, true)).then_some(a.idle_token)) else {
        return;
    };
    let idle = crate::debug::idle_shutdown().unwrap_or(IDLE_SHUTDOWN);
    let when = DispatchTime::NOW.time(idle.as_nanos() as i64);
    let _ = DispatchQueue::main().after(when, move || {
        let player = with(|a| {
            if a.idle_token != token {
                return None;
            }
            a.idle_armed = false;
            if a.playing { None } else { a.player.take() }
        })
        .flatten();
        if let Some(player) = player {
            crate::debug::trace(|| "idle: shutting the player helper down".into());
            dispose(player, Duration::from_secs(2));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::clock;

    #[test]
    fn formats_track_times() {
        assert_eq!(clock(0.0), "0:00");
        assert_eq!(clock(-3.0), "0:00");
        assert_eq!(clock(213.9), "3:33");
        assert_eq!(clock(3725.0), "1:02:05");
    }
}
