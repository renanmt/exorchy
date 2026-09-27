//! The host: what the backend needs from the application shell.
//!
//! The backend was written against Tauri's `AppHandle` / `State` / events /
//! `async_runtime`. This module provides the same four things without a
//! webview, so the GTK app (or a test) can drive the backend directly:
//!
//! - [`AppHandle`]: a cheap, cloneable handle holding the managed state
//!   (`manage` / `state` / `try_state`) and the event bus (`emit`).
//! - [`State`]: a shared reference to one managed value (`Deref<Target = T>`).
//! - [`Event`]: what `emit` broadcasts; the UI subscribes with
//!   [`AppHandle::subscribe`].
//! - [`async_runtime`]: `spawn` / `spawn_blocking` / `block_on` on the one
//!   tokio runtime the process owns.
//!
//! The [`Manager`] and [`Emitter`] traits exist so the ported modules keep
//! their `use crate::host::{Manager, Emitter}` lines; they carry no extra
//! behaviour.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::ops::Deref;
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;
use tokio::sync::broadcast;

/// One backend event, as the UI receives it. `payload` is the JSON the
/// webview used to get; the UI deserialises what it needs.
#[derive(Debug, Clone)]
pub struct Event {
    pub name: String,
    pub payload: serde_json::Value,
}

struct Inner {
    states: Mutex<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
    events: broadcast::Sender<Event>,
}

/// The application handle: managed state + event bus. Clone freely.
#[derive(Clone)]
pub struct AppHandle(Arc<Inner>);

impl Default for AppHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl AppHandle {
    pub fn new() -> Self {
        let (events, _) = broadcast::channel(256);
        AppHandle(Arc::new(Inner { states: Mutex::new(HashMap::new()), events }))
    }

    /// Register a value; the first registration of a type wins. Returns
    /// whether it was stored (mirrors Tauri's contract).
    pub fn manage<T: Send + Sync + 'static>(&self, value: T) -> bool {
        let mut states = self.0.states.lock().unwrap_or_else(|e| e.into_inner());
        if states.contains_key(&TypeId::of::<T>()) {
            return false;
        }
        states.insert(TypeId::of::<T>(), Arc::new(value));
        true
    }

    /// The managed value of type `T`. Panics when nothing was managed for
    /// it, like Tauri: every state is registered at startup.
    pub fn state<T: Send + Sync + 'static>(&self) -> State<'static, T> {
        self.try_state::<T>()
            .unwrap_or_else(|| panic!("state not managed: {}", std::any::type_name::<T>()))
    }

    pub fn try_state<T: Send + Sync + 'static>(&self) -> Option<State<'static, T>> {
        let states = self.0.states.lock().unwrap_or_else(|e| e.into_inner());
        let any = states.get(&TypeId::of::<T>())?.clone();
        let typed: Arc<T> = Arc::downcast::<T>(any).ok()?;
        Some(State(typed, std::marker::PhantomData))
    }

    /// Broadcast an event to every subscriber. Never fails: with no
    /// subscriber the event is dropped, which is what a webview that has not
    /// registered a listener did too.
    pub fn emit<S: Serialize>(&self, name: &str, payload: S) -> Result<(), String> {
        let payload = serde_json::to_value(payload).map_err(|e| e.to_string())?;
        let _ = self.0.events.send(Event { name: name.to_string(), payload });
        Ok(())
    }

    /// A receiver for every event emitted from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.0.events.subscribe()
    }

    /// Tauri's `app.handle()`: this handle.
    pub fn handle(&self) -> &AppHandle {
        self
    }

    /// End the process with `code`, after the shell had its chance to clean
    /// up. The GTK app never calls this; it exists for the startup-error path.
    pub fn exit(&self, code: i32) {
        std::process::exit(code)
    }
}

