#![cfg_attr(feature = "marker_lookup", feature(const_fn))]
#![cfg_attr(feature = "prefetching", feature(core_intrinsics))]
extern crate bincode;
extern crate mmap_bitvec;
extern crate murmurhash3;
extern crate serde;
#[macro_use]
extern crate serde_derive;
#[cfg(feature = "legacy")]
extern crate serde_json;

mod bfield;
mod bfield_member;
mod marker;

pub use bfield::BField;
pub use bfield_member::BFieldVal;
