use std::ffi::c_void;
use std::io;

use crate::Sample;

type Handle = *mut c_void;
type Bool = i32;

const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const TH32CS_SNAPPROCESS: u32 = 0x2;
const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;

/// `PROCESS_MEMORY_COUNTERS_EX2` (Windows 10 1809+), which carries the
/// private working set that Task Manager shows.
#[repr(C)]
#[derive(Default)]
struct MemoryCounters {
    cb: u32,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
    private_usage: usize,
    private_working_set_size: usize,
    shared_commit_usage: u64,
}

#[repr(C)]
#[derive(Default)]
struct FileTime {
    low: u32,
    high: u32,
}

impl FileTime {
    /// FILETIME durations are in 100 ns units.
    fn nanos(&self) -> u64 {
        ((u64::from(self.high) << 32) | u64::from(self.low)) * 100
    }
}

/// `PROCESSENTRY32W`.
#[repr(C)]
struct ProcessEntry {
    size: u32,
    usage: u32,
    pid: u32,
    default_heap_id: usize,
    module_id: u32,
    threads: u32,
    parent_pid: u32,
    pri_class_base: i32,
    flags: u32,
    exe_file: [u16; 260],
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn OpenProcess(access: u32, inherit: Bool, pid: u32) -> Handle;
    fn CloseHandle(handle: Handle) -> Bool;
    fn K32GetProcessMemoryInfo(process: Handle, counters: *mut MemoryCounters, cb: u32) -> Bool;
    fn GetProcessTimes(
        process: Handle,
        creation: *mut FileTime,
        exit: *mut FileTime,
        kernel: *mut FileTime,
        user: *mut FileTime,
    ) -> Bool;
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> Handle;
    fn Process32FirstW(snapshot: Handle, entry: *mut ProcessEntry) -> Bool;
    fn Process32NextW(snapshot: Handle, entry: *mut ProcessEntry) -> Bool;
}

struct Owned(Handle);

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: we own this handle and close it exactly once.
        unsafe { CloseHandle(self.0) };
    }
}

pub fn sample(pid: u32) -> io::Result<Sample> {
    // SAFETY: plain Win32 calls with valid out-pointers; the handle is closed
    // by `Owned`.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return Err(io::Error::last_os_error());
        }
        let process = Owned(process);

        let mut mem = MemoryCounters { cb: size_of::<MemoryCounters>() as u32, ..Default::default() };
        if K32GetProcessMemoryInfo(process.0, &mut mem, mem.cb) == 0 {
            return Err(io::Error::last_os_error());
        }
        let (mut creation, mut exit, mut kernel, mut user) = Default::default();
        if GetProcessTimes(process.0, &mut creation, &mut exit, &mut kernel, &mut user) == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Sample {
            footprint: mem.private_working_set_size as u64,
            resident: mem.working_set_size as u64,
            cpu_ns: kernel.nanos() + user.nanos(),
            wakeups: None,
        })
    }
}

pub fn children(pid: u32) -> Vec<u32> {
    snapshot().into_iter().filter(|e| e.1 == pid && e.0 != pid).map(|e| e.0).collect()
}

/// WebView2's msedgewebview2.exe processes are ordinary descendants.
pub fn adopted(_pids: &[u32]) -> Vec<u32> {
    Vec::new()
}

pub fn all_processes() -> Vec<(u32, String)> {
    snapshot().into_iter().map(|(pid, _, name)| (pid, name)).collect()
}

/// (pid, parent pid, exe name) for every process.
fn snapshot() -> Vec<(u32, u32, String)> {
    let mut out = Vec::new();
    // SAFETY: the snapshot handle is closed by `Owned`; `entry.size` is set
    // as the API requires.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let snap = Owned(snap);
        let mut entry: ProcessEntry = std::mem::zeroed();
        entry.size = size_of::<ProcessEntry>() as u32;
        let mut ok = Process32FirstW(snap.0, &mut entry);
        while ok != 0 {
            let len = entry.exe_file.iter().position(|&c| c == 0).unwrap_or(entry.exe_file.len());
            out.push((entry.pid, entry.parent_pid, String::from_utf16_lossy(&entry.exe_file[..len])));
            ok = Process32NextW(snap.0, &mut entry);
        }
    }
    out
}
