use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
};

use log::{debug, error, info, trace};
use monero::{
    cryptonote::{hash::Hashable, onetime_key::SubKeyChecker},
    Amount, Transaction, VarInt,
};
use tokio::{join, sync::Mutex};

use crate::{
    caching::{BlockCache, TxpoolCache},
    invoice::Transfer,
    pubsub::Publisher,
    rpc::RpcClient,
    storage::{InvoiceStorage, Store},
    AcceptXmrError, SubIndex,
};

pub(crate) struct Scanner<S: InvoiceStorage> {
    invoice_store: Store<S>,
    // Block cache and txpool cache are mutexed to allow concurrent block & txpool scanning. This is
    // necessary even though txpool scanning doesn't use the block cache, and vice versa, because
    // rust doesn't allow mutably borrowing only part of "self".
    block_cache: Mutex<BlockCache>,
    txpool_cache: Mutex<TxpoolCache>,
    publisher: Arc<Publisher>,
    first_scan: bool,
}

impl<S: InvoiceStorage> Scanner<S> {
    pub async fn new(
        rpc_client: RpcClient,
        invoice_store: Store<S>,
        block_cache_size: usize,
        atomic_cache_height: Arc<AtomicU64>,
        atomic_daemon_height: Arc<AtomicU64>,
        publisher: Arc<Publisher>,
    ) -> Result<Scanner<S>, AcceptXmrError<S::Error>> {
        // Determine sensible initial height for block cache.
        let daemon_height = rpc_client.daemon_height().await?;
        let cache_height = match invoice_store.lowest_height() {
            Ok(Some(h)) => {
                info!("Pending invoices found in AcceptXMR database. Resuming from last block scanned: {}", h);
                h - 1
            }
            Ok(None) => {
                trace!("Retrieving daemon hight for scanner setup.");
                let h = daemon_height;
                info!("No pending invoices found in AcceptXMR database. Skipping to blockchain tip: {}", h);
                h - 1
            }
            Err(e) => return Err(AcceptXmrError::InvoiceStorage(e)),
        };

        // Set atomic height to the above determined initial height. This sets the height of the
        // main PaymentGateway as well.
        atomic_cache_height.store(cache_height, Ordering::Relaxed);
        atomic_daemon_height.store(daemon_height, Ordering::Relaxed);

        // Initialize block cache and txpool cache.
        let (block_cache, txpool_cache) = join!(
            BlockCache::init::<S>(
                rpc_client.clone(),
                block_cache_size,
                atomic_cache_height,
                atomic_daemon_height
            ),
            TxpoolCache::init::<S>(rpc_client.clone())
        );

        Ok(Scanner {
            invoice_store,
            block_cache: Mutex::new(block_cache?),
            txpool_cache: Mutex::new(txpool_cache?),
            publisher,
            first_scan: true,
        })
    }

