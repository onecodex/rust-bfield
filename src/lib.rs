#![deny(missing_docs)]

//! The B-field datastructure, implemented in Rust.
//! A space-efficient, probabilistic data structure and storage and retrieval method for key-value information.
//! These Rust docs represent some minimal documentation of the crate itself.
//! See the [Github README](https://github.com/onecodex/rust-bfield) for an
//! extensive write-up, including the math and design underpinning the B-field
//! data structure, guidance on B-field parameter selection, as well as usage
//! examples.[^1]
//!
//! [^1]: These are not embeddable in the Cargo docs as they include MathJax,
//! which is currently unsupported.

mod bfield;
mod bfield_member;
/// Some combinatorial utilities
mod combinatorial;

pub use crate::bfield::BField;
pub use crate::bfield_member::BFieldVal;
pub use combinatorial::choose;
