#![cfg_attr(feature = "prefetching", feature(core_intrinsics))]

mod bfield;
mod bfield_member;
mod combinatorial;
mod member;

pub use crate::bfield::BField;
pub use crate::bfield_member::BFieldVal;
pub use mmap_bitvec::combinatorial::choose;
