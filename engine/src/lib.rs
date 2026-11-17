// Interleaved Study's engine: the database, AI queue, keys, timer and scheduling. The native Mac app (app/) links it
// as a static library and talks to it through the C interface in ffi.rs; nothing here draws anything.
pub mod aijson;
mod commands;
pub mod core;
pub mod deadlines;
#[cfg(debug_assertions)]
mod devbridge;
mod ffi;
pub mod files;
pub mod history;
mod host;
pub mod importer;
pub mod jobs;
mod keys;
pub mod organize;
pub mod preprocess;
pub mod provider;
pub mod queue;
pub mod routine;
pub mod sched;
pub mod session;
pub mod settings;
pub mod store;
pub mod types;
pub mod util;
