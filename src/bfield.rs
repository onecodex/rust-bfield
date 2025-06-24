use std::io;
use std::path::{Path, PathBuf};

use crate::combinatorial::rank;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::bfield_member::{BFieldLookup, BFieldMember, BFieldVal};

/// The `struct` holding the `BField` primary and secondary bit arrays.
pub struct BField<T> {
    members: Vec<BFieldMember<T>>,
    read_only: bool,
}

// This is safe in theory, as the mmap is send+sync
unsafe impl<T> Send for BField<T> {}
unsafe impl<T> Sync for BField<T> {}

impl<T: Clone + DeserializeOwned + Serialize> BField<T> {
    /// A (rather complex) method for creating a `BField`.
    ///
    /// This will create a series of `BField` bit array files in `directory` with the given `filename` and the
    /// suffixes `(0..n_secondaries).bfd`. If you set `in_memory` to true, remember to call `persist_to_disk` once it's built to
    /// save it.
    ///
    /// The following parameters are required. See the [README.md](https://github.com/onecodex/rust-bfield/)
    /// for additional details as well as the
    /// [parameter selection notebook](https://github.com/onecodex/rust-bfield/blob/main/docs/notebook/calculate-parameters.ipynb)
    /// for helpful guidance in picking optimal parameters.
    /// - `size` is the primary `BField` size, subsequent `BField` sizes will be determined
    ///   by the `secondary_scaledown` and `max_scaledown` parameters
    /// - `n_hashes`. The number of hash functions _k_ to use.
    /// - `marker_width` or v (nu). The length of the bit-string to use for
    /// - `n_marker_bits` or κ (kappa). The number of 1s to set in each v-length bit-string (also its Hamming weight).
    /// - `secondary_scaledown` or β (beta). The scaling factor to use for each subsequent `BField` size.
    /// - `max_scaledown`. A maximum scaling factor to use for secondary `BField` sizes, since β raised to the power of
    ///   `n_secondaries` can be impractically/needlessly small.
    /// - `n_secondaries`. The number of secondary `BField`s to create.
    /// - `in_memory`. Whether to create the `BField` in memory or on disk.
    #[allow(clippy::too_many_arguments)]
    pub fn create<P>(
        directory: P,
        filename: &str,
        size: usize,
        n_hashes: u8,             // k
        marker_width: u8,         // nu
        n_marker_bits: u8,        // kappa
        secondary_scaledown: f64, // beta
        max_scaledown: f64,
        n_secondaries: u8,
        in_memory: bool,
        other_params: T,
    ) -> Result<Self, io::Error>
    where
        P: AsRef<Path>,
    {
        debug_assert!(!filename.is_empty());
        let mut cur_size = size;
        let mut members = Vec::new();

        for n in 0..n_secondaries {
            let file = directory.as_ref().join(format!("{filename}.{n}.bfd"));
            let params = if n == 0 {
                Some(other_params.clone())
            } else {
                None
            };
            let member = BFieldMember::create(
                file,
                in_memory,
                cur_size,
                n_hashes,
                marker_width,
                n_marker_bits,
                params,
            )?;
            members.push(member);
            cur_size = f64::max(
                cur_size as f64 * secondary_scaledown,
                size as f64 * max_scaledown,
            ) as usize;
        }

        // Initialize our marker table, so we don't
        // have any race conditions across threads
        let _ = rank(0, n_marker_bits);

        Ok(BField {
            members,
            read_only: false,
        })
    }