    /// Scan for invoice updates.
    pub async fn scan(
        &mut self,
        sub_key_checker: &SubKeyChecker<'_>,
    ) -> Result<(), AcceptXmrError<S::Error>> {
        // Update block and txpool caches.
        let (blocks_updated, new_transactions) = self.update_caches().await?;

        // Scan block cache and new transactions in the txpool.
        let (blocks_amounts_or_err, txpool_amounts_or_err) = join!(
            self.scan_blocks(sub_key_checker, blocks_updated),
            self.scan_txpool(sub_key_checker, &new_transactions)
        );
        let block_cache_height = self.block_cache.lock().await.height.load(Ordering::Relaxed);

        let blocks_amounts = match blocks_amounts_or_err {
            Ok(amts) => amts,
            Err(e) => {
                error!("Skipping scan! Encountered a problem while updating or scanning the block cache: {}", e);
                return Err(e);
            }
        };
        let txpool_amounts = match txpool_amounts_or_err {
            Ok(amts) => amts,
            Err(e) => {
                error!("Skipping scan! Encountered a problem while updating or scanning the txpool cache: {}", e);
                return Err(e);
            }
        };

        // Combine transfers into one big vec.
        let transfers: Vec<(SubIndex, Transfer)> = blocks_amounts
            .into_iter()
            .chain(txpool_amounts.into_iter())
            .collect();

        if self.first_scan {
            self.first_scan = false;
        }

        // Prepare updated invoices.
        // TODO: Break this out into its own function.
        let deepest_update = block_cache_height - blocks_updated as u64 + 1;
        let mut updated_invoices = Vec::new();
        for invoice_or_err in self
            .invoice_store
            .lock()
            .try_iter()
            .map_err(AcceptXmrError::InvoiceStorage)?
        {
            // Retrieve old invoice object.
            let old_invoice = match invoice_or_err {
                Ok(p) => p,
                Err(e) => {
                    error!(
                        "Failed to retrieve old invoice object from database while iterating through database: {}", e
                    );
                    continue;
                }
            };
            let mut invoice = old_invoice.clone();

            // Remove transfers occurring in or after the deepest block update.
            invoice.transfers.retain(|transfer| {
                transfer
                    .cmp_by_height(&Transfer::new(0, Some(deepest_update)))
                    .is_lt()
            });

            // Add transfers from blocks and txpool.
            for (sub_index, owned_transfer) in &transfers {
                if sub_index == &invoice.index()
                    && owned_transfer
                        // Creation height - 1 because creation height is one greater than top block
                        // height.
                        .cmp_by_height(&Transfer::new(0, Some(invoice.creation_height() - 1)))
                        .is_gt()
                {
                    invoice.transfers.push(*owned_transfer);
                }
            }

            // Update invoice's current_block.
            if invoice.current_height != block_cache_height + 1 {
                invoice.current_height = block_cache_height + 1;
            }

            // No need to recalculate total paid_amount or paid_at unless something changed.
            if invoice != old_invoice {
                // Zero it out first.
                invoice.paid_height = None;
                invoice.amount_paid = 0;
                // Now add up the transfers.
                for transfer in &invoice.transfers {
                    invoice.amount_paid += transfer.amount;
                    if invoice.amount_paid >= invoice.amount_requested()
                        && invoice.paid_height.is_none()
                    {
                        invoice.paid_height = transfer.height;
                    }
                }

                // This invoice has been updated. We can now add it in with the other
                // updated_invoices.
                updated_invoices.push(invoice);
            }
        }

        // Save and log updates.
        for invoice in updated_invoices {
            debug!(
                "Invoice update for subaddress index {}: \
                    \n{}",
                invoice.index(),
                invoice
            );
            if let Err(e) = self.invoice_store.update(invoice.clone()) {
                error!(
                    "Failed to save update to invoice for index {} to database: {}",
                    invoice.index(),
                    e
                );
            } else {
                // If the update was successful, send an update that down the subscriber channel.
                self.publisher.send_updates(&invoice).await;
            }
        }

        // Flush changes to the database.
        self.invoice_store
            .flush()
            .map_err(AcceptXmrError::InvoiceStorage)?;
        Ok(())
    }

    async fn update_caches(&self) -> Result<(usize, Vec<Transaction>), AcceptXmrError<S::Error>> {
        // Update block cache.
        let mut block_cache = self.block_cache.lock().await;
        let blocks_updated = if self
            .invoice_store
            .is_empty()
            .map_err(AcceptXmrError::InvoiceStorage)?
        {
            // Skip ahead to blockchain tip if there are no pending invoices.
            block_cache.skip_ahead::<S>().await?
        } else {
            block_cache.update::<S>().await?
        };

        // Update txpool.
        let mut txpool_cache = self.txpool_cache.lock().await;
        let new_transactions = txpool_cache.update::<S>().await?;

        Ok((blocks_updated, new_transactions))
    }

    /// Scan the block cache up to `updated_blocks` deep.
    ///
    /// Returns a vector of tuples containing [`Transfer`]s and their associated subaddress indices.
    async fn scan_blocks(
        &self,
        sub_key_checker: &SubKeyChecker<'_>,
        mut blocks_updated: usize,
    ) -> Result<Vec<(SubIndex, Transfer)>, AcceptXmrError<S::Error>> {
        let block_cache = self.block_cache.lock().await;

        // If this is the first scan, we want to scan all the blocks in the cache.
        if self.first_scan {
            blocks_updated = block_cache.blocks.len();
        }

        let mut transfers = Vec::new();

        // Scan updated blocks.
        for i in (0..blocks_updated).rev() {
            let transactions = &block_cache.blocks[i].3;
            let amounts_received = self.scan_transactions(transactions, sub_key_checker)?;
            trace!(
                "Scanned {} transactions from block {}, and found {} transactions to tracked invoices",
                transactions.len(),
                block_cache.blocks[i].1,
                amounts_received.len()
            );

            let block_cache_height: u64 = block_cache.height.load(Ordering::Relaxed) - i as u64;

            // Add what was found into the list.
            transfers.extend::<Vec<(SubIndex, Transfer)>>(
                amounts_received
                    .into_iter()
                    .flat_map(|(_, amounts)| amounts)
                    .map(|OwnedAmount { sub_index, amount }| {
                        (
                            sub_index,
                            Transfer::new(amount.as_pico(), Some(block_cache_height)),
                        )
                    })
                    .collect(),
            );
        }

        Ok(transfers)
    }

