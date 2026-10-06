//! `kilo-player <videoIdA> <videoIdB>`: drives the player helper through a
//! scripted session and measures each phase, the way Kilo will use it:
//! play A, let it end, switch to B in the same page, pause, kill the helper
//! as if idle, then start a fresh helper and resume B where it left off.
//!
//! `kilo-player --player-helper` runs the helper side (spawned by the host).

use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use kilo_player::host::PlayerProcess;
use kilo_player::protocol::{Command, Event, Position, VideoId};
use kilo_probe::Sample;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--player-helper") {
        kilo_player::run_helper();
    }
    let ids: Vec<VideoId> = args.iter().filter_map(|a| VideoId::parse(a)).collect();
    if ids.len() != 2 || args.len() != 2 {
        eprintln!("usage: kilo-player <videoIdA> <videoIdB>");
        return ExitCode::from(2);
    }
    match Session::new().and_then(|mut s| s.run(&ids[0], &ids[1])) {
        Ok(report) => {
            println!("\n==== report ====\n{report}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

struct Session {
    start: Instant,
    player: Option<PlayerProcess>,
    events: Option<Receiver<Event>>,
    report: String,
}

type Result<T> = std::result::Result<T, String>;

impl Session {
    fn new() -> Result<Self> {
        Ok(Session { start: Instant::now(), player: None, events: None, report: String::new() })
    }

    fn run(&mut self, a: &VideoId, b: &VideoId) -> Result<String> {
        self.line(&format!("host alone                   {}", fmt(&tree())));

        let t = Instant::now();
        self.spawn()?;
        self.wait("ready", Duration::from_secs(15), |e| matches!(e, Event::Ready))?;
        self.send(Command::Load(a.clone(), 0.0))?;
        let first = self.wait_playing(a, Duration::from_secs(30))?;
        self.line(&format!("cold start to audio          {:.1} s", t.elapsed().as_secs_f64()));
        self.measure("playing A, 20 s", Duration::from_secs(20))?;

        // Let A finish, then switch to B inside the same page.
        self.send(Command::Seek((first.duration - 4.0).max(0.0)))?;
        self.wait("A to end", Duration::from_secs(30), |e| matches!(e, Event::Ended(p) if p.video == *a))?;
        let t = Instant::now();
        self.send(Command::Load(b.clone(), 0.0))?;
        self.wait_playing(b, Duration::from_secs(30))?;
        self.line(&format!("track switch to audio        {:.2} s", t.elapsed().as_secs_f64()));
        self.measure("playing B, 20 s", Duration::from_secs(20))?;

        self.send(Command::Pause)?;
        self.wait("pause", Duration::from_secs(10), |e| matches!(e, Event::Paused(_)))?;
        self.measure("paused, 10 s", Duration::from_secs(10))?;

        let t = Instant::now();
        self.send(Command::Play)?;
        self.wait_playing(b, Duration::from_secs(10))?;
        self.line(&format!("resume from pause            {:.2} s", t.elapsed().as_secs_f64()));
        std::thread::sleep(Duration::from_secs(3));
        self.send(Command::Pause)?;
        let paused = self.wait("pause", Duration::from_secs(10), |e| matches!(e, Event::Paused(_)))?;
        let Event::Paused(at) = paused else { unreachable!() };

        // What Kilo does after N idle minutes: kill the helper outright.
        if let Some(p) = self.player.take() {
            p.quit(Duration::from_secs(2));
        }
        self.events = None;
        std::thread::sleep(Duration::from_secs(3));
        self.line(&format!("helper killed (idle)         {}", fmt(&tree())));

        let t = Instant::now();
        self.spawn()?;
        self.wait("ready", Duration::from_secs(15), |e| matches!(e, Event::Ready))?;
        self.send(Command::Load(b.clone(), at.seconds))?;
        let resumed = self.wait_playing(b, Duration::from_secs(30))?;
        self.line(&format!(
            "resume after idle kill       {:.1} s (paused at {:.1} s, resumed at {:.1} s)",
            t.elapsed().as_secs_f64(),
            at.seconds,
            resumed.seconds
        ));
        self.measure("resumed, 10 s", Duration::from_secs(10))?;

        if let Some(p) = self.player.take() {
            p.quit(Duration::from_secs(2));
        }
        Ok(std::mem::take(&mut self.report))
    }

    fn spawn(&mut self) -> Result<()> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let (tx, rx) = mpsc::channel();
        let player = PlayerProcess::spawn(&exe, move |e| {
            let _ = tx.send(e);
        })
        .map_err(|e| format!("cannot start helper: {e}"))?;
        self.player = Some(player);
        self.events = Some(rx);
        Ok(())
    }

    fn send(&mut self, cmd: Command) -> Result<()> {
        self.log(&format!("-> {}", cmd.encode()));
        self.player.as_mut().ok_or("no helper")?.send(&cmd).map_err(|e| e.to_string())
    }

    /// Waits for an event matching `want`, logging every event on the way.
    /// Ad and sign-in problems end the session (Premium-only rule).
    fn wait(&mut self, what: &str, timeout: Duration, want: impl Fn(&Event) -> bool) -> Result<Event> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let rx = self.events.as_ref().ok_or("no helper")?;
            let event = rx.recv_timeout(left).map_err(|_| format!("timed out waiting for {what}"))?;
            self.log(&format!("<- {}", event.encode()));
            match event {
                Event::SignedOut => return Err("not signed in: run `kilo-spike-web login` first".into()),
                Event::AdShowing => return Err("an ad showed: this account isn't Premium".into()),
                ref e if want(e) => return Ok(event),
                _ => {}
            }
        }
    }

    fn wait_playing(&mut self, id: &VideoId, timeout: Duration) -> Result<Position> {
        match self.wait("playback", timeout, |e| matches!(e, Event::Playing(p) if p.video == *id))? {
            Event::Playing(p) => Ok(p),
            _ => unreachable!(),
        }
    }

    /// Samples the whole tree (host + helper + WebKit's processes) over
    /// `window`, counting how many events the helper sent meanwhile. Peak is
    /// sampled every 5 s so the measuring itself stays cheap; per-process
    /// CPU is listed for the playing and paused windows.
    fn measure(&mut self, label: &str, window: Duration) -> Result<()> {
        let me = [std::process::id()];
        let first = kilo_probe::breakdown(&me);
        let start = tree();
        let at = Instant::now();
        let mut events = 0;
        let mut peak = start.footprint;
        while at.elapsed() < window {
            let left = window.saturating_sub(at.elapsed()).min(Duration::from_secs(5));
            if let Some(rx) = &self.events
                && let Ok(e) = rx.recv_timeout(left)
            {
                events += 1;
                self.log(&format!("<- {}", e.encode()));
                if matches!(e, Event::AdShowing) {
                    return Err("an ad showed: this account isn't Premium".into());
                }
                continue;
            }
            peak = peak.max(tree().footprint);
        }
        let end = tree();
        let wall = at.elapsed().as_nanos() as f64;
        let cpu = end.cpu_ns.saturating_sub(start.cpu_ns) as f64 / wall * 100.0;
        let wakeups = end.wakeups.zip(start.wakeups).map_or(0.0, |(e, s)| e.saturating_sub(s) as f64 / wall * 1e9);
        let host = kilo_probe::sample(std::process::id()).map(|s| s.footprint).unwrap_or(0);
        self.line(&format!(
            "{label:<28} {}  peak {}  cpu {cpu:4.1}%  wakeups {wakeups:3.0}/s  host {}  helper events {events}",
            fmt(&end),
            mb(peak),
            mb(host)
        ));
        if label.starts_with("playing B") || label.starts_with("paused") {
            for (pid, name, s) in kilo_probe::breakdown(&me) {
                let before = first.iter().find(|(p, _, _)| *p == pid).map_or(s.cpu_ns, |(_, _, f)| f.cpu_ns);
                let cpu = s.cpu_ns.saturating_sub(before) as f64 / wall * 100.0;
                self.line(&format!("    {pid:>6} {name:<36} {}  cpu {cpu:4.1}%", mb(s.footprint)));
            }
        }
        Ok(())
    }

    fn line(&mut self, s: &str) {
        self.log(s);
        self.report.push_str(s);
        self.report.push('\n');
    }

    fn log(&self, s: &str) {
        println!("[{:7.2}s] {s}", self.start.elapsed().as_secs_f64());
    }
}

fn tree() -> Sample {
    kilo_probe::sample_trees(&[std::process::id()]).map(|(s, _)| s).unwrap_or_default()
}

fn fmt(s: &Sample) -> String {
    let procs = kilo_probe::process_trees(&[std::process::id()]).len();
    format!("{} ({procs:>2} procs)", mb(s.footprint))
}

fn mb(bytes: u64) -> String {
    format!("{:6.1} MB", bytes as f64 / (1024.0 * 1024.0))
}
