//! `kilo-probe <pid|name> [-i SECS] [-d SECS] [--csv FILE] [--breakdown | --list]`
//!
//! Samples a process and all its descendants at a fixed interval and prints
//! footprint, CPU (% of one core) and wakeups per second.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use std::collections::HashMap;

use kilo_probe::{Sample, find_by_name, sample_each, total, used_between};

const USAGE: &str = "usage: kilo-probe <pid|process-name> [-i SECS] [-d SECS] [--csv FILE] [--breakdown | --list]

Samples the process tree's footprint (Activity Monitor / Task Manager / PSS),
CPU as % of one core, and wakeups per second.
  -i SECS     sampling interval (default 1)
  -d SECS     stop after this long and print a summary (default: until exit)
  --csv FILE  also write every sample to FILE
  --breakdown at the end, list each process with its footprint and CPU
  --list      print the tree's processes (pid and name) and exit";

struct Args {
    target: String,
    interval: Duration,
    duration: Option<Duration>,
    csv: Option<String>,
    breakdown: bool,
    list: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = std::env::args().skip(1);
    let mut target = None;
    let mut interval = Duration::from_secs(1);
    let mut duration = None;
    let mut csv = None;
    let mut breakdown = false;
    let mut list = false;
    let secs = |v: Option<String>, flag: &str| {
        v.and_then(|v| v.parse::<f64>().ok())
            .filter(|s| *s > 0.0)
            .and_then(|s| Duration::try_from_secs_f64(s).ok())
            .ok_or(format!("{flag} needs a positive number of seconds"))
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-i" => interval = secs(args.next(), "-i")?,
            "-d" => duration = Some(secs(args.next(), "-d")?),
            "--csv" => csv = Some(args.next().ok_or("--csv needs a file")?),
            "--breakdown" => breakdown = true,
            "--list" => list = true,
            "-h" | "--help" => return Err(String::new()),
            _ if target.is_none() => target = Some(arg),
            _ => return Err(format!("unexpected argument: {arg}")),
        }
    }
    Ok(Args { target: target.ok_or("missing <pid|process-name>")?, interval, duration, csv, breakdown, list })
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("error: {e}\n");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    let roots = match args.target.parse::<u32>() {
        Ok(pid) => vec![pid],
        Err(_) => find_by_name(&args.target),
    };
    if roots.is_empty() {
        eprintln!("error: no process named {:?}", args.target);
        return ExitCode::FAILURE;
    }
    if args.list {
        for (pid, name, _) in kilo_probe::breakdown(&roots) {
            println!("{pid} {name}");
        }
        return ExitCode::SUCCESS;
    }

    let mut csv = match args.csv.as_deref().map(File::create).transpose() {
        Ok(f) => f.map(BufWriter::new),
        Err(e) => {
            eprintln!("error: cannot create csv: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(w) = csv.as_mut() {
        let _ = writeln!(w, "t_s,footprint_bytes,resident_bytes,cpu_pct,wakeups_per_s,procs");
    }

    let start = Instant::now();
    let Some(mut prev) = sample_each(&roots) else {
        eprintln!("error: cannot read process {:?} (exited, or not ours?)", args.target);
        return ExitCode::FAILURE;
    };
    let mut prev_at = Instant::now();
    let mut stats = Stats::default();
    // Each process's first sample, for the breakdown: processes that start
    // later are measured from when they were first seen.
    let mut first: HashMap<u32, (Instant, Sample)> = prev.iter().map(|(pid, s)| (*pid, (prev_at, *s))).collect();

    loop {
        std::thread::sleep(args.interval);
        let Some(each) = sample_each(&roots) else {
            println!("process exited");
            break;
        };
        let at = Instant::now();
        for (pid, s) in &each {
            first.entry(*pid).or_insert((at, *s));
        }
        let (now, procs) = (total(&each), each.len());
        let wall_ns = at.duration_since(prev_at).as_nanos().max(1) as f64;
        let (cpu_ns, woke) = used_between(&prev, &each);
        let cpu_pct = cpu_ns as f64 / wall_ns * 100.0;
        let wakeups = woke.map(|w| w as f64 / wall_ns * 1e9);
        let t = at.duration_since(start).as_secs_f64();
        stats.add(&now, cpu_pct);

        println!(
            "t={t:7.1}s  footprint {:8}  (peak {:8})  rss {:8}  cpu {cpu_pct:5.1}%  wakeups {:>6}  procs {procs}",
            mb(now.footprint),
            mb(stats.peak),
            mb(now.resident),
            wakeups.map_or("-".into(), |w| format!("{w:.0}/s")),
        );
        if let Some(w) = csv.as_mut() {
            let _ = writeln!(
                w,
                "{t:.3},{},{},{cpu_pct:.3},{},{procs}",
                now.footprint,
                now.resident,
                wakeups.map_or(String::new(), |w| format!("{w:.1}"))
            );
        }

        prev = each;
        prev_at = at;
        if args.duration.is_some_and(|d| start.elapsed() >= d) {
            break;
        }
    }

    if let Some(mut w) = csv {
        let _ = w.flush();
    }
    stats.print();
    if args.breakdown {
        print_breakdown(&roots, &first);
    }
    ExitCode::SUCCESS
}

/// Lists each process with its current footprint and its CPU use since it
/// was first seen, largest first.
fn print_breakdown(roots: &[u32], first: &HashMap<u32, (Instant, Sample)>) {
    let mut rows = kilo_probe::breakdown(roots);
    rows.sort_by_key(|(_, _, s)| std::cmp::Reverse(s.footprint));
    println!("{:>7}  {:<40} {:>10}  {:>6}  {:>8}", "pid", "process", "footprint", "cpu", "wakeups");
    for (pid, name, s) in rows {
        let (since, start) = first.get(&pid).copied().unwrap_or((Instant::now(), s));
        let wall_ns = since.elapsed().as_nanos().max(1) as f64;
        let cpu = s.cpu_ns.saturating_sub(start.cpu_ns) as f64 / wall_ns * 100.0;
        let wakeups =
            s.wakeups.zip(start.wakeups).map_or("-".into(), |(n, p)| format!("{:.1}/s", n.saturating_sub(p) as f64 / wall_ns * 1e9));
        println!("{pid:>7}  {name:<40} {:>10}  {cpu:5.1}%  {wakeups:>8}", mb(s.footprint));
    }
}

#[derive(Default)]
struct Stats {
    n: u64,
    sum: u128,
    min: u64,
    peak: u64,
    cpu_sum: f64,
}

impl Stats {
    fn add(&mut self, s: &Sample, cpu_pct: f64) {
        self.min = if self.n == 0 { s.footprint } else { self.min.min(s.footprint) };
        self.n += 1;
        self.sum += u128::from(s.footprint);
        self.peak = self.peak.max(s.footprint);
        self.cpu_sum += cpu_pct;
    }

    fn print(&self) {
        if self.n == 0 {
            return;
        }
        let avg = (self.sum / u128::from(self.n)) as u64;
        println!(
            "summary: {} samples  footprint min {}  avg {}  peak {}  avg cpu {:.2}%",
            self.n,
            mb(self.min),
            mb(avg),
            mb(self.peak),
            self.cpu_sum / self.n as f64
        );
    }
}

fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}
