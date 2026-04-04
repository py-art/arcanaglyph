// crates/arcanaglyph-core/src/lib.rs

pub mod audio;
pub mod config;
pub mod engine;
pub mod error;
pub mod input;

pub use config::CoreConfig;
pub use engine::{ArcanaEngine, EngineEvent};
pub use error::ArcanaError;
