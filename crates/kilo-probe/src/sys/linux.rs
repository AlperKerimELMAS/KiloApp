use std::ffi::{c_int, c_long};
use std::fs;
use std::io;
use std::sync::OnceLock;

use crate::Sample;

const SC_CLK_TCK: c_int = 2; // Same value on glibc and musl.

unsafe extern "C" {
    fn sysconf(name: c_int) -> c_long;
}

pub fn sample(pid: u32) -> io::Result<Sample> {
    let rollup = fs::read_to_string(format!("/proc/{pid}/smaps_rollup"))?;
    let kib = |key: &str| {
        rollup
            .lines()
            .find_map(|l| l.strip_prefix(key))
            .and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<u64>().ok())
            .unwrap_or(0)
            * 1024
    };

    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let fields = stat_fields(&stat).ok_or_else(|| io::Error::other("malformed /proc stat"))?;
    // utime and stime are fields 14 and 15 of /proc/<pid>/stat.
    let ticks: u64 = fields.get(11..13).into_iter().flatten().filter_map(|f| f.parse::<u64>().ok()).sum();

    Ok(Sample {
        footprint: kib("Pss:"),
        resident: kib("Rss:"),
        cpu_ns: ticks * 1_000_000_000 / clock_ticks(),
        wakeups: context_switches(pid),
    })
}

/// Fields of /proc/<pid>/stat starting at field 3 (state). The command name
/// (field 2) may itself contain spaces and parentheses, so skip past the last
/// ')'.
fn stat_fields(stat: &str) -> Option<Vec<&str>> {
    Some(stat.get(stat.rfind(')')? + 1..)?.split_whitespace().collect())
}

fn clock_ticks() -> u64 {
    static HZ: OnceLock<u64> = OnceLock::new();
    // SAFETY: sysconf has no preconditions.
    *HZ.get_or_init(|| u64::try_from(unsafe { sysconf(SC_CLK_TCK) }).ok().filter(|&h| h > 0).unwrap_or(100))
}

/// Sum of voluntary + involuntary context switches across all threads: the
/// closest cheap Linux analogue to macOS wakeup counts.
fn context_switches(pid: u32) -> Option<u64> {
    let mut total = 0;
    for task in fs::read_dir(format!("/proc/{pid}/task")).ok()?.flatten() {
        let Ok(status) = fs::read_to_string(task.path().join("status")) else { continue };
        total += status
            .lines()
            .filter(|l| l.starts_with("voluntary_ctxt_switches:") || l.starts_with("nonvoluntary_ctxt_switches:"))
            .filter_map(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
            .sum::<u64>();
    }
    Some(total)
}

pub fn children(pid: u32) -> Vec<u32> {
    let pid = pid.to_string();
    proc_pids()
        .filter(|child| {
            fs::read_to_string(format!("/proc/{child}/stat"))
                .ok()
                .is_some_and(|s| stat_fields(&s).is_some_and(|f| f.get(1) == Some(&pid.as_str())))
        })
        .collect()
}

/// WebKitGTK's helper processes are ordinary descendants on Linux.
pub fn adopted(_pids: &[u32]) -> Vec<u32> {
    Vec::new()
}

pub fn all_processes() -> Vec<(u32, String)> {
    proc_pids().filter_map(|pid| Some((pid, fs::read_to_string(format!("/proc/{pid}/comm")).ok()?.trim_end().to_owned()))).collect()
}

fn proc_pids() -> impl Iterator<Item = u32> {
    fs::read_dir("/proc").into_iter().flatten().flatten().filter_map(|e| e.file_name().to_str()?.parse().ok())
}
