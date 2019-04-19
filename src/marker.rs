#![cfg_attr(feature = "const_fn", feature(marker_lookup))]

use mmap_bitvec::BitVecSlice;

use crate::bfield_member::BFieldVal;

#[cfg(feature = "marker_lookup")]
const MARKER_TABLE_SIZE: usize = 200000;
#[cfg(feature = "marker_lookup")]
static mut MARKER_TABLE: [BitVecSlice; MARKER_TABLE_SIZE] = [0; MARKER_TABLE_SIZE];
#[cfg(feature = "marker_lookup")]
static mut MARKER_BITS: u8 = 0;

#[cfg(feature = "marker_lookup")]
pub fn to_marker(value: BFieldVal, k: u8) -> BitVecSlice {
    // note that you can only test this feature with
    // `RUST_TEST_THREADS=1 cargo test` or else you'll get tons of
    // errors because of data races between threads with different k's

    // also some tests (i.e. ones with k=1) fail because there are fewer
    // than 200000 possible values for the table
    unsafe {
        // clear out the lookup table if we have a new k and fill
        // it with values for the new k
        if MARKER_BITS != k {
            MARKER_TABLE = [0; MARKER_TABLE_SIZE];
            MARKER_TABLE[0] = (1 << k) - 1 as BitVecSlice;
            for i in 1..MARKER_TABLE_SIZE {
                MARKER_TABLE[i] = next_marker(MARKER_TABLE[i - 1]);
            }
            MARKER_BITS = k;
        }
        if value as usize >= MARKER_TABLE_SIZE {
            let mut marker = MARKER_TABLE[MARKER_TABLE_SIZE - 1];
            for _ in 0..(value as usize - MARKER_TABLE_SIZE) {
                marker = next_marker(marker);
            }
            marker
        } else {
            MARKER_TABLE[value as usize]
        }
    }
}

#[cfg(not(feature = "marker_lookup"))]
pub fn to_marker(value: BFieldVal, k: u8) -> BitVecSlice {
    // set the appropriate number of bits in the marker
    let mut marker = (1 << k) - 1 as BitVecSlice;
    // just step through `value` times until we find the marker we want
    // (this could be speed up *a lot* with some kind of lookup table)
    for _ in 0..value {
        marker = next_marker(marker)
    }
    marker
}

#[test]
fn test_to_marker() {
    assert_eq!(to_marker(0, 3), 7);
    assert_eq!(to_marker(2, 3), 13);
    assert_eq!(to_marker(0, 3).count_ones(), 3);
    assert_eq!(to_marker(2, 3).count_ones(), 3);
    assert_eq!(to_marker(35001, 4).count_ones(), 4);

    // Maximum value of 64 choose 3
    assert_eq!(to_marker(41663, 3).count_ones(), 3);
}

pub fn from_marker(marker: BitVecSlice) -> BFieldVal {
    // val = choose(rank(0), 1) + choose(rank(1), 2) + choose(rank(2), 3) + ...
    let mut working_marker = marker;
    let mut value = 0u64;
    let mut idx = 0;
    while working_marker != 0 {
        let rank = u64::from(working_marker.trailing_zeros());
        working_marker -= 1 << rank;
        idx += 1;
        value += choose(rank, idx);
    }
    value as BFieldVal
}

#[test]
fn test_from_marker() {
    // 3 bit markers
    assert_eq!(from_marker(7), 0);
    assert_eq!(from_marker(13), 2);
}

#[test]
fn test_to_and_from_marker() {
    for k in 1..4u8 {
        for value in [1 as BFieldVal, 23, 45].iter() {
            assert_eq!(from_marker(to_marker(*value, k)), *value);
        }
    }
}

/// (Hopefully) fast implementation of a binomial
///
/// This may overflow for k > 9 (with n = 128), but hopefully
/// that doesn't happen
#[inline]
fn choose(n: u64, k: u8) -> u64 {
    match k {
        1 => n,
        2 => n * (n - 1) / 2,
        3 => n * (n - 1) * (n - 2) / 6,
        4 => n * (n - 1) * (n - 2) * (n - 3) / 24,
        5 => n * (n - 1) * (n - 2) * (n - 3) * (n - 4) / 120,
        6 => n * (n - 1) * (n - 2) * (n - 3) * (n - 4) * (n - 5) / 720,
        7 => n * (n - 1) * (n - 2) * (n - 3) * (n - 4) * (n - 5) * (n - 6) / 5040,
        _ => unimplemented!(),
        // TODO: put a slow implementation here for >7?
    }
}

#[test]
fn test_choose() {
    assert_eq!(choose(1, 1), 1);
    assert_eq!(choose(10, 1), 10);

    assert_eq!(choose(5, 2), 10);

    assert_eq!(choose(5, 3), 10);

    assert_eq!(choose(5, 4), 5);

    assert_eq!(choose(5, 5), 1);
    assert_eq!(choose(20, 5), 15504);

    assert_eq!(choose(20, 6), 38760);

    assert_eq!(choose(20, 7), 77520);
    assert_eq!(choose(23, 7), 245157);
}

#[inline]
fn next_marker(marker: BitVecSlice) -> BitVecSlice {
    let t = marker | (marker - 1);
    (t + 1) | (((!t & (t + 1)) - 1) >> (marker.trailing_zeros() + 1))
}

#[test]
fn test_next_marker() {
    assert_eq!(next_marker(0b1), 0b10);
    assert_eq!(next_marker(0b100), 0b1000);

    assert_eq!(next_marker(0b111), 0b1011);
    assert_eq!(next_marker(0b1000101), 0b1000110);
}
