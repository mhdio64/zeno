//! Core of the zeno editor: document access, line indexing, edit overlay and
//! viewport rendering.
//!
//! The crate is split so that the performance-critical parts (mapping the
//! file, discovering lines, painting the viewport) can be measured in
//! isolation by the `zeno-bench` binary, without a terminal attached.

pub mod clipboard;
pub mod doc;
pub mod motion;
pub mod prompt;
pub mod search;
pub mod render;
pub mod sequence;

pub use doc::{Doc, LineIndex, Overlay};
pub use render::{View, ViewState};
