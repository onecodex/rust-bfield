use std::hash::Hash;
use std::io;
use std::path::Path;

use bfield_member::{BFieldLookup, BFieldMember, BFieldVal};


pub struct BField {
    members: Vec<BFieldMember>,
}

impl BField {
    pub fn create<P>(
        filename: P,
        size: usize,
        n_hashes: u8, // k
        marker_width: u8, // nu
        n_marker_bits: u8, // kappa
        secondary_scaledown: f64, // beta
        max_scaledown: f64,
        n_secondaries: u8,
    ) -> Result<Self, io::Error>
    where
        P: AsRef<Path>,
    {
        let mut cur_size = size;
        let mut members = Vec::new();
        for n in 0..n_secondaries {
            // panics if filename == ''
            let file = Path::with_extension(
                Path::file_stem(filename.as_ref()).unwrap().as_ref(),
                format!("{}.bfd", n),
            );
            let next_file = if n < n_secondaries - 1 {
                Some(
                    Path::with_extension(
                        Path::file_stem(filename.as_ref()).unwrap().as_ref(),
                        format!("{}.bfd", n + 1),
                    ).to_str()
                        .unwrap()
                        .to_string(),
                )
            } else {
                None
            };
            let member = BFieldMember::create(
                file,
                cur_size,
                n_hashes,
                marker_width,
                n_marker_bits,
                next_file,
            )?;
            members.push(member);
            cur_size = f64::max(
                cur_size as f64 * secondary_scaledown,
                size as f64 * max_scaledown,
            ) as usize;
        }

        Ok(BField { members: members })
    }

    pub fn from_file<P>(filename: P, read_only: bool) -> Result<Self, io::Error>
    where
        P: AsRef<Path>,
    {
        let mut member_filename: String = filename.as_ref().to_str().unwrap().to_string();
        let mut members = Vec::new();
        loop {
            let member = BFieldMember::open(&member_filename, read_only)?;
            let secondary = member.secondary();
            members.push(member);
            match secondary {
                Some(filename) => member_filename = filename,
                None => break,
            }
        }
        Ok(BField { members: members })
    }

    pub fn insert<H>(&mut self, key: H, value: BFieldVal)
    where
        H: Hash,
    {
        for secondary in self.members.iter() {
            unimplemented!();
        }
    }

    pub fn get<H>(&self, key: H) -> Option<BFieldVal>
    where
        H: Hash,
    {
        for secondary in self.members.iter() {
            match secondary.get(&key) {
                BFieldLookup::Indeterminate => continue,
                BFieldLookup::Some(value) => return Some(value),
                BFieldLookup::None => return None,
            }
        }
        // TODO: better value for totally indeterminate? panic?
        // or return a Result<Option<BFieldVal>, ...> instead?
        None
    }
}