    /// Loads the `BField` given the path to the primary array data file (eg the one ending with `0.bfd`).
    pub fn load<P: AsRef<Path>>(main_db_path: P, read_only: bool) -> Result<Self, io::Error> {
        let mut members = Vec::new();
        let mut n = 0;

        let main_db_filename = match main_db_path.as_ref().file_name() {
            Some(p) => p.to_string_lossy(),
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("Couldn't get filename from {:?}", main_db_path.as_ref()),
                ));
            }
        };
        assert!(main_db_path.as_ref().parent().is_some());
        assert!(main_db_filename.ends_with("0.bfd"));

        loop {
            let member_filename =
                PathBuf::from(&main_db_filename.replace("0.bfd", &format!("{n}.bfd")));
            let member_path = main_db_path
                .as_ref()
                .parent()
                .unwrap()
                .join(member_filename);
            if !member_path.exists() {
                break;
            }
            let member = BFieldMember::open(&member_path, read_only)?;
            members.push(member);
            n += 1;
        }

        if members.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("No Bfield found at {:?}", main_db_path.as_ref()),
            ));
        }
        Ok(BField { members, read_only })
    }

    /// Write the current `BField` to disk.
    /// Only useful if you are creating a `BField` in memory.
    pub fn persist_to_disk(self) -> Result<Self, io::Error> {
        let mut members = Vec::with_capacity(self.members.len());
        for m in self.members {
            members.push(m.persist_to_disk()?);
        }
        Ok(Self {
            members,
            read_only: self.read_only,
        })
    }

    /// Returns `(n_hashes, marker_width, n_marker_bits, Vec<size of each member>)`.
    pub fn build_params(&self) -> (u8, u8, u8, Vec<usize>) {
        let (_, n_hashes, marker_width, n_marker_bits) = self.members[0].info();
        let sizes = self.members.iter().map(|i| i.info().0).collect();
        (n_hashes, marker_width, n_marker_bits, sizes)
    }

    /// Returns the params given at build time to the `BField` arrays.
    pub fn params(&self) -> &Option<T> {
        &self.members[0].params.other
    }

    /// ⚠️ Method for setting parameters without actually updating any files on disk. **Only useful for supporting legacy file formats
    /// in which these parameters are not saved.**
    pub fn mock_params(&mut self, params: T) {
        self.members[0].params.other = Some(params);
    }

    /// ⚠️ Method for inserting a value into a `BField`
    /// after it has been fully built and finalized.
    /// **This method should be used with extreme care**
    /// as it does not guarantee that keys are properly propagated
    /// to secondary arrays and therefore may make lookups of previously
    /// set values return an indeterminate result in the primary array,
    /// then causing fallback to the secondary arrays where they were never
    /// inserted (and returning a false negative).
    pub fn force_insert(&self, key: &[u8], value: BFieldVal) {
        debug_assert!(!self.read_only, "Can't insert into read_only bfields");
        for secondary in &self.members {
            if secondary.mask_or_insert(key, value) {
                break;
            }
        }
    }

    /// Insert the given key/value at the given pass (1-indexed `BField` array/member).
    /// Returns whether the value was inserted during this call, i.e., will return `false` if
    /// the value was already present.
    pub fn insert(&self, key: &[u8], value: BFieldVal, pass: usize) -> bool {
        debug_assert!(!self.read_only, "Can't insert into read_only bfields");
        debug_assert!(
            pass < self.members.len(),
            "Can't have more passes than bfield members"
        );
        if pass > 0 {
            for secondary in self.members[..pass].iter() {
                match secondary.get(key) {
                    BFieldLookup::Indeterminate => continue,
                    _ => return false,
                }
            }
        }
        self.members[pass].insert(key, value);
        true
    }

    /// Returns the value of the given key if found, `None` otherwise.
    /// The current implementation also returns `None` for indeterminate values.
    pub fn get(&self, key: &[u8]) -> Option<BFieldVal> {
        for secondary in self.members.iter() {
            match secondary.get(key) {
                BFieldLookup::Indeterminate => continue,
                BFieldLookup::Some(value) => return Some(value),
                BFieldLookup::None => return None,
            }
        }
        // TODO: better value for totally indeterminate? panic?
        // or return a Result<Option<BFieldVal>, ...> instead?
        None
    }

    /// Get the info of each secondary array (`BFieldMember`) in the `BField`.
    /// Returns `Vec<(size, n_hashes, marker_width, n_marker_bits)>`.
    pub fn info(&self) -> Vec<(usize, u8, u8, u8)> {
        self.members.iter().map(|m| m.info()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_build_and_query_file_bfield() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let n_secondaries = 4;
        let bfield = BField::create(
            tmp_dir.path(),
            "bfield",
            1_000_000,
            10,
            39,
            4,
            0.1,
            0.025,
            n_secondaries,
            false,
            String::new(),
        )
        .expect("to build");

        // Identity database
        let max_value: u32 = 10_000;
        for p in 0..n_secondaries {
            for i in 0..max_value {
                bfield.insert(&i.to_be_bytes().to_vec(), i, p as usize);
            }
        }

        for i in 0..max_value {
            let val = bfield.get(&i.to_be_bytes().to_vec()).unwrap();
            assert_eq!(i, val);
        }
        drop(bfield);

        // and we can load them
        let bfield = BField::<String>::load(&tmp_dir.path().join("bfield.0.bfd"), true).unwrap();
        for i in 0..max_value {
            let val = bfield.get(&i.to_be_bytes().to_vec()).unwrap();
            assert_eq!(i, val);
        }
    }

    #[test]
    fn can_build_and_query_in_memory_bfield() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let n_secondaries = 4;
        let mut bfield = BField::create(
            tmp_dir.path(),
            "bfield",
            1_000_000,
            10,
            39,
            4,
            0.1,
            0.025,
            n_secondaries,
            true,
            String::new(),
        )
        .expect("to build");

        // Identity database
        let max_value: u32 = 10_000;
        for p in 0..n_secondaries {
            for i in 0..max_value {
                bfield.insert(&i.to_be_bytes().to_vec(), i, p as usize);
            }
        }

        for i in 0..max_value {
            let val = bfield.get(&i.to_be_bytes().to_vec()).unwrap();
            assert_eq!(i, val);
        }
        bfield = bfield.persist_to_disk().unwrap();
        for m in &bfield.members {
            assert!(m.filename.exists());
        }
        for i in 0..max_value {
            let val = bfield.get(&i.to_be_bytes().to_vec()).unwrap();
            assert_eq!(i, val);
        }
    }
}

// Causes cargo test to run doc tests on all `rust` code blocks
#[doc = include_str!("../README.md")]
#[cfg(doctest)]
struct ReadmeDoctests;
