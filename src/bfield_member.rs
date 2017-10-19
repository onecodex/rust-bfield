use std::cmp::Ordering;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::Path;

use bincode::{serialize, deserialize, Infinite};
use mmap_bitvec::{BitVec, BitVecSlice};
use mmap_bitvec::bloom::MurmurHasher;

use marker::{from_marker, to_marker};


#[derive(Deserialize, Serialize)]
struct BFieldParams {
    n_hashes: u8, // k
    marker_width: u8, // nu
    n_marker_bits: u8, // kappa
    secondary_path: Option<String>, // path to a child lookup table
}

pub(crate) struct BFieldMember {
    bitvec: BitVec,
    params: BFieldParams,
}

pub type BFieldVal = u32;
const BF_MAGIC: [u8; 2] = [0xBF, 0x1D];

#[derive(Debug, PartialEq)]
pub(crate) enum BFieldLookup {
    Indeterminate,
    Some(BFieldVal),
    None,
}

impl BFieldMember {
    pub fn create<P>(
        filename: P,
        size: usize,
        n_hashes: u8,
        marker_width: u8,
        n_marker_bits: u8,
        secondary_path: Option<String>,
    ) -> Result<Self, io::Error>
    where
        P: AsRef<Path>,
    {
        let bf_params = BFieldParams {
            n_hashes: n_hashes,
            marker_width: marker_width,
            n_marker_bits: n_marker_bits,
            secondary_path: secondary_path,
        };

        let header: Vec<u8> = serialize(&bf_params, Infinite).unwrap();
        let bv = BitVec::create(filename, size, &BF_MAGIC, &header)?;

        Ok(BFieldMember {
            bitvec: bv,
            params: bf_params,
        })
    }

    pub fn open<P>(filename: P, read_only: bool) -> Result<Self, io::Error>
    where
        P: AsRef<Path>,
    {
        let bv = BitVec::open(filename, Some(&BF_MAGIC), read_only)?;
        let bf_params: BFieldParams = {
            let header = bv.header();
            deserialize(&header[..]).unwrap()
        };

        Ok(BFieldMember {
            bitvec: bv,
            params: bf_params,
        })
    }

    pub fn in_memory(
        size: usize,
        n_hashes: u8,
        marker_width: u8,
        n_marker_bits: u8,
    ) -> Result<Self, io::Error> {
        let bf_params = BFieldParams {
            n_hashes: n_hashes,
            marker_width: marker_width,
            n_marker_bits: n_marker_bits,
            secondary_path: None,
        };

        let header: Vec<u8> = serialize(&bf_params, Infinite).unwrap();
        let bv = BitVec::from_memory(size)?;

        Ok(BFieldMember {
            bitvec: bv,
            params: bf_params,
        })
    }

    pub fn secondary(&self) -> Option<String> {
        self.params.secondary_path.clone()
    }

    pub fn insert<H>(&mut self, key: H, value: BFieldVal)
    where
        H: Hash,
    {
        // TODO: need to do a check that `value` < allowable range based on
        // self.params.marker_width and self.params.n_marker_bits
        let k = self.params.n_marker_bits;
        self.insert_raw(key, to_marker(value, k));
    }

    #[inline]
    fn insert_raw<H>(&mut self, key: H, value: BitVecSlice)
    where
        H: Hash,
    {
        let marker_width = self.params.marker_width as usize;
        let mut hasher = MurmurHasher::new();
        let size = self.bitvec.size() - marker_width;
        key.hash(&mut hasher);
        let hash: (u64, u64) = hasher.values();

        for marker_ix in 0usize..self.params.n_hashes as usize {
            let pos = ((hash.0 as usize).wrapping_add(marker_ix.wrapping_mul(hash.1 as usize))) %
                size;
            self.bitvec.set_range(pos..pos + marker_width, value);
        }
    }

    pub fn get<H>(&self, key: H) -> BFieldLookup
    where
        H: Hash,
    {
        let k = u32::from(self.params.n_marker_bits);
        let putative_marker = self.get_raw(key);
        match putative_marker.count_ones().cmp(&k) {
            Ordering::Greater => BFieldLookup::Indeterminate,
            Ordering::Equal => BFieldLookup::Some(from_marker(putative_marker)),
            Ordering::Less => BFieldLookup::None,
        }
    }

    #[inline]
    fn get_raw<H>(&self, key: H) -> BitVecSlice
    where
        H: Hash,
    {
        let marker_width = self.params.marker_width as usize;
        let mut hasher = MurmurHasher::new();
        let size = self.bitvec.size() - marker_width;
        key.hash(&mut hasher);
        let hash: (u64, u64) = hasher.values();

        let mut merged_marker = BitVecSlice::max_value();
        for marker_ix in 0usize..self.params.n_hashes as usize {
            let pos = ((hash.0 as usize).wrapping_add(marker_ix.wrapping_mul(hash.1 as usize))) %
                size;
            let marker = self.bitvec.get_range(pos..pos + marker_width);
            merged_marker &= marker;
        }
        merged_marker
    }
}


#[test]
fn test_bfield() {
    let mut bfield = BFieldMember::in_memory(1024, 3, 64, 4).unwrap();
    // check that inserting keys adds new entries
    bfield.insert("test", 2);
    assert_eq!(bfield.get("test"), BFieldLookup::Some(2));

    bfield.insert("test2", 106);
    assert_eq!(bfield.get("test2"), BFieldLookup::Some(106));

    // test3 was never added
    assert_eq!(bfield.get("test3"), BFieldLookup::None);
}

#[test]
fn test_bfield_collisions() {
    // comically small bfield with too many hashes to cause saturation
    let mut bfield = BFieldMember::in_memory(128, 50, 16, 4).unwrap();

    bfield.insert("test", 100);
    assert_eq!(bfield.get("test"), BFieldLookup::Indeterminate);
}
