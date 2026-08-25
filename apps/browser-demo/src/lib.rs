//! Private WebAssembly/WebGPU browser acceptance host.
//!
//! This package is a repository verification application, not the supported
//! JavaScript SDK planned by the roadmap. The target-neutral host and scene
//! models retain native unit coverage while the browser adapter is compiled
//! only for `wasm32`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[cfg(any(target_arch = "wasm32", test))]
mod diagnostics;
#[cfg(any(target_arch = "wasm32", test))]
mod display;
#[cfg(any(target_arch = "wasm32", test))]
mod host;
#[cfg(any(target_arch = "wasm32", test))]
mod scene;
#[cfg(any(target_arch = "wasm32", test))]
mod streaming;

#[cfg(target_arch = "wasm32")]
mod browser;

#[cfg(target_arch = "wasm32")]
pub use browser::create_viewer;
