//! Process memory and CPU sampling.
//!
//! "Footprint" is the number each OS's own task manager reports, so our
//! measurements match what users see:
//! - macOS: `phys_footprint` (Activity Monitor "Memory")
//! - Windows: private working set (Task Manager "Memory")
//! - Linux: PSS (proportional set size; shared pages split between users)

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
    /// Cumulative wakeups (macOS: idle + interrupt wakeups; Linux: context
    /// switches). `None` where the OS doesn't expose a cheap equivalent.
    pub wakeups: Option<u64>,
}

impl std::ops::AddAssign for Sample {
    fn add_assign(&mut self, o: Self) {
        self.footprint += o.footprint;
        self.resident += o.resident;
        self.cpu_ns += o.cpu_ns;
        self.wakeups = match (self.wakeups, o.wakeups) {
            (Some(a), Some(b)) => Some(a + b),
            (a, b) => a.or(b),
        };
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
    let mut total = Sample::default();
    let mut count = 0;
    let mut root_alive = false;
    for pid in process_trees(roots) {
        if let Ok(s) = sample(pid) {
            total += s;
            count += 1;
            root_alive |= roots.contains(&pid);
        }
    }
    root_alive.then_some((total, count))
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

/// Finds processes whose executable name matches `name` (case-insensitive;
/// a Windows `.exe` suffix is ignored), skipping the calling process.
pub fn find_by_name(name: &str) -> Vec<u32> {
    let me = std::process::id();
    let name = strip_exe(name);
    sys::all_processes().into_iter().filter(|(pid, n)| *pid != me && strip_exe(n).eq_ignore_ascii_case(name)).map(|(pid, _)| pid).collect()
}

fn strip_exe(name: &str) -> &str {
    match name.len().checked_sub(4) {
        Some(i) if name.is_char_boundary(i) && name[i..].eq_ignore_ascii_case(".exe") => &name[..i],
        _ => name,
    }
}