    /// Retrieve and scan transaction pool.
    ///
    /// Returns a vector of tuples of the form (subaddress index, amount)
    async fn scan_txpool(
        &self,
        sub_key_checker: &SubKeyChecker<'_>,
        new_transactions: &[Transaction],
    ) -> Result<Vec<(SubIndex, Transfer)>, AcceptXmrError<S::Error>> {
        let mut txpool_cache = self.txpool_cache.lock().await;

        // Transfers previously discovered in the txpool (no reason to scan the same transactions
        // twice).
        let discovered_transfers = txpool_cache.discovered_transfers();

        // Scan txpool.
        let amounts_received = self.scan_transactions(new_transactions, sub_key_checker)?;
        trace!(
            "Scanned {} transactions from txpool, and found {} transfers for tracked invoices",
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
                        .map(|OwnedAmount { sub_index, amount }| {
                            (*sub_index, Transfer::new(amount.as_pico(), None))
                        })
                        .collect(),
                )
            })
            .collect();

        let mut transfers: HashMap<monero::Hash, Vec<(SubIndex, Transfer)>> = new_transfers.clone();
        // CLoning here because discovered_transactions is owned by the txpool cache.
        transfers.extend(discovered_transfers.clone());

        // Add the new transfers to the cache for next scan.
        txpool_cache.insert_transfers(&new_transfers);

        Ok(transfers
            .into_iter()
            .flat_map(|(_, amounts)| amounts)
            .collect())
    }

    fn scan_transactions(
        &self,
        transactions: &[monero::Transaction],
        sub_key_checker: &SubKeyChecker<'_>,
    ) -> Result<HashMap<monero::Hash, Vec<OwnedAmount>>, AcceptXmrError<S::Error>> {
        let mut amounts_received = HashMap::new();
        for tx in transactions {
            // Ensure the time lock is zero.
            if tx.prefix().unlock_time != VarInt(0) {
                continue;
            }

            // Scan transaction for owned outputs.
            let transfers = tx.check_outputs_with(sub_key_checker)?;

            for transfer in &transfers {
                let sub_index = SubIndex::from(transfer.sub_index());

                // If this invoice is being tracked, add the amount and subindex to the result set.
                if self
                    .invoice_store
                    .contains_sub_index(sub_index)
                    .map_err(AcceptXmrError::InvoiceStorage)?
                {
                    let amount = OwnedAmount {
                        sub_index,
                        amount: transfers[0]
                            .amount()
                            .ok_or(AcceptXmrError::<S::Error>::Unblind(sub_index))?,
                    };
                    amounts_received
                        .entry(tx.hash())
                        .or_insert_with(Vec::new)
                        .push(amount);
                }
            }
        }

        Ok(amounts_received.into_iter().collect())
    }
}

pub(crate) struct ScannerHandle<S: InvoiceStorage>(
    JoinHandle<Result<(), AcceptXmrError<S::Error>>>,
);

impl<S: InvoiceStorage> ScannerHandle<S> {
    pub fn join(self) -> thread::Result<Result<(), AcceptXmrError<S::Error>>> {
        self.0.join()
    }

    pub fn is_finished(&self) -> bool {
        self.0.is_finished()
    }
}

impl<S: InvoiceStorage> From<JoinHandle<Result<(), AcceptXmrError<S::Error>>>>
    for ScannerHandle<S>
{
    fn from(inner: JoinHandle<Result<(), AcceptXmrError<S::Error>>>) -> Self {
        ScannerHandle(inner)
    }
}

struct OwnedAmount {
    sub_index: SubIndex,
    amount: Amount,
}
