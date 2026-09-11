//! `sujay-core` — host orchestration with no windowing and no UI.
//!
//! Owns the audio engine, preferences, background decode and rekordbox
//! library loading, and produces the state a UI shows. The macOS host drives
//! it from its event loop; an FFI layer drives it from Swift.

pub mod decode;
pub mod library;
pub mod preferences;
pub mod runtime;
pub mod state;

pub use runtime::{Core, DeckBuffers, DeckId, Tick};
