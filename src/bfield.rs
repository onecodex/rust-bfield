use std::io;
use std::path::Path;

use mmap_bitvec::combinatorial::rank;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::bfield_member::{BFieldLookup, BFieldMember, BFieldVal};

pub struct BField<T> {
    members: Vec<BFieldMember<T>>,
    read_only: bool,
}

impl<T: Clone + DeserializeOwned + Serialize> BField<T> {
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

    pub fn from_file<P>(filename: P, read_only: bool) -> Result<Self, io::Error>
    where
        P: AsRef<Path>,
    {
        let mut members = Vec::new();
        let mut n = 0;
        loop {
            let member_filename = filename.as_ref().with_file_name(Path::with_extension(
                Path::file_stem(filename.as_ref()).unwrap().as_ref(),
                format!("{}.bfd", n),
            ));
            if !member_filename.exists() {
                break;
            }
            let member = BFieldMember::open(&member_filename, read_only)?;
            members.push(member);
            n += 1;
        }
        if members.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("No Bfield found at {:?}", filename.as_ref()),
            ));
        }
        Ok(BField { members, read_only })
    }

    pub fn build_params(&self) -> (u8, u8, u8, Vec<usize>) {
        let (_, n_hashes, marker_width, n_marker_bits) = self.members[0].info();
        let sizes = self.members.iter().map(|i| i.info().0).collect();
        (n_hashes, marker_width, n_marker_bits, sizes)
    }

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
    pub fn force_insert(&mut self, key: &[u8], value: BFieldVal) {
        debug_assert!(!self.read_only, "Can't insert into read_only bfields");
        for secondary in self.members.iter_mut() {
            if secondary.mask_or_insert(key, value) {
                break;
            }
        }
    }

    pub fn insert(&mut self, key: &[u8], value: BFieldVal, pass: usize) -> bool {
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

    pub fn info(&self) -> Vec<(usize, u8, u8, u8)> {
        self.members.iter().map(|m| m.info()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_build_and_query_bfield() {
        let mut tmp_dir = tempfile::tempdir().unwrap();
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
    }
}
