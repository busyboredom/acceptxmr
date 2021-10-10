use std::collections::HashMap;
use std::convert::TryInto;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use log::{error, info, trace};
use tokio::join;

use crate::util;
use crate::{BlockCache, OwnedOutput, Payment, PaymentsDb, SubIndex};

pub(crate) struct Scanner {
    url: String,
    viewpair: monero::ViewPair,
    payments_db: PaymentsDb,
    // Block cache is mutexed to allow concurrent block & txpool scanning.
    // This is necessary even though txpool scanning doesn't use the block cache,
    // because rust doesn't allow mutably borrowing only part of "self".
    block_cache: Mutex<BlockCache>,
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

        // Initialize block cache.
        let block_cache = BlockCache::init(&url, block_cache_size, atomic_height)
            .await
            .unwrap();

        Scanner {
            url,
            viewpair,
            payments_db,
            block_cache: Mutex::new(block_cache),
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

        // Combine owned outputs into one big vec.
        let mut owned_outputs: Vec<(SubIndex, OwnedOutput)> = blocks_amounts
            .into_iter()
            .chain(txpool_amounts.into_iter())
            .collect();

        if self.first_scan {
            self.first_scan = false;
        }

        let deepest_update = owned_outputs
            .iter()
            .min_by(|(_, output_1), (_, output_2)| output_1.cmp_by_age(output_2))
            .map_or(height + 1, |(_, output)| {
                output.height.unwrap_or(height + 1)
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

            // Remove owned outputs occurring later than the deepest block update.
            payment
                .owned_outputs
                .retain(|output| output.older_than(deepest_update));

            // Add owned outputs from blocks and txpool.
            for i in 0..owned_outputs.len() {
                let (sub_index, owned_output) = owned_outputs[i];
                if sub_index == payment.index && owned_output.newer_than(payment.started_at) {
                    owned_outputs.remove(i);
                    payment.owned_outputs.push(owned_output);
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
                // Now add up the owned outputs.
                for owned_output in &payment.owned_outputs {
                    payment.amount_paid += owned_output.amount;
                    if payment.amount_paid >= payment.amount_requested && payment.paid_at.is_none()
                    {
                        payment.paid_at = owned_output.height;
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
    async fn scan_blocks(&self) -> Vec<(SubIndex, OwnedOutput)> {
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

        let mut owned_outputs = Vec::new();

        // Scan updated blocks.
        for i in (0..blocks_updated.try_into().unwrap()).rev() {
            let transactions = &block_cache.blocks[i].3;
            let amounts_received = self.scan_transactions(transactions.to_vec());
            trace!(
                "Scanned {} transactions from block {}, and found {} tracked payments",
                transactions.len(),
                block_cache.blocks[i].1,
                amounts_received.len()
            );

            let height: u64 = block_cache.height.load(Ordering::Relaxed) - i as u64;

            // Add what was found into the list.
            owned_outputs.extend::<Vec<(SubIndex, OwnedOutput)>>(
                amounts_received
                    .into_iter()
                    .map(|(sub_index, amount)| (sub_index, OwnedOutput::new(amount, Some(height))))
                    .collect(),
            )
        }

        owned_outputs
    }

    /// Retrieve and scan transaction pool.
    ///
    /// Returns a vector of tuples of the form (subaddress index, amount)
    async fn scan_txpool(&self) -> Vec<(SubIndex, OwnedOutput)> {
        // Retrieve txpool.
        // TODO: Retrieve hashes, and then only retrieve transactions we don't already have.
        let txpool = match util::get_txpool(&self.url).await {
            Ok(pool) => pool,
            Err(e) => {
                error!("Failed to get transaction pool: {}", e);
                return Vec::new();
            }
        };

        // Scan txpool.
        let amounts_received = self.scan_transactions(txpool.to_vec());
        trace!(
            "Scanned {} transactions from txpool, and found {} tracked payments",
            txpool.len(),
            amounts_received.len()
        );

        amounts_received
            .iter()
            .map(|(sub_index, amount)| (*sub_index, OwnedOutput::new(*amount, None)))
            .collect()
    }

    fn scan_transactions(&self, transactions: Vec<monero::Transaction>) -> Vec<(SubIndex, u64)> {
        let mut amounts_received = HashMap::new();
        for tx in transactions {
            // Get owned outputs.
            let owned_outputs = tx.check_outputs(&self.viewpair, 0..2, 0..2).unwrap();

            for output in &owned_outputs {
                let sub_index = SubIndex::from(output.sub_index());

                // If this payment is being tracked, add the amount and payment ID to the result set.
                match self.payments_db.contains_key(&sub_index) {
                    Ok(true) => {
                        let amount = match owned_outputs[0].amount() {
                            Some(a) => a,
                            None => {
                                error!("Failed to unblind transaction amount");
                                continue;
                            }
                        };
                        *amounts_received.entry(sub_index).or_insert(0) += amount;
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
            let confirmations = match payment.confirmations() {
                Some(height) => height.to_string(),
                None => "N/A".to_string(),
            };
            trace!(
                "Payment update for subaddress index {}: \
                \nPaid: {}/{} \
                \nConfirmations: {} \
                \nStarted at: {} \
                \nCurrent height: {} \
                \nExpiration at: {} \
                \nOwned outputs: \
                \n{:#?}",
                payment.index,
                monero::Amount::from_pico(payment.amount_paid).as_xmr(),
                monero::Amount::from_pico(payment.amount_requested).as_xmr(),
                confirmations,
                payment.started_at,
                payment.current_height,
                payment.expiration_at,
                payment.owned_outputs,
            );
        }
    }

    /// TODO: Retry on failure instead of panic.
    async fn get_daemon_height(url: &str) -> u64 {
        util::get_daemon_height(url).await.unwrap()
    }
}
