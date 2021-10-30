use std::cmp;
use std::ops::Range;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use indexmap::{IndexMap, IndexSet};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha12Rng;

use log::{debug, error};
use monero::{cryptonote::subaddress, ViewPair};

use crate::invoices_db::InvoicesDb;
use crate::{SubIndex};

const MIN_AVAILABLE_SUBADDRESSES: u32 = 100;

pub(crate) struct SubaddressCache {
    highest_minor_index: Arc<AtomicU32>,
    available_subaddresses: IndexMap<SubIndex, String>,
    viewpair: ViewPair,
    rng: ChaCha12Rng,
}

impl SubaddressCache {
    pub fn init(
        invoices_db: &InvoicesDb,
        viewpair: monero::ViewPair,
        highest_minor_index: Arc<AtomicU32>,
        seed: Option<u64>,
    ) -> SubaddressCache {
        // Get currently used subindexes from database, so they won't be put in the list of
        // available subindexes.
        let used_sub_indexes: IndexSet<SubIndex> = invoices_db
            .iter()
            .map(|invoice_or_err| match invoice_or_err {
                Ok(invoice) => invoice.index(),
                Err(e) => {
                    // TODO: Ideally, we'd carry on after logging this error.
                    panic!(
                        "failed to read used subindex from invoice in database: {}",
                        e
                    );
                }
            })
            .collect();

        // Get highest index from list of used subindexes.
        let max_used = if let Some(max_sub_index) = used_sub_indexes.iter().max() {
            debug!(
                "Highest subaddress index in the database: {}",
                SubIndex::new(1, max_sub_index.minor)
            );
            max_sub_index.minor
        } else {
            debug!("Highest subaddress index in the database: N/A");
            0
        };

        // Generate enough subaddresses to cover all pending invoices.
        let minor_index_range = 0..cmp::max(MIN_AVAILABLE_SUBADDRESSES, max_used + 1);
        highest_minor_index.store(minor_index_range.end - 1, Ordering::Relaxed);
        let mut available_subaddresses: IndexMap<SubIndex, String> =
            SubaddressCache::generate_range(1..2, minor_index_range, &viewpair)
                .into_iter()
                .collect();

        // Remove subaddresses that are present in the database.
        available_subaddresses.retain(|sub_index, _| !used_sub_indexes.contains(sub_index));

        // If a seed is supplied, seed the random number generator with it.
        let mut rng = ChaCha12Rng::from_entropy();
        if let Some(s) = seed {
            rng = ChaCha12Rng::seed_from_u64(s);
        }

        SubaddressCache {
            highest_minor_index,
            available_subaddresses,
            viewpair,
            rng,
        }
    }

    pub fn remove_random(&mut self) -> (SubIndex, String) {
        let map_index = self.rng.gen_range(0..self.available_subaddresses.len());

        if let Some((sub_index, subaddress)) =
            self.available_subaddresses.shift_remove_index(map_index)
        {
            if self.len() <= MIN_AVAILABLE_SUBADDRESSES as usize {
                self.extend_by(MIN_AVAILABLE_SUBADDRESSES);
            }
            (sub_index, subaddress)
        } else {
            // Is this the best way to handle this error?
            error!("Failed to retrieve subaddress by index from subaddress cache; retrying");
            self.remove_random()
        }
    }

    pub fn insert(&mut self, sub_index: SubIndex, address: String) -> Option<String> {
        self.available_subaddresses.insert(sub_index, address)
    }

    pub fn len(&self) -> usize {
        self.available_subaddresses.len()
    }

    /// Generates `n` subaddresses at the end of the current range, and appends them to the
    /// subaddress cache.
    ///
    /// If adding `n` additional subaddresses would extend the cache beyond the maximum index of
    /// `(1, u32::MAX)`, generation stops prematurely.
    ///
    /// Returns the number of subaddresses appended to the subaddress cache.
    pub fn extend_by(&mut self, n: u32) -> u32 {
        // TODO: Change this to use generate_range().
        let mut count = 0;
        for _ in 0..n {
            if self.highest_minor_index.load(Ordering::Relaxed) == u32::MAX {
                // We're at the max, time to quit.
                return count;
            }
            let sub_index = SubIndex::new(1, self.highest_minor_index.load(Ordering::Relaxed) + 1);
            let subaddress = format!(
                "{}",
                subaddress::get_subaddress(&self.viewpair, sub_index.into(), None)
            );
            self.available_subaddresses.insert(sub_index, subaddress);
            self.highest_minor_index
                .store(sub_index.minor, Ordering::Relaxed);
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
