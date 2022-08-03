use std::io;
use std::path::{Path, PathBuf};

use mmap_bitvec::combinatorial::rank;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::bfield_member::{BFieldLookup, BFieldMember, BFieldVal};

/// The struct holding the various bfields
pub struct BField<T> {
    members: Vec<BFieldMember<T>>,
    read_only: bool,
}

// This is safe in theory, as the mmap is send+sync
unsafe impl<T> Send for BField<T> {}
unsafe impl<T> Sync for BField<T> {}

impl<T: Clone + DeserializeOwned + Serialize> BField<T> {
    /// The (complicated) method to create a bfield.
    /// The bfield files will be created in `directory` with the given `filename` and the
    /// suffixes `(0..n_secondaries).bfd`
    /// `size` is the primary bfield size, subsequent bfield sizes will be determined by
    /// `secondary_scaledown` and `max_scaledown`.
    /// If you set `in_memory` to true, remember to call `persist_to_disk` when it's built to
    /// save it.
    /// The params are the following in the paper:
    /// `n_hashes` -> k
    /// `marker_width` -> v (nu)
    /// `n_marker_bits` -> κ (kappa)
    /// `secondary_scaledown` -> β (beta)
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
            let file = directory.as_ref().join(format!("{}.{}.bfd", filename, n));
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

    /// Loads the bfield given the path to the "main" db path (eg the one ending with `0.bfd`).
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

    /// Write the current bfields to disk.
    /// Only useful if you are creating a bfield in memory
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

    /// Returns (n_hashes, marker_width, n_marker_bits, Vec<size of each member>)
    pub fn build_params(&self) -> (u8, u8, u8, Vec<usize>) {
        let (_, n_hashes, marker_width, n_marker_bits) = self.members[0].info();
        let sizes = self.members.iter().map(|i| i.info().0).collect();
        (n_hashes, marker_width, n_marker_bits, sizes)
    }

    /// Returns the params given at build time to the bfields
    pub fn params(&self) -> &Option<T> {
        &self.members[0].params.other
    }

    /// This doesn't actually update the file, so we can use it to e.g.
    /// simulate params on an old legacy file that may not actually have
    /// them set.
    pub fn mock_params(&mut self, params: T) {
        self.members[0].params.other = Some(params);
    }

    /// This allows an insert of a value into the b-field after the entire
    /// b-field build process has been completed.
    ///
    /// It has the very bad downside of potentially knocking other keys out
    /// of the b-field by making them indeterminate (which will make them fall
    /// back to the secondaries where they don't exist and thus it'll appear
    /// as if they were never inserted to begin with)
    pub fn force_insert(&self, key: &[u8], value: BFieldVal) {
        debug_assert!(!self.read_only, "Can't insert into read_only bfields");
        for secondary in &self.members {
            if secondary.mask_or_insert(key, value) {
                break;
            }
        }
    }

    /// Insert the given key/value at the given pass
    /// Returns whether the value was inserted during this call, eg will return `false` if
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

    /// Returns the value of the given key if found, None otherwise.
    /// If the value is indeterminate, we still return None.
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

    /// Get the info of each member
    /// Returns Vec<(size, n_hashes, marker_width, n_marker_bits)>
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
        bfield.persist_to_disk().unwrap();
        for m in &bfield.members {
            assert!(m.filename.exists());
        }
        for i in 0..max_value {
            let val = bfield.get(&i.to_be_bytes().to_vec()).unwrap();
            assert_eq!(i, val);
        }
    }
}
