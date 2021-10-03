use std::collections::HashMap;
use std::convert::TryInto;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;

use log::{debug, error, trace, warn};
use tokio::join;

use crate::util::{get_txpool, scan_transactions};
use crate::{BlockCache, Payment, SubIndex, PaymentsDb};

pub struct Scanner {
    url: String,
    viewpair: monero::ViewPair,
    payment_rx: Receiver<Payment>,
    channel_tx: Sender<Receiver<Payment>>,
    payments: HashMap<SubIndex, Payment>,
    payments_db: PaymentsDb,
    channels: HashMap<SubIndex, Sender<Payment>>,
    // Block cache is mutexed to allow concurrent block & txpool scanning.
    // This is necessary even though txpool scanning doesn't use the block cache,
    // because rust doesn't allow mutably borrowing only part of "self".
    block_cache: Mutex<BlockCache>,
    first_scan: bool,
}

impl Scanner {
    pub fn new(
        url: String,
        viewpair: monero::ViewPair,
        payment_rx: Receiver<Payment>,
        channel_tx: Sender<Receiver<Payment>>,
        payments: HashMap<SubIndex, Payment>,
        payments_db: PaymentsDb,
        channels: HashMap<SubIndex, Sender<Payment>>,
        block_cache: BlockCache,
    ) -> Scanner {
        Scanner {
            url,
            viewpair,
            payment_rx,
            channel_tx,
            payments,
            payments_db,
            channels,
            block_cache: Mutex::new(block_cache),
            first_scan: true,
        }
    }

    pub fn track_new_payments(&mut self) {
        // Check for new payments to track.
        for payment in self.payment_rx.try_iter() {
            // Add the payment to the hashmap for tracking.
            self.payments.insert(payment.index, payment.clone());

            // Add payment to the db for tracking.
            self.payments_db
                .insert(&payment)
                .unwrap();

            // Set up communication for sending updates on this payment.
            let (update_tx, update_rx) = channel();
            self.channels.insert(payment.index, update_tx);
            self.channel_tx.send(update_rx).unwrap();

            debug!("Now tracking subaddress index {}", payment.index);
        }
    }

    /// Scan for payment updates and send them down their respective channels.
    pub async fn scan(&mut self) {
        // Update block cache, and scan both it and the txpool.
        let (updated_payments, txpool_amounts) =
            join!(self.scan_block_cache(), self.scan_txpool());
        
        if self.first_scan {
            self.first_scan = false;
        }

        // Add txpool amounts to block_cache updates.
        let mut updates = updated_payments;
        for (&payment_id, amount) in txpool_amounts.iter() {
            let payment = updates.get_mut(&payment_id).unwrap();
            payment.paid_amount += amount;

            if payment.paid_amount >= payment.expected_amount && payment.paid_at.is_none() {
                let height = self.block_cache.lock().unwrap().height;
                payment.paid_at = Some(height + 1);
            }
        }

        // Send updates, and remove completed transactions.
        self.send_updates(&mut updates);
    }

