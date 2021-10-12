use std::collections::HashMap;
use std::convert::TryInto;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use log::{error, info, trace};
use monero::cryptonote::hash::Hashable;
use tokio::join;

use crate::util;
use crate::{BlockCache, Payment, PaymentsDb, SubIndex, Transfer, TxpoolCache};

pub(crate) struct Scanner {
    url: String,
    viewpair: monero::ViewPair,
    payments_db: PaymentsDb,
    // Block cache and txpool cache are mutexed to allow concurrent block & txpool scanning. This is
    // necessary even though txpool scanning doesn't use the block cache, and vice versa, because
    // rust doesn't allow mutably borrowing only part of "self".
    block_cache: Mutex<BlockCache>,
    txpool_cache: Mutex<TxpoolCache>,
    first_scan: bool,
}

impl Scanner {
    pub async fn new(
        url: String,
        viewpair: monero::ViewPair,
        payments_db: PaymentsDb,
        block_cache_size: u64,
        atomic_height: Arc<AtomicU64>,
    ) -> Scanner {
        // Determine sensible initial height for block cache.
        let height = match payments_db.get_lowest_height() {
            Ok(Some(h)) => {
                info!("Pending payments found in AcceptXMR database. Resuming from last block scanned: {}", h);
                h
            }
            Ok(None) => {
                let h = Scanner::get_daemon_height(&url).await;
                info!("No pending payments found in AcceptXMR database. Skipping to blockchain tip: {}", h);
                h
            }
            Err(e) => {
                panic!("failed to determine suitable initial height for block cache from pending payments database: {}", e);
            }
        };

        // Set atomic height to the above determined initial height. This sets the height of the
        // main PaymentGateway as well.
        atomic_height.store(height, Ordering::Relaxed);

        // Initialize block cache and txpool cache.
        let (block_cache, txpool_cache) = join!(
            BlockCache::init(&url, block_cache_size, atomic_height),
            TxpoolCache::init(&url)
        );

        Scanner {
            url,
            viewpair,
            payments_db,
            block_cache: Mutex::new(block_cache.unwrap()),
            txpool_cache: Mutex::new(txpool_cache),
            first_scan: true,
        }
    }

    /// Scan for payment updates.
    pub async fn scan(&mut self) {
        // Update block cache, and scan both it and the txpool.
        let (blocks_amounts, txpool_amounts) = join!(self.scan_blocks(), self.scan_txpool());
        let height = self
            .block_cache
            .lock()
            // TODO: Handle this properly.
            .unwrap()
            .height
            .load(Ordering::Relaxed);

        // Combine transfers into one big vec.
        let mut transfers: Vec<(SubIndex, Transfer)> = blocks_amounts
            .into_iter()
            .chain(txpool_amounts.into_iter())
            .collect();

        if self.first_scan {
            self.first_scan = false;
        }

        let deepest_update = transfers
            .iter()
            .min_by(|(_, transfer_1), (_, transfer_2)| transfer_1.cmp_by_age(transfer_2))
            .map_or(height + 1, |(_, transfer)| {
                transfer.height.unwrap_or(height + 1)
            });

        // A place to keep track of what payments are changing, so we can log updates later.
        let mut updated_payments = Vec::new();

        // Prepare updated payments.
        // TODO: Break this out into its own function.
        for payment_or_err in self.payments_db.iter() {
            // Retrieve old payment object.
            let old_payment = match payment_or_err {
                Ok(p) => p,
                Err(e) => {
                    error!(
                        "Failed to retrieve old payment object from database while iterating through database: {}", e
                    );
                    continue;
                }
            };
            let mut payment = old_payment.clone();

            // Remove transfers occurring later than the deepest block update.
            payment
                .transfers
                .retain(|transfer| transfer.older_than(deepest_update));

            // Add transfers from blocks and txpool.
            for i in 0..transfers.len() {
                let (sub_index, owned_transfer) = transfers[i];
                if sub_index == payment.index && owned_transfer.newer_than(payment.started_at) {
                    transfers.remove(i);
                    payment.transfers.push(owned_transfer);
                }
            }

            // Update payment's current_block.
            if payment.current_height != height {
                payment.current_height = height;
            }

            // No need to recalculate total paid_amount or paid_at unless something changed.
            if payment != old_payment {
                // Zero it out first.
                payment.paid_at = None;
                payment.amount_paid = 0;
                // Now add up the transfers.
                for transfer in &payment.transfers {
                    payment.amount_paid += transfer.amount;
                    if payment.amount_paid >= payment.amount_requested && payment.paid_at.is_none()
                    {
                        payment.paid_at = transfer.height;
                    }
                }

                // This payment has been updated. We can now add it in with the other
                // updated_payments.
                updated_payments.push(payment);
            }
        }

        // log updates.
        self.log_updates(&updated_payments);

        // Save updates.
        for payment in &updated_payments {
            if let Err(e) = self.payments_db.update(&payment.index, payment) {
                error!(
                    "Failed to save update to payment for index {} to database: {}",
                    payment.index, e
                );
            }
        }

        // Flush changes to the database.
        self.payments_db.flush();
    }

