use indexmap::IndexMap;
use rand::Rng;
use std::ops::Range;

use log::{error, info};
use monero::cryptonote::subaddress;

use crate::SubIndex;

const INITIAL_SUBADDRESSES: u32 = 10;

pub(crate) struct SubaddressCache {
    major: Range<u32>,
    minor: Range<u32>,
    subaddresses: IndexMap<SubIndex, String>,
}

impl SubaddressCache {
    pub fn init(viewpair: &monero::ViewPair) -> SubaddressCache {
        info!("Initializing subaddress cache with {} subaddresses", INITIAL_SUBADDRESSES);
        // TODO: Change this to use extend_by().
        let major = 0..1;
        let minor = 1..INITIAL_SUBADDRESSES + 1;
        let subaddresses =
            SubaddressCache::generate_range(major.to_owned(), minor.to_owned(), viewpair)
                .into_iter()
                .collect();

        SubaddressCache {
            major,
            minor,
            subaddresses,
        }
    }

    pub fn pop_random(&mut self) -> (SubIndex, String) {
        let mut rng = rand::thread_rng();
        let map_index = rng.gen_range(0..self.subaddresses.len());

        match self.subaddresses.shift_remove_index(map_index) {
            Some((sub_index, subaddress)) => (sub_index, subaddress),
            None => {
                error!("Failed to retrieve subaddress by index from subaddress cache; retrying");
                self.pop_random()
            }
        }
    }

    pub fn len(&self) -> usize {
        self.subaddresses.len()
    }

    /// Generates `n` subaddresses at the end of the current range, and appends them to the
    /// subaddress cache.
    ///
    /// If adding `n` additional subaddresses would extend the cache beyond the maximum index of
    /// (u32::MAX, u32::MAX), generation stop prematurely.
    ///
    /// Returns the number of subaddresses appended to the cache.
    pub fn extend_by(&mut self, n: u64, viewpair: &monero::ViewPair) -> u64 {
        // TODO: Change this to use generate_range().
        let mut count = 0;
        for _ in 0..n {
            if self.minor.end == u32::MAX {
                if self.major.end == u32::MAX {
                    // We're at the max, time to quit.
                    return count;
                }
                self.major.end += 1;
                self.minor.end = 0;
            }
            let sub_index = SubIndex::new(self.major.end - 1, self.minor.end - 1);
            let subaddress = format!(
                "{}",
                subaddress::get_subaddress(viewpair, sub_index.into(), None)
            );
            self.subaddresses.insert(sub_index, subaddress);
            count += 1;
        }
        count
    }

    /// Generates subaddresses for range of indexes.
    ///
    /// # Panics
    ///
    /// Panics if ending both major and minor ranges start at 0, because (0, 0) is the primary
    /// address index (and therefor is an invalid subaddress index).
    fn generate_range(
        major: Range<u32>,
        minor: Range<u32>,
        viewpair: &monero::ViewPair,
    ) -> Vec<(SubIndex, String)> {
        if major.start == 0 && minor.start == 0 {
            panic!("to avoid the primary address index, major and minor index ranges cannot both start at zero.");
        }

        let mut subaddresses = Vec::new();
        let major_end = major.end;
        let major_start = major.start;
        for major_index in major {
            let mut starting_minor = 0;
            let mut ending_minor = u32::MAX;
            if major_index == major_start {
                starting_minor = minor.start;
            }
            if major_index == major_end - 1 {
                ending_minor = minor.end;
            }
            for minor_index in starting_minor..ending_minor {
                let sub_index = SubIndex::new(major_index, minor_index);
                let subaddress = format!(
                    "{}",
                    subaddress::get_subaddress(viewpair, sub_index.into(), None)
                );
                subaddresses.push((sub_index, subaddress));
            }
        }

        subaddresses
    }
}
