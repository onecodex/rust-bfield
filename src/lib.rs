#![cfg_attr(feature = "marker_lookup", feature(const_fn))]
extern crate bincode;
extern crate mmap_bitvec;
#[macro_use]
extern crate serde;
#[macro_use]
extern crate serde_derive;

mod bfield;
mod bfield_member;
mod marker;
