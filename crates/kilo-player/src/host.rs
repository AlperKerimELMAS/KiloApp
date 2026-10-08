//! App side: spawns and talks to the player helper.

use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command as Process, Stdio};
use std::time::{Duration, Instant};

use crate::protocol::{Command, Event};

/// A running player helper. Dropping it kills the helper.
pub struct PlayerProcess {
    child: Child,
    stdin: ChildStdin,
}

impl PlayerProcess {
    /// Starts `exe --player-helper` and delivers its events to `on_event`
    /// from a reader thread, ending with `Event::Exited` when the helper
    /// exits.
    pub fn spawn(exe: &Path, on_event: impl Fn(Event) + Send + 'static) -> io::Result<Self> {
        let mut child =
            Process::new(exe).arg("--player-helper").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn()?;
        let stdin = child.stdin.take().expect("piped");
        let stdout = child.stdout.take().expect("piped");
        std::thread::Builder::new().name("player-events".into()).stack_size(64 * 1024).spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                match Event::parse(&line) {
                    Some(event) => on_event(event),
                    None => on_event(Event::Error(format!("unparsable helper message: {line}"))),
                }
            }
            // Its stdout closed: the helper is gone.
            on_event(Event::Exited);
        })?;
        Ok(PlayerProcess { child, stdin })
    }

    pub fn send(&mut self, cmd: &Command) -> io::Result<()> {
        writeln!(self.stdin, "{}", cmd.encode())?;
        self.stdin.flush()
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Waits for the helper to end and says how it did (once its events
    /// have stopped, so it's ending anyway).
    pub fn wait(mut self) -> Option<std::process::ExitStatus> {
        self.child.wait().ok()
    }

    /// Asks the helper to quit, and kills it if it hasn't exited within
    /// `grace`.
    pub fn quit(mut self, grace: Duration) {
        let _ = self.send(&Command::Quit);
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // Drop kills it.
    }
}

impl Drop for PlayerProcess {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}
