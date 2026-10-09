//! Process memory and CPU sampling.
//!
//! "Footprint" is `phys_footprint`, the number Activity Monitor shows as
//! "Memory", so our measurements match what users see.

use std::io;

mod sys;

#[derive(Clone, Copy, Debug, Default)]
pub struct Sample {
    /// Bytes, as defined in the module docs.
    pub footprint: u64,
    /// Resident set size in bytes (includes shared pages; informational).
    pub resident: u64,
    /// Total user + system CPU time consumed so far, in nanoseconds.
    pub cpu_ns: u64,
    /// Cumulative wakeups (idle and interrupt wakeups).
    pub wakeups: u64,
}

impl std::ops::AddAssign for Sample {
    fn add_assign(&mut self, o: Self) {
        self.footprint += o.footprint;
        self.resident += o.resident;
        self.cpu_ns += o.cpu_ns;
        self.wakeups += o.wakeups;
    }
}

/// Samples a single process.
pub fn sample(pid: u32) -> io::Result<Sample> {
    sys::sample(pid)
}

/// Returns `roots` followed by all of their descendants and adopted helper
/// processes (see `sys::adopted`), without duplicates. Helper processes count
/// against our budget too.
pub fn process_trees(roots: &[u32]) -> Vec<u32> {
    fn push_new(tree: &mut Vec<u32>, pids: Vec<u32>) {
        for pid in pids {
            if !tree.contains(&pid) {
                tree.push(pid);
            }
        }
    }
    let mut tree = Vec::with_capacity(roots.len());
    push_new(&mut tree, roots.to_vec());
    let mut i = 0;
    loop {
        while i < tree.len() {
            let children = sys::children(tree[i]);
            push_new(&mut tree, children);
            i += 1;
        }
        let before = tree.len();
        let adopted = sys::adopted(&tree);
        push_new(&mut tree, adopted);
        if tree.len() == before {
            return tree;
        }
    }
}

/// Samples `roots` and all their descendants, summed. Returns the total and
/// the number of processes sampled, or `None` once every root has exited.
/// Descendants that exit between listing and sampling are skipped.
pub fn sample_trees(roots: &[u32]) -> Option<(Sample, usize)> {
    let each = sample_each(roots)?;
    Some((total(&each), each.len()))
}

/// Samples `roots` and all their descendants, each on its own, or `None`
/// once every root has exited.
pub fn sample_each(roots: &[u32]) -> Option<Vec<(u32, Sample)>> {
    let each: Vec<(u32, Sample)> = process_trees(roots).into_iter().filter_map(|pid| Some((pid, sample(pid).ok()?))).collect();
    each.iter().any(|(pid, _)| roots.contains(pid)).then_some(each)
}

/// The sum of `samples`.
pub fn total(samples: &[(u32, Sample)]) -> Sample {
    samples.iter().fold(Sample::default(), |mut t, (_, s)| {
        t += *s;
        t
    })
}

/// The CPU time (ns) and wakeups the processes used between two samples.
/// Counters are per process, for their whole lives, so only processes in
/// both samples count: one that just appeared would add its whole past, and
/// one that exited would take its whole past away. (What a process did in
/// the interval it started or exited in is lost.)
pub fn used_between(before: &[(u32, Sample)], after: &[(u32, Sample)]) -> (u64, u64) {
    let mut cpu = 0;
    let mut wakeups = 0;
    for (pid, now) in after {
        let Some((_, then)) = before.iter().find(|(p, _)| p == pid) else { continue };
        cpu += now.cpu_ns.saturating_sub(then.cpu_ns);
        wakeups += now.wakeups.saturating_sub(then.wakeups);
    }
    (cpu, wakeups)
}

/// Per-process samples for `roots` and all their helpers, with executable
/// names, to show where the memory goes.
pub fn breakdown(roots: &[u32]) -> Vec<(u32, String, Sample)> {
    let names = sys::all_processes();
    process_trees(roots)
        .into_iter()
        .filter_map(|pid| {
            let s = sample(pid).ok()?;
            let name = names.iter().find(|(p, _)| *p == pid).map_or_else(String::new, |(_, n)| n.clone());
            Some((pid, name, s))
        })
        .collect()
}

/// Finds processes whose executable name matches `name` (case-insensitive),
/// skipping the calling process.
pub fn find_by_name(name: &str) -> Vec<u32> {
    let me = std::process::id();
    sys::all_processes().into_iter().filter(|(pid, n)| *pid != me && n.eq_ignore_ascii_case(name)).map(|(pid, _)| pid).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(cpu_s: u64) -> Sample {
        Sample { cpu_ns: cpu_s * 1_000_000_000, wakeups: cpu_s, ..Sample::default() }
    }

    #[test]
    fn processes_coming_and_going_dont_skew_cpu() {
        // The helper (20 s of CPU so far) exits; the parent used 1 s.
        assert_eq!(used_between(&[(1, s(10)), (2, s(20))], &[(1, s(11))]), (1_000_000_000, 1));
        // A process with a long past appears: its past isn't counted.
        assert_eq!(used_between(&[(1, s(10))], &[(1, s(12)), (3, s(500))]), (2_000_000_000, 2));
    }
}
