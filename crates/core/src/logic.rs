//! Pure logic ported from `src/lib` and `src/store` (no IO, no UI). The app wraps these in entities.

pub mod deck;
pub mod layout;
pub mod orchestrator;
pub mod permissions;
pub mod queue;
pub mod ring_buffer;
pub mod settings;
pub mod tasks;
pub mod templates;
pub mod text;
pub mod theme;
pub mod voice_route;