    /// Update block cache and scan the blocks.
    async fn scan_block_cache(&self) -> HashMap<SubIndex, Payment> {
        let mut block_cache = self.block_cache.lock().unwrap();
        
        // Update block cache.
        let mut blocks_updated = match block_cache.update(&self.url).await {
            Ok(num) => num,
            Err(e) => {
                error!("Faled to update block cache: {}", e);
                0
            }
        };

        // If this is the first scan, we want to scan all the blocks in the cache.
        if self.first_scan {
            blocks_updated = block_cache.blocks.len().try_into().unwrap();
        }

        // Set up temporary payments hashmap so we'll know which payments are updated.
        let mut updated_payments = self.payments.clone();
        let deepest_update = block_cache.height - blocks_updated + 1;
        // Remove partial payments occuring later then the deepest block cache update.
        for payment in updated_payments.values_mut() {
            let index = match payment.partial_payments.binary_search_by_key(&deepest_update, |&(key, _)| key) {
                Ok(num) => num,
                Err(num) => num,
            };
            payment.partial_payments.truncate(index);
        }

        // Scan updated blocks.
        for i in (0..blocks_updated.try_into().unwrap()).rev() {
            let transactions = &block_cache.blocks[i].3;
            let amounts_received =
                scan_transactions(&self.viewpair, &self.payments, transactions.to_vec());
            trace!(
                "Scanned {} transactions from block {}, and found {} tracked payments.",
                transactions.len(),
                block_cache.blocks[i].1,
                amounts_received.len()
            );

            let pos_in_cache: u64 = i.try_into().unwrap();
            let height = block_cache.height - pos_in_cache;

            // Update partial payment amounts.
            for (&subindex, amount) in amounts_received.iter() {
                let payment = updated_payments.get_mut(&subindex).unwrap();
                payment.partial_payments.push((height, *amount));
            }
        }

        // Recalculate total payment amounts.
        for (subaddress_index, payment) in updated_payments.iter_mut() {
            // No need recalculate unless something changed.
            if payment != self.payments.get(&subaddress_index).unwrap() {
                // Zero it out first.
                payment.paid_at = None;
                payment.paid_amount = 0;
                // Now add up the partial payments.
                for (height, amount) in &payment.partial_payments {
                    payment.paid_amount += amount;
                    if payment.paid_amount >= payment.expected_amount && payment.paid_at.is_none() {
                        payment.paid_at = Some(*height);
                    }
                }
            }
        }

        updated_payments
    }

    /// Retreive and scan transaction pool.
    async fn scan_txpool(&self) -> HashMap<SubIndex, u64> {
        // Retreive txpool.
        let txpool = match get_txpool(&self.url).await {
            Ok(pool) => pool,
            Err(e) => {
                error!("Faled to get transaction pool: {}", e);
                Vec::new()
            }
        };

        // Scan txpool.
        let amounts_received = scan_transactions(&self.viewpair, &self.payments, txpool.to_vec());
        trace!(
            "Scanned {} transactions from txpool, and found {} tracked payments.",
            txpool.len(),
            amounts_received.len()
        );
        amounts_received
    }

    /// Send updates down their respective channels, and remove completed/expired payments.
    fn send_updates(&mut self, updated_payments: &mut HashMap<SubIndex, Payment>) {
        // Send updates and mark completed/expired payments for removal.
        let mut completed = Vec::new();
        let block_cache = self.block_cache.lock().unwrap();
        for payment in updated_payments.values_mut() {
            // Update payment's current_block.
            if block_cache.height != payment.current_block {
                payment.current_block = block_cache.height;
            }
            // If payment was updated, send an update.
            if payment != self.payments.get(&payment.index).unwrap() {
                if self
                    .channels
                    .get(&payment.index)
                    .unwrap()
                    .send(payment.clone())
                    .is_err()
                {
                    warn!("Receiver disconnected before payment completed.");
                    completed.push(payment.index);
                }
                // Copy the updated payment parameters to the main one.
                self.payments.insert(payment.index, payment.clone());
            }
            // If the payment is fully confirmed (and at least a block old), mark it as complete.
            if payment.is_confirmed() && payment.starting_block < block_cache.height {
                completed.push(payment.index);
            }
            // If the payment has expired, mark it as complete.
            if payment.is_expired() {
                completed.push(payment.index);
            }
        }
        // Remove completed/expired transactions.
        for index in completed {
            if self.payments.remove(&index).is_none() {
                warn!("Attempted to remove subaddress index {} from tracked payments, but it didn't exist.", index);
            }
            if self.channels.remove(&index).is_none() {
                warn!("Attempted to remove subaddress index {} from payment update channels, but it didn't exist.", index);
            }
            debug!("No longer tracking subaddress index {}", index);
        }
    }
}
