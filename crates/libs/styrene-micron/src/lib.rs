//! Micron markup parser for NomadNet/Reticulum page rendering.
//!
//! Parses NomadNet's micron markup language into a structured document
//! model suitable for rendering in Dioxus, ratatui, HTML, or any other
//! display backend.
//!
//! Pinned to the canonical NomadNet specification (nomad_net_guide.mu).
//!
//! # Usage
//!
//! ```
//! use styrene_micron::{parse, Block, InlineNode};
//!
//! let doc = parse(">Heading\nChild text");
//!
//! for block in &doc.blocks {
//!     match block {
//!         Block::Section { level, heading, children } => {
//!             println!("Section level {level}");
//!         }
//!         Block::Line(line) => {
//!             for node in &line.nodes {
//!                 if let InlineNode::Text { style, text } = node {
//!                     println!("{text} (bold={})", style.has_bold());
//!                 }
//!             }
//!         }
//!         _ => {}
//!     }
//! }
//! ```

pub mod model;
pub mod parser;

// Re-export public API at crate root for ergonomic use.
pub use model::*;
pub use parser::parse;
