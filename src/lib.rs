#![deny(missing_docs)]

//! The bfield datastructure, implemented in Rust.
//! A space-efficient, probabilistic data structure and storage and retrieval method for key-value information.

mod bfield;
mod bfield_member;
/// Some combinatorial utilities
mod combinatorial;

pub use crate::bfield::BField;
pub use crate::bfield_member::BFieldVal;
pub use combinatorial::choose;
