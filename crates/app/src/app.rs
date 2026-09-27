//! The bridge between the backend (tokio, `exorchy_core::host`) and GTK's
//! main loop.
//!
//! Every backend call is an `async fn` that must run on tokio; every widget
//! update must happen on the GTK thread. [`spawn`] runs a backend future on
//! tokio and hands its result to a closure on the main loop; [`call`] does
//! the same inside an `async` block that already runs on the main loop
//! (`glib::spawn_future_local`). [`on_event`] subscribes a main-loop closure
//! to the backend's named events (`theme-changed`, `game-exited`, ...).

use gtk::glib;
use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::rc::Rc;

use exorchy_core::host::{async_runtime, AppHandle};

type Listener = Rc<dyn Fn(&serde_json::Value)>;

struct Shell {
    core: AppHandle,
    listeners: RefCell<HashMap<String, Vec<Listener>>>,
}

thread_local! {
    static SHELL: RefCell<Option<Rc<Shell>>> = const { RefCell::new(None) };
}

fn shell() -> Rc<Shell> {
    SHELL.with(|s| s.borrow().clone().expect("app::install() runs before any UI code"))
}

/// Register the backend handle with the main thread and start the event
/// pump. Call once, before `Application::run`.
pub fn install(core: AppHandle) {
    let shell = Rc::new(Shell { core: core.clone(), listeners: RefCell::new(HashMap::new()) });
    SHELL.with(|s| *s.borrow_mut() = Some(shell));
    // The pump: a main-loop task that awaits the broadcast receiver. A lagged
    // receiver skips to the newest events; the UI polls state anyway.
    let mut rx = core.subscribe();
    glib::spawn_future_local(async move {
        loop {
            match rx.recv().await {
                Ok(ev) => dispatch(&ev.name, &ev.payload),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

fn dispatch(name: &str, payload: &serde_json::Value) {
    let listeners: Vec<Listener> = shell()
        .listeners
        .borrow()
        .get(name)
        .map(|v| v.to_vec())
        .unwrap_or_default();
    for l in listeners {
        l(payload);
    }
}

/// The backend handle, for building command futures.
pub fn core() -> AppHandle {
    shell().core.clone()
}

/// Subscribe to a backend event by name. The closure runs on the GTK thread.
pub fn on_event(name: &str, f: impl Fn(&serde_json::Value) + 'static) {
    shell().listeners.borrow_mut().entry(name.to_string()).or_default().push(Rc::new(f));
}

/// Run a backend future on tokio; `done` runs on the GTK thread with the result.
pub fn spawn<F, T>(fut: F, done: impl FnOnce(T) + 'static)
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let handle = async_runtime::spawn(fut);
    glib::spawn_future_local(async move {
        match handle.await {
            Ok(v) => done(v),
            Err(e) => log::error!("backend task failed: {e}"),
        }
    });
}

/// Await a backend future from a main-loop task.
pub async fn call<F, T>(fut: F) -> T
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    async_runtime::spawn(fut).await.expect("backend task panicked")
}

/// Run a main-loop future (widgets may be captured).
pub fn local<F: Future<Output = ()> + 'static>(fut: F) {
    glib::spawn_future_local(fut);
}