    /// Update block cache and scan the blocks.
    ///
    /// Returns a vector of tuples of the form (subaddress index, amount, height)
    async fn scan_blocks(&self) -> Vec<(SubIndex, Transfer)> {
        let mut block_cache = self.block_cache.lock().unwrap();

        // Update block cache.
        let mut blocks_updated = match block_cache.update(&self.url).await {
            Ok(num) => num,
            Err(e) => {
                error!("Failed to update block cache: {}", e);
                0
            }
        };

        // If this is the first scan, we want to scan all the blocks in the cache.
        if self.first_scan {
            blocks_updated = block_cache.blocks.len().try_into().unwrap();
        }

        let mut transfers = Vec::new();

        // Scan updated blocks.
        for i in (0..blocks_updated.try_into().unwrap()).rev() {
            let transactions = &block_cache.blocks[i].3;
            let amounts_received = self.scan_transactions(transactions);
            trace!(
                "Scanned {} transactions from block {}, and found {} transfers for tracked payments",
                transactions.len(),
                block_cache.blocks[i].1,
                amounts_received.len()
            );

            let height: u64 = block_cache.height.load(Ordering::Relaxed) - i as u64;

            // Add what was found into the list.
            transfers.extend::<Vec<(SubIndex, Transfer)>>(
                amounts_received
                    .into_iter()
                    .map(|(_, amounts)| amounts)
                    .flatten()
                    .map(|amount| (amount.0, Transfer::new(amount.1, Some(height))))
                    .collect(),
            )
        }

        transfers
    }

    /// Retrieve and scan transaction pool.
    ///
    /// Returns a vector of tuples of the form (subaddress index, amount)
    async fn scan_txpool(&self) -> Vec<(SubIndex, Transfer)> {
        // Update txpool.
        let mut txpool_cache = self.txpool_cache.lock().unwrap();
        let new_transactions = txpool_cache.update(&self.url).await;

        // Transfers previously discovered the txpool (no reason to scan the same transactions
        // twice).
        let discovered_transfers = txpool_cache.discovered_transfers();

        // Scan txpool.
        let amounts_received = self.scan_transactions(&new_transactions);
        trace!(
            "Scanned {} transactions from txpool, and found {} transfers for tracked payments",
            new_transactions.len(),
            amounts_received.len()
        );

        let new_transfers: HashMap<monero::Hash, Vec<(SubIndex, Transfer)>> = amounts_received
            .iter()
            .map(|(hash, amounts)| {
                (
                    *hash,
                    amounts
                        .iter()
                        .map(|(sub_index, amount)| (*sub_index, Transfer::new(*amount, None)))
                        .collect(),
                )
            })
            .collect();

        let mut transfers: HashMap<monero::Hash, Vec<(SubIndex, Transfer)>> =
            new_transfers.to_owned();
        transfers.extend(discovered_transfers.to_owned());

        // Add the new transfers to the cache for next scan.
        txpool_cache.insert_transfers(&new_transfers);

        transfers
            .into_iter()
            .map(|(_, amounts)| amounts)
            .flatten()
            .collect()
    }

    fn scan_transactions(
        &self,
        transactions: &[monero::Transaction],
    ) -> HashMap<monero::Hash, Vec<(SubIndex, u64)>> {
        let mut amounts_received = HashMap::new();
        for tx in transactions {
            // Get transfers.
            let transfers = tx.check_outputs(&self.viewpair, 0..2, 0..2).unwrap();

            for transfer in &transfers {
                let sub_index = SubIndex::from(transfer.sub_index());

                // If this payment is being tracked, add the amount and payment ID to the result set.
                match self.payments_db.contains_key(&sub_index) {
                    Ok(true) => {
                        let amount = match transfers[0].amount() {
                            Some(a) => a,
                            None => {
                                error!("Failed to unblind transaction amount");
                                continue;
                            }
                        };
                        amounts_received
                            .entry(tx.hash())
                            .or_insert_with(Vec::new)
                            .push((sub_index, amount));
                    }
                    Ok(false) => continue,
                    Err(e) => {
                        error!("Failed to determine if database contains subaddress of discovered output: {}", e);
                    }
                }
            }
        }

        amounts_received.into_iter().collect()
    }

    /// Log updates
    fn log_updates(&self, updated_payments: &[Payment]) {
        for payment in updated_payments {
            trace!(
                "Payment update for subaddress index {}: \
                \n{}",
                payment.index,
                payment
            );
        }
    }

    /// TODO: Retry on failure instead of panic.
    async fn get_daemon_height(url: &str) -> u64 {
        util::get_daemon_height(url).await.unwrap()
    }
}
