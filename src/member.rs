use std::cmp::Ordering;
use std::path::Path;

use bitvec::prelude::*;
use murmurhash3::murmurhash3_x64_128;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::combinatorial::{rank, unrank};


pub type BFieldVal = u32;


#[derive(Debug, PartialEq)]
pub(crate) enum BFieldLookup {
    Indeterminate,
    Some(BFieldVal),
    None,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct BFieldParams<T> {
    n_hashes: u8,      // k
    marker_width: u8,  // nu
    n_marker_bits: u8, // kappa
    pub(crate) other: Option<T>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct BFieldMember<T> {
    bitvec: BitVec,
    pub(crate) params: BFieldParams<T>,
}

impl<T: Clone + DeserializeOwned + Serialize> BFieldMember<T> {
    pub fn create(
        size: usize,
        n_hashes: u8,
        marker_width: u8,
        n_marker_bits: u8,
        other_params: Option<T>,
    ) -> Self {
        let bf_params = BFieldParams {
            n_hashes,
            marker_width,
            n_marker_bits,
            other: other_params,
        };

        let mut bv = BitVec::with_capacity(size);
        for _ in 0..size {
            bv.push(false);
        }

        BFieldMember {
            bitvec: bv,
            params: bf_params,
        }
    }

    pub fn insert(&mut self, key: &[u8], value: BFieldVal) {
        // TODO: need to do a check that `value` < allowable range based on
        // self.params.marker_width and self.params.n_marker_bits
        self.insert_raw(key, rank(value as usize, self.params.n_marker_bits));
    }

    /// The marker is essentially a bit array converted to a u128
    #[inline]
    fn insert_raw(&mut self, key: &[u8], marker: u128) {
        let marker_width = self.params.marker_width as usize;
        let hash = murmurhash3_x64_128(key, 0);

        // We only care about the ones, so we skip all the leading zeros below
        let first_1 = marker.leading_zeros() as usize;
        for marker_ix in 0..(self.params.n_hashes as usize) {
            let pos = marker_pos(hash, marker_ix, self.bitvec.len(), marker_width);
            for (i, j) in (pos..(pos + marker_width)).rev().enumerate() {
                if marker & (1 << i) != 0 {
                    self.bitvec.set(j, true);
                }
                if i >= 128 - first_1 {
                    break;
                }
            }
        }
    }

    #[inline]
    pub fn get(&self, key: &[u8]) -> BFieldLookup {
        let k = u32::from(self.params.n_marker_bits);
        let putative_marker = self.get_raw(key, k);
        match putative_marker.count_ones().cmp(&k) {
            Ordering::Greater => BFieldLookup::Indeterminate,
            Ordering::Equal => BFieldLookup::Some(unrank(putative_marker) as u32),
            Ordering::Less => BFieldLookup::None,
        }
    }

    #[inline]
    fn get_raw(&self, key: &[u8], k: u32) -> u128 {
        assert!(self.params.n_hashes <= 16);

        let marker_width = self.params.marker_width as usize;
        let hash = murmurhash3_x64_128(key, 0);
        let mut merged_marker = u128::MAX;

        for marker_ix in 0..self.params.n_hashes as usize {
            let pos = marker_pos(hash, marker_ix, self.bitvec.len(), marker_width);

            let mut marker: u128 = 0;
            let bit_slice = &self.bitvec[pos..(pos + marker_width)];
            for i in bit_slice.iter_ones().rev() {
                marker = marker | (1 << marker_width - i - 1);
            }

            merged_marker &= marker;
            if merged_marker.count_ones() < k {
                return 0;
            }
        }

        merged_marker
    }

    /// "Removes" a key from the b-field by flipping an extra bit to make it
    /// indeterminate. Use this with caution because it can make other keys
    /// indeterminate by saturating the b-field with ones.
    ///
    /// Returns `true` if the value was inserted or was already present with
    /// the correct value; `false` if masking occurred or if it was already
    /// indeterminate.
    pub fn mask_or_insert(&mut self, key: &[u8], value: BFieldVal) -> bool {
        let correct_marker = rank(value as usize, self.params.n_marker_bits);
        let k = self.params.n_marker_bits as u32;
        let existing_marker = self.get_raw(key, k);

        match existing_marker.count_ones().cmp(&k) {
            Ordering::Greater => false, // already indeterminate
            Ordering::Equal => {
                // value already in b-field, but is it correct?
                if existing_marker == correct_marker {
                    return true;
                }
                // try to find a new, invalid marker that has an extra
                // bit over the existing marker so that it'll become
                // indeterminate once we overwrite it
                let mut pos = 0;
                let mut new_marker = existing_marker;
                while new_marker.count_ones() == k {
                    new_marker = existing_marker | (1 << pos);
                    pos += 1;
                }
                // mask out the existing!
                self.insert_raw(key, new_marker);
                false
            }
            Ordering::Less => {
                // nothing present; insert the value
                self.insert_raw(key, correct_marker);
                true
            }
        }
    }

    pub fn info(&self) -> (usize, u8, u8, u8) {
        (
            self.bitvec.len(),
            self.params.n_hashes,
            self.params.marker_width,
            self.params.n_marker_bits,
        )
    }
}

#[inline]
fn marker_pos(hash: (u64, u64), n: usize, total_size: usize, marker_size: usize) -> usize {
    ((hash.0 as usize).wrapping_add(n.wrapping_mul(hash.1 as usize))) % (total_size - marker_size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bfield() {
        let mut bfield: BFieldMember<usize> = BFieldMember::create(1024, 3, 64, 4, Some(0_usize));
        // check that inserting keys adds new entries
        bfield.insert(b"test", 2);
        assert_eq!(bfield.get(b"test"), BFieldLookup::Some(2));

        bfield.insert(b"test2", 106);
        assert_eq!(bfield.get(b"test2"), BFieldLookup::Some(106));

        // test3 was never added
        assert_eq!(bfield.get(b"test3"), BFieldLookup::None);
    }

    #[test]
    fn test_bfield_collisions() {
        // comically small bfield with too many (16) hashes
        // and too many bits (8) to cause saturation
        let mut bfield: BFieldMember<usize> = BFieldMember::create(128, 16, 64, 8, Some(0_usize));

        bfield.insert(b"test", 100);
        assert_eq!(bfield.get(b"test"), BFieldLookup::Indeterminate);
    }

    #[test]
    fn test_bfield_bits_set() {
        let mut bfield: BFieldMember<usize> = BFieldMember::create(128, 2, 16, 4, Some(0_usize));

        bfield.insert(b"test", 100);
        assert_eq!(bfield.bitvec.count_ones(), 8);
        bfield.insert(b"test2", 200);
        assert_eq!(bfield.bitvec.count_ones(), 16);
        bfield.insert(b"test3", 300);
        assert_eq!(bfield.bitvec.count_ones(), 23);
    }

    #[test]
    fn test_bfieild_mask_or_insert() {
        let mut bfield: BFieldMember<usize> = BFieldMember::create(1024, 2, 16, 4, Some(0_usize));

        bfield.insert(b"test", 2);
        assert_eq!(bfield.get(b"test"), BFieldLookup::Some(2));

        // `mask_or_insert`ing the same value doesn't change anything
        assert_eq!(bfield.mask_or_insert(b"test", 2), true);
        assert_eq!(bfield.get(b"test"), BFieldLookup::Some(2));

        // `mask_or_insert`ing a new value results in an indeterminate
        assert_eq!(bfield.mask_or_insert(b"test", 3), false);
        assert_eq!(bfield.get(b"test"), BFieldLookup::Indeterminate);

        // `mask_or_insert`ing an indeterminate value is still indeterminate
        assert_eq!(bfield.mask_or_insert(b"test", 3), false);
        assert_eq!(bfield.get(b"test"), BFieldLookup::Indeterminate);

        // `mask_or_insert`ing a new key just sets that key
        assert_eq!(bfield.mask_or_insert(b"test2", 2), true);
        assert_eq!(bfield.get(b"test2"), BFieldLookup::Some(2));
    }
}
