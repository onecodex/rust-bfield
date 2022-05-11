use std::cmp::Ordering;
#[cfg(feature = "prefetching")]
use std::intrinsics;
use std::io;
use std::path::{Path, PathBuf};

use bincode::{deserialize, serialize};
use mmap_bitvec::combinatorial::{rank, unrank};
use mmap_bitvec::{BitVector, MmapBitVec};
use murmurhash3::murmurhash3_x64_128;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

// Empty function on some archs
#[allow(unused_variables)]
#[inline]
fn prefetch_read(pointer: *const u8) {
    #[cfg(all(target_arch = "x86_64", target_feature = "sse"))]
    {
        use std::arch::x86_64 as arch_impl;

        unsafe {
            arch_impl::_mm_prefetch::<{ arch_impl::_MM_HINT_NTA }>(pointer as *const i8);
        }

        return;
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct BFieldParams<T> {
    n_hashes: u8,      // k
    marker_width: u8,  // nu
    n_marker_bits: u8, // kappa
    pub(crate) other: Option<T>,
}

pub(crate) struct BFieldMember<T> {
    bitvec: MmapBitVec,
    // Used when loading mmap in memory to know where to save it if needed
    pub(crate) filename: PathBuf,
    pub(crate) params: BFieldParams<T>,
}

pub type BFieldVal = u32;
const BF_MAGIC: [u8; 2] = [0xBF, 0x1D];

#[derive(Debug, PartialEq)]
pub(crate) enum BFieldLookup {
    Indeterminate,
    Some(BFieldVal),
    None,
}

impl<T: Clone + DeserializeOwned + Serialize> BFieldMember<T> {
    pub fn create<P: AsRef<Path>>(
        filename: P,
        in_memory: bool,
        size: usize,
        n_hashes: u8,
        marker_width: u8,
        n_marker_bits: u8,
        other_params: Option<T>,
    ) -> Result<Self, io::Error> {
        let bf_params = BFieldParams {
            n_hashes,
            marker_width,
            n_marker_bits,
            other: other_params,
        };

        let bv = if in_memory {
            MmapBitVec::from_memory(size)?
        } else {
            let header: Vec<u8> = serialize(&bf_params).unwrap();
            MmapBitVec::create(&filename, size, BF_MAGIC, &header)?
        };

        Ok(BFieldMember {
            filename: filename.as_ref().to_path_buf(),
            bitvec: bv,
            params: bf_params,
        })
    }

    pub fn open<P: AsRef<Path>>(filename: P, read_only: bool) -> Result<Self, io::Error> {
        let bv = MmapBitVec::open(&filename, Some(&BF_MAGIC), read_only)?;
        let bf_params: BFieldParams<T> = {
            let header = bv.header();
            deserialize(header).unwrap()
        };

        Ok(BFieldMember {
            filename: filename.as_ref().to_path_buf(),
            bitvec: bv,
            params: bf_params,
        })
    }

    pub fn persist_to_disk(mut self) -> Result<Self, io::Error> {
        let header: Vec<u8> = serialize(&self.params).unwrap();
        self.bitvec = self
            .bitvec
            .into_mmap_file(&self.filename, BF_MAGIC, &header)?;
        Ok(self)
    }

    pub fn insert(&mut self, key: &[u8], value: BFieldVal) {
        // TODO: need to do a check that `value` < allowable range based on
        // self.params.marker_width and self.params.n_marker_bits
        let k = self.params.n_marker_bits;
        self.insert_raw(key, rank(value as usize, k));
    }

    #[inline]
    fn insert_raw(&mut self, key: &[u8], marker: u128) {
        let marker_width = self.params.marker_width as usize;
        let hash = murmurhash3_x64_128(key, 0);

        for marker_ix in 0usize..self.params.n_hashes as usize {
            let pos = marker_pos(hash, marker_ix, self.bitvec.size(), marker_width);
            self.bitvec.set_range(pos..pos + marker_width, marker);
        }
    }

    /// "Removes" a key from the b-field by flipping an extra bit to make it
    /// indeterminate. Use this with caution because it can make other keys
    /// indeterminate by saturating the b-field with ones.
    ///
    /// Returns `true` if the value was inserted or was already present with
    /// the correct value; `false` if masking occured or if it was already
    /// indeterminate.
    pub fn mask_or_insert(&mut self, key: &[u8], value: BFieldVal) -> bool {
        let correct_marker = rank(value as usize, self.params.n_marker_bits);
        let k = u32::from(self.params.n_marker_bits);
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
        let mut positions: [usize; 16] = [0; 16]; // support up to 16 hashes
        #[allow(clippy::needless_range_loop)]
        for marker_ix in 0usize..self.params.n_hashes as usize {
            let pos = marker_pos(hash, marker_ix, self.bitvec.size(), marker_width);
            positions[marker_ix] = pos;
            unsafe {
                let byte_idx_st = (pos >> 3) as usize;
                let ptr: *const u8 = self.bitvec.mmap.as_ptr().add(byte_idx_st);
                prefetch_read(ptr);
            }
        }

        for pos in positions.iter().take(self.params.n_hashes as usize) {
            let marker = self.bitvec.get_range(*pos..*pos + marker_width);
            merged_marker &= marker;
            if merged_marker.count_ones() < k {
                return 0;
            }
        }
        merged_marker
    }

    pub fn info(&self) -> (usize, u8, u8, u8) {
        (
            self.bitvec.size(),
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
        let mut bfield: BFieldMember<usize> =
            BFieldMember::create("test", true, 1024, 3, 64, 4, None).unwrap();
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
        let mut bfield: BFieldMember<usize> =
            BFieldMember::create("test", true, 128, 16, 64, 8, None).unwrap();

        bfield.insert(b"test", 100);
        assert_eq!(bfield.get(b"test"), BFieldLookup::Indeterminate);
    }

    #[test]
    fn test_bfield_bits_set() {
        let mut bfield: BFieldMember<usize> =
            BFieldMember::create("test", true, 128, 2, 16, 4, None).unwrap();

        bfield.insert(b"test", 100);
        assert_eq!(bfield.bitvec.rank(0..128), 8);
        bfield.insert(b"test2", 200);
        assert_eq!(bfield.bitvec.rank(0..128), 16);
        bfield.insert(b"test3", 300);
        assert!(bfield.bitvec.rank(0..128) < 24); // 23 bits set
    }

    #[test]
    fn test_bfield_mask_or_insert() {
        let mut bfield: BFieldMember<usize> =
            BFieldMember::create("test", true, 1024, 2, 16, 4, None).unwrap();

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
