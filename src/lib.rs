//! Core of the zeno editor: document access, line indexing, edit overlay and
//! viewport rendering.
//!
//! The crate is split so that the performance-critical parts (mapping the
//! file, discovering lines, painting the viewport) can be measured in
//! isolation by the `zeno-bench` binary, without a terminal attached.

pub mod clipboard;
pub mod complete;
pub mod config;
pub mod doc;
pub mod finder;
pub mod highlight;
pub mod indent;
pub mod keys;
pub mod motion;
pub mod panel;
pub mod prompt;
pub mod recent;
pub mod render;
pub mod search;
pub mod sequence;
pub mod theme;

pub use doc::{Doc, LineIndex, Overlay};
pub use render::{View, ViewState};
