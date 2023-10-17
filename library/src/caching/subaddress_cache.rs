use std::{
    cmp,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc, PoisonError, RwLock,
    },
};

use indexmap::{IndexMap, IndexSet};
use log::{debug, error, warn};
use monero::{cryptonote::subaddress, ViewPair};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha12Rng;

use crate::{storage::InvoiceStorage, SubIndex};

const MIN_AVAILABLE_SUBADDRESSES: u32 = 100;

pub(crate) struct SubaddressCache {
    major_index: u32,
    highest_minor_index: Arc<AtomicU32>,
    available_subaddresses: IndexMap<SubIndex, String>,
    viewpair: ViewPair,
    rng: ChaCha12Rng,
}

impl SubaddressCache {
    pub(crate) fn init<S: InvoiceStorage>(
        storage: &Arc<RwLock<S>>,
        viewpair: monero::ViewPair,
        major_index: u32,
        highest_minor_index: Arc<AtomicU32>,
        seed: Option<u64>,
    ) -> Result<SubaddressCache, S::Error> {
        // Get currently used subindexes from database, so they won't be put in the list
        // of available subindexes.
        let used_sub_indexes = storage
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .try_iter()?
            .map(|invoice_or_err| match invoice_or_err {
                Ok(invoice) => Ok(invoice.index()),
                Err(e) => Err(e),
            })
            .collect::<Result<IndexSet<SubIndex>, S::Error>>()?;

        // Get highest index from list of used subindexes.
        let max_used = if let Some(max_sub_index) = used_sub_indexes.iter().max() {
            debug!(
                "Highest subaddress index in the database: {}",
                SubIndex::new(major_index, max_sub_index.minor)
            );
            max_sub_index.minor
        } else {
            debug!("Highest subaddress index in the database: N/A");
            0
        };

        // Generate enough subaddresses to cover all pending invoices.
        highest_minor_index.store(
            cmp::max(MIN_AVAILABLE_SUBADDRESSES - 1, max_used),
            Ordering::Relaxed,
        );
        let mut available_subaddresses: IndexMap<SubIndex, String> = generate_range(
            SubIndex::new(major_index, 0),
            SubIndex::new(major_index, highest_minor_index.load(Ordering::Relaxed)),
            &viewpair,
        )
        .into_iter()
        .collect();

        // Remove subaddresses that are present in the database.
        available_subaddresses.retain(|sub_index, _| !used_sub_indexes.contains(sub_index));

        // If a seed is supplied, seed the random number generator with it.
        let mut rng = ChaCha12Rng::from_entropy();
        if let Some(s) = seed {
            rng = ChaCha12Rng::seed_from_u64(s);
        }

        Ok(SubaddressCache {
            major_index,
            highest_minor_index,
            available_subaddresses,
            viewpair,
            rng,
        })
    }

    pub(crate) fn remove_random(&mut self) -> (SubIndex, String) {
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

    pub(crate) fn insert(&mut self, sub_index: SubIndex, address: String) -> Option<String> {
        self.available_subaddresses.insert(sub_index, address)
    }

    pub(crate) fn len(&self) -> usize {
        self.available_subaddresses.len()
    }

    /// Generates `n` subaddresses at the end of the current range, and appends
    /// them to the subaddress cache.
    ///
    /// If adding `n` additional subaddresses would extend the cache beyond the
    /// maximum index of `(1, u32::MAX)`, generation stops prematurely.
    ///
    /// Returns the number of subaddresses appended to the subaddress cache.
    fn extend_by(&mut self, n: u32) -> u32 {
        // TODO: Change this to use generate_range().
        let mut count = 0;
        for _ in 0..n {
            if self.highest_minor_index.load(Ordering::Relaxed) == u32::MAX {
                // We're at the max, time to quit.
                return count;
            }
            let sub_index = SubIndex::new(
                self.major_index,
                self.highest_minor_index.load(Ordering::Relaxed) + 1,
            );
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
}

/// Generates range of subaddresses between the two `SubIndex`s provided,
/// inclusive.
///
/// Will stop generation if the minor index saturates (i.e. it will not generate
/// subaddresses across more than one major index)
fn generate_range(
    from: SubIndex,
    to: SubIndex,
    viewpair: &monero::ViewPair,
) -> Vec<(SubIndex, String)> {
    let mut subaddresses = Vec::new();
    if to < from {
        return subaddresses;
    }

    let mut current = from;
    while current <= to {
        let subaddress = format!(
            "{}",
            subaddress::get_subaddress(viewpair, current.into(), None)
        );
        subaddresses.push((current, subaddress));

        current.minor = if let Some(minor) = current.minor.checked_add(1) {
            minor
        } else {
            warn!("Cannot generate additional subaddresses. Minor index is saturated");
            break;
        }
    }

    subaddresses
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use std::{cmp::Ordering, str::FromStr};

    use test_case::test_case;

    use super::generate_range;
    use crate::SubIndex;

    #[test_case(SubIndex::new(0, 0), SubIndex::new(0, 100))]
    #[test_case(SubIndex::new(0, 0), SubIndex::new(0, 0))]
    #[test_case(SubIndex::new(1, 0), SubIndex::new(1, 0))]
    #[test_case(SubIndex::new(1, 0), SubIndex::new(1, 100))]
    #[test_case(SubIndex::new(1, 100), SubIndex::new(1, 0))]
    #[test_case(SubIndex::new(0, u32::MAX - 100), SubIndex::new(1, 0))]
    #[test_case(SubIndex::new(0, u32::MAX - 100), SubIndex::new(0, u32::MAX))]
    fn test_generated_range(from: SubIndex, to: SubIndex) {
        let private_view_key = "ad2093a5705b9f33e6f0f0c1bc1f5f639c756cdfc168c8f2ac6127ccbdab3a03";
        let primary_address = "4613YiHLM6JMH4zejMB2zJY5TwQCxL8p65ufw8kBP5yxX9itmuGLqp1dS4tkVoTxjyH3aYhYNrtGHbQzJQP5bFus3KHVdmf";
        let viewpair = monero::ViewPair {
            view: monero::PrivateKey::from_str(private_view_key).unwrap(),
            spend: monero::Address::from_str(primary_address)
                .unwrap()
                .public_spend,
        };

        let subaddresses = generate_range(from, to, &viewpair);
        let expected_num_generated = match from.major.cmp(&to.major) {
            Ordering::Equal => to
                .minor
                .checked_sub(from.minor)
                .map(|n| n + 1)
                .unwrap_or_default(),
            Ordering::Less => u32::MAX - from.minor + 1,
            Ordering::Greater => 0,
        };
        assert_eq!(
            subaddresses.len(),
            usize::try_from(expected_num_generated).unwrap()
        );

        let sub_indices: Vec<SubIndex> = subaddresses.into_iter().map(|(sub, _)| sub).collect();
        assert_eq!(
            sub_indices.iter().min(),
            if from <= to { Some(&from) } else { None }
        );

        let max_generated = sub_indices.into_iter().max().map(|mut sub_index| {
            sub_index.major = from.major;
            sub_index
        });
        let expected_max_generated = match (from <= to, from.major.cmp(&to.major)) {
            (true, Ordering::Equal) => Some(to),
            (_, Ordering::Greater) | (false, _) => None,
            (true, Ordering::Less) => Some(SubIndex::new(from.major, u32::MAX)),
        };
        assert_eq!(max_generated, expected_max_generated);
    }
}
