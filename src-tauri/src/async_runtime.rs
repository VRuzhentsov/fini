//! Tokio-backed async runtime compatibility for headless CLI builds.
//!
//! Shared services refer to `tauri::async_runtime`; in a CLI-only build this crate
//! aliases itself as `tauri` and exposes the same minimal scheduling surface without
//! bringing desktop Tauri dependencies into the artifact graph.

use std::future::Future;

pub fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    tokio::runtime::Runtime::new()
        .expect("create Tokio runtime for headless CLI")
        .block_on(future)
}

pub fn spawn<F>(future: F) -> std::thread::JoinHandle<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    std::thread::spawn(move || block_on(future))
}
