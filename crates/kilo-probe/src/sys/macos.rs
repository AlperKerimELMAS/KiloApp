use std::ffi::{c_int, c_void};
use std::io;
use std::sync::OnceLock;

use crate::Sample;

const RUSAGE_INFO_V2: c_int = 2;

/// `struct rusage_info_v2` from <sys/resource.h>.
#[repr(C)]
#[derive(Default)]
struct RusageInfoV2 {
    uuid: [u8; 16],
    user_time: u64,
    system_time: u64,
    pkg_idle_wkups: u64,
    interrupt_wkups: u64,
    pageins: u64,
    wired_size: u64,
    resident_size: u64,
    phys_footprint: u64,
    proc_start_abstime: u64,
    proc_exit_abstime: u64,
    child_user_time: u64,
    child_system_time: u64,
    child_pkg_idle_wkups: u64,
    child_interrupt_wkups: u64,
    child_pageins: u64,
    child_elapsed_abstime: u64,
    diskio_bytesread: u64,
    diskio_byteswritten: u64,
}

#[repr(C)]
struct MachTimebaseInfo {
    numer: u32,
    denom: u32,
}

// All of these live in libSystem, which every macOS binary links.
unsafe extern "C" {
    fn proc_pid_rusage(pid: c_int, flavor: c_int, buffer: *mut c_void) -> c_int;
    fn proc_listallpids(buffer: *mut c_void, buffersize: c_int) -> c_int;
    fn proc_listchildpids(ppid: c_int, buffer: *mut c_void, buffersize: c_int) -> c_int;
    fn proc_name(pid: c_int, buffer: *mut c_void, buffersize: u32) -> c_int;
    fn mach_timebase_info(info: *mut MachTimebaseInfo) -> c_int;
    fn dlsym(handle: *mut c_void, symbol: *const std::ffi::c_char) -> *mut c_void;
}

pub fn sample(pid: u32) -> io::Result<Sample> {
    let mut ri = RusageInfoV2::default();
    // SAFETY: `ri` is a correctly sized, writable rusage_info_v2.
    if unsafe { proc_pid_rusage(pid as c_int, RUSAGE_INFO_V2, (&raw mut ri).cast()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Sample {
        footprint: ri.phys_footprint,
        resident: ri.resident_size,
        cpu_ns: ticks_to_ns(ri.user_time + ri.system_time),
        wakeups: Some(ri.pkg_idle_wkups + ri.interrupt_wkups),
    })
}

/// rusage CPU times are in Mach absolute-time ticks (not nanoseconds on
/// Apple Silicon), so scale them by the timebase.
fn ticks_to_ns(ticks: u64) -> u64 {
    static TIMEBASE: OnceLock<(u128, u128)> = OnceLock::new();
    let &(numer, denom) = TIMEBASE.get_or_init(|| {
        let mut tb = MachTimebaseInfo { numer: 1, denom: 1 };
        // SAFETY: `tb` is a valid out-pointer.
        unsafe { mach_timebase_info(&mut tb) };
        (u128::from(tb.numer.max(1)), u128::from(tb.denom.max(1)))
    });
    (u128::from(ticks) * numer / denom) as u64
}

pub fn children(pid: u32) -> Vec<u32> {
    // SAFETY: `list_pids` passes a buffer of `size` writable bytes.
    list_pids(|buf, size| unsafe { proc_listchildpids(pid as c_int, buf, size) })
}

/// Processes that launchd started on behalf of one of `pids`, such as
/// WebKit's WebContent, Networking and GPU XPC services. Activity Monitor
/// attributes these to the app through the same "responsible pid" API.
pub fn adopted(pids: &[u32]) -> Vec<u32> {
    type Responsible = unsafe extern "C" fn(c_int) -> c_int;
    static FUNC: OnceLock<Option<Responsible>> = OnceLock::new();
    let func = FUNC.get_or_init(|| {
        const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;
        // Private but long-stable libSystem symbol; resolved at runtime so a
        // future removal degrades to "no adoption" instead of a crash.
        // SAFETY: dlsym with a NUL-terminated name; the symbol has this signature.
        let sym = unsafe { dlsym(RTLD_DEFAULT, c"responsibility_get_pid_responsible_for_pid".as_ptr()) };
        (!sym.is_null()).then(|| unsafe { std::mem::transmute::<*mut c_void, Responsible>(sym) })
    });
    let Some(func) = func else { return Vec::new() };
    // SAFETY: as in `children`.
    list_pids(|buf, size| unsafe { proc_listallpids(buf, size) })
        .into_iter()
        .filter(|pid| !pids.contains(pid))
        // SAFETY: the function takes any pid and returns -1 on failure.
        .filter(|&pid| u32::try_from(unsafe { func(pid as c_int) }).is_ok_and(|r| pids.contains(&r)))
        .collect()
}

pub fn all_processes() -> Vec<(u32, String)> {
    // SAFETY: as above.
    let pids = list_pids(|buf, size| unsafe { proc_listallpids(buf, size) });
    let mut name = [0u8; 256];
    pids.into_iter()
        .filter_map(|pid| {
            // SAFETY: `name` is writable for its full length.
            let len = unsafe { proc_name(pid as c_int, name.as_mut_ptr().cast(), name.len() as u32) };
            (len > 0).then(|| (pid, String::from_utf8_lossy(&name[..len as usize]).into_owned()))
        })
        .collect()
}

/// Calls a libproc listing function, which returns a pid count, first to size
/// the buffer and then to fill it.
fn list_pids(f: impl Fn(*mut c_void, c_int) -> c_int) -> Vec<u32> {
    let estimate = f(std::ptr::null_mut(), 0);
    if estimate <= 0 {
        return Vec::new();
    }
    let mut buf = vec![0 as c_int; estimate as usize + 32];
    let n = f(buf.as_mut_ptr().cast(), (buf.len() * size_of::<c_int>()) as c_int);
    buf.truncate(n.max(0) as usize);
    buf.into_iter().filter(|&p| p > 0).map(|p| p as u32).collect()
}