/// A managed value. The lifetime is phantom: the value lives as long as the
/// handle it came from, and every handle shares one store.
pub struct State<'a, T: Send + Sync + 'static>(Arc<T>, std::marker::PhantomData<&'a ()>);

impl<T: Send + Sync + 'static> State<'_, T> {
    pub fn inner(&self) -> &T {
        &self.0
    }
}

impl<T: Send + Sync + 'static> Clone for State<'_, T> {
    fn clone(&self) -> Self {
        State(self.0.clone(), std::marker::PhantomData)
    }
}

impl<T: Send + Sync + 'static> Deref for State<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

/// Kept so ported modules can `use crate::host::Manager`; the methods live on
/// [`AppHandle`] directly.
pub trait Manager {
    fn app_handle(&self) -> &AppHandle;
}

impl Manager for AppHandle {
    fn app_handle(&self) -> &AppHandle {
        self
    }
}

/// Kept so ported modules can `use crate::host::Emitter`; `emit` lives on
/// [`AppHandle`] directly.
pub trait Emitter {
    fn emitter(&self) -> &AppHandle;
}

impl Emitter for AppHandle {
    fn emitter(&self) -> &AppHandle {
        self
    }
}

/// The process-wide tokio runtime, used when no runtime is current (the GTK
/// main thread, plain threads). Tests run inside `#[tokio::test]` and never
/// touch it.
pub mod async_runtime {
    use super::OnceLock;
    use std::future::Future;

    pub use tokio::task::JoinHandle;

    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

    /// The runtime every backend task runs on. Created on first use; the app
    /// calls this once at startup so the thread pool exists before the UI.
    pub fn runtime() -> &'static tokio::runtime::Runtime {
        RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_name("exorchy-rt")
                .build()
                .expect("tokio runtime")
        })
    }

    /// The current runtime's handle when inside one, else the process runtime's.
    pub fn handle() -> tokio::runtime::Handle {
        tokio::runtime::Handle::try_current().unwrap_or_else(|_| runtime().handle().clone())
    }

    pub fn spawn<F>(task: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        handle().spawn(task)
    }

    pub fn spawn_blocking<F, R>(task: F) -> JoinHandle<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        handle().spawn_blocking(task)
    }

    /// Run a future to completion from synchronous code. Inside a runtime
    /// worker it yields the worker first (`block_in_place`) so the pool never
    /// deadlocks on itself.
    pub fn block_on<F: Future>(task: F) -> F::Output {
        match tokio::runtime::Handle::try_current() {
            Ok(h) => tokio::task::block_in_place(|| h.block_on(task)),
            Err(_) => runtime().block_on(task),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Counter(Mutex<u32>);

    #[test]
    fn managed_state_is_shared_between_clones() {
        let app = AppHandle::new();
        assert!(app.manage(Counter(Mutex::new(1))));
        assert!(!app.manage(Counter(Mutex::new(9))), "first registration wins");
        let other = app.clone();
        *other.state::<Counter>().inner().0.lock().unwrap() += 1;
        assert_eq!(*app.state::<Counter>().inner().0.lock().unwrap(), 2);
        assert!(app.try_state::<String>().is_none());
    }

    #[test]
    fn events_reach_subscribers_and_vanish_without_any() {
        let app = AppHandle::new();
        assert!(app.emit("nobody-listens", 1).is_ok());
        let mut rx = app.subscribe();
        app.emit("game-exited", serde_json::json!({ "id": 7 })).unwrap();
        let ev = rx.try_recv().unwrap();
        assert_eq!(ev.name, "game-exited");
        assert_eq!(ev.payload["id"], 7);
    }

    #[test]
    fn block_on_works_outside_and_inside_a_runtime() {
        assert_eq!(async_runtime::block_on(async { 3 }), 3);
        let n = async_runtime::runtime().block_on(async {
            async_runtime::spawn_blocking(|| async_runtime::block_on(async { 4 })).await.unwrap()
        });
        assert_eq!(n, 4);
    }
}
