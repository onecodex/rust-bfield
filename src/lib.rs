#![cfg_attr(feature = "marker_lookup", feature(const_fn))]
#![cfg_attr(feature = "prefetching", feature(core_intrinsics))]

#[macro_use]
extern crate serde_derive;

mod bfield;
mod bfield_member;

pub use crate::bfield::BField;
pub use crate::bfield_member::BFieldVal;
pub use mmap_bitvec::combinatorial::choose;
