//! Background work with main-thread completions. Jobs run on a few small
//! worker threads (or, for work that waits on another process, a thread of
//! their own); their results come back on the main thread, where the UI
//! lives, via GCD.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock};

use dispatch2::DispatchQueue;

type Job = Box<dyn FnOnce() + Send>;
type Callback = Box<dyn FnOnce(Box<dyn Any + Send>)>;

/// API calls and images get separate workers so a page of thumbnails never
/// delays a click.
#[derive(Clone, Copy)]
pub enum Pool {
    Api,
    Images,
}

thread_local! {
    static CALLBACKS: RefCell<HashMap<u64, Callback>> = RefCell::new(HashMap::new());
    static NEXT_ID: Cell<u64> = const { Cell::new(0) };
}

fn pool(which: Pool) -> &'static Sender<Job> {
    static API: OnceLock<Sender<Job>> = OnceLock::new();
    static IMAGES: OnceLock<Sender<Job>> = OnceLock::new();
    let (cell, threads, name) = match which {
        Pool::Api => (&API, 2, "kilo-api"),
        Pool::Images => (&IMAGES, 2, "kilo-images"),
    };
    cell.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        for i in 0..threads {
            let rx = rx.clone();
            let _ = std::thread::Builder::new().name(format!("{name}-{i}")).stack_size(512 * 1024).spawn(move || {
                loop {
                    let job = match rx.lock() {
                        Ok(rx) => rx.recv(),
                        Err(_) => return,
                    };
                    match job {
                        Ok(job) => job(),
                        Err(_) => return,
                    }
                }
            });
        }
        tx
    })
}

/// Runs `job` on a worker thread, then `done` with its result on the main
/// thread. Must be called on the main thread.
pub fn run<T: Send + 'static>(which: Pool, job: impl FnOnce() -> T + Send + 'static, done: impl FnOnce(T) + 'static) {
    let id = register(done);
    let _ = pool(which).send(Box::new(move || deliver(id, job())));
}

/// Like `run`, on a thread of its own: for work that waits on another
/// process (the sign-in window, a helper quitting), which must neither wait
/// behind page loads nor hold them up. Rare, so a thread each is fine.
pub fn spawn<T: Send + 'static>(job: impl FnOnce() -> T + Send + 'static, done: impl FnOnce(T) + 'static) {
    let id = register(done);
    let spawned = std::thread::Builder::new().name("kilo-process".into()).stack_size(256 * 1024).spawn(move || deliver(id, job()));
    if spawned.is_err() {
        CALLBACKS.with_borrow_mut(|c| c.remove(&id));
    }
}

/// Keeps `done` until its result arrives (`deliver`).
fn register<T: Send + 'static>(done: impl FnOnce(T) + 'static) -> u64 {
    let id = NEXT_ID.with(|n| {
        n.set(n.get() + 1);
        n.get()
    });
    CALLBACKS.with_borrow_mut(|c| {
        c.insert(id, Box::new(move |any| done(*any.downcast::<T>().expect("callback type"))));
    });
    id
}

/// From a worker: hands `out` to the callback `id` on the main thread.
fn deliver<T: Send + 'static>(id: u64, out: T) {
    DispatchQueue::main().exec_async(move || {
        if let Some(cb) = CALLBACKS.with_borrow_mut(|c| c.remove(&id)) {
            cb(Box::new(out));
        }
    });
}

/// Runs `f` on the main thread after the current event finishes. Used to
/// step out of AppKit callbacks before touching app state again.
pub fn later(f: impl FnOnce() + Send + 'static) {
    DispatchQueue::main().exec_async(f);
}
