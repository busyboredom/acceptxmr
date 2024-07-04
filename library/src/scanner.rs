use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, PoisonError,
    },
};

use log::{debug, error, info, trace};
use monero::{
    blockdata::transaction::TxOutTarget,
    cryptonote::{hash::Hashable, onetime_key::SubKeyChecker},
    Amount, OwnedTxOut, Transaction, VarInt,
};
use rayon::prelude::*;
use thiserror::Error;
use tokio::{
    join,
    sync::Mutex as AsyncMutex,
    task::{JoinError, JoinHandle},
};

use crate::{
    caching::{BlockCache, BlockCacheError, TxpoolCache, TxpoolCacheError},
    invoice::Transfer,
    monerod_client::{
        Client as MonerodClient, RpcClient as MonerodRpcClient, RpcError as MonerodRpcError,
    },
    pubsub::Publisher,
    storage::{Client as StorageClient, OutputId, OutputPubKey, Storage, StorageError},
    Invoice, SubIndex,
};

pub(crate) struct Scanner<S: Storage, M: MonerodClient = MonerodRpcClient> {
    store: StorageClient<S>,
    // Block cache and txpool cache are mutexed to allow concurrent block &
    // txpool scanning. This is necessary even though txpool scanning doesn't
    // use the block cache, and vice versa, because rust doesn't allow mutably
    // borrowing only part of "self".
    block_cache: AsyncMutex<BlockCache<M>>,
    txpool_cache: AsyncMutex<TxpoolCache<M>>,
    publisher: Arc<Publisher>,
    first_scan: bool,
}

impl<S: Storage + 'static, M: MonerodClient> Scanner<S, M> {
    pub(crate) async fn new(
        monerod_client: M,
        store: StorageClient<S>,
        block_cache_size: usize,
        atomic_cache_height: Arc<AtomicU64>,
        atomic_daemon_height: Arc<AtomicU64>,
        // Optionally specify the height to start scanning from.
        initial_height: Option<u64>,
        publisher: Arc<Publisher>,
    ) -> Result<Scanner<S, M>, ScannerError> {
        trace!("Retrieving daemon height for scanner setup.");

        let daemon_height = monerod_client.daemon_height().await?;

        let cache_height = last_height(&store)
            .await?
            .or(initial_height)
            .unwrap_or(daemon_height)
            .min(daemon_height)
            .max(block_cache_size as u64)
            - 1;

        // Set atomic height to the above determined initial height. This sets the
        // height of the main PaymentGateway as well.
        atomic_cache_height.store(cache_height, Ordering::Relaxed);
        atomic_daemon_height.store(daemon_height, Ordering::Relaxed);

        // Initialize block cache and txpool cache.
        let (block_cache, txpool_cache) = join!(
            BlockCache::init(
                monerod_client.clone(),
                block_cache_size,
                atomic_cache_height,
                atomic_daemon_height
            ),
            TxpoolCache::init(monerod_client.clone())
        );

        // Initialize the publisher with all currently-tracked invoices.
        store
            .get_invoice_ids()
            .await?
            .iter()
            .for_each(|&id| publisher.insert_invoice(id));

        Ok(Scanner {
            store,
            block_cache: AsyncMutex::new(block_cache?),
            txpool_cache: AsyncMutex::new(txpool_cache?),
            publisher,
            first_scan: true,
        })
    }

    /// Scan for invoice updates.
    pub(crate) async fn scan(
        &mut self,
        sub_key_checker: &SubKeyChecker<'_>,
    ) -> Result<(), ScannerError> {
        // Update block and txpool caches.
        let (blocks_updated, new_transactions) = self.update_caches().await?;

        // Scan block cache and new transactions in the txpool.
        let (blocks_amounts_or_err, txpool_amounts_or_err) = join!(
            self.scan_blocks(sub_key_checker, blocks_updated),
            self.scan_txpool(sub_key_checker, &new_transactions)
        );

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
        let transfers: Vec<(SubIndex, Transfer)> =
            blocks_amounts.into_iter().chain(txpool_amounts).collect();

        if self.first_scan {
            self.first_scan = false;
        }

        let updated_invoices = self.update_invoices(transfers, blocks_updated).await?;

        // Save and log updates.
        for invoice in updated_invoices {
            debug!(
                "Invoice update for subaddress index {}: \
                    \n{}",
                invoice.index(),
                invoice
            );
            let result = self.store.update_invoice(invoice.clone()).await;
            if let Err(e) = result {
                error!(
                    "Failed to save update to invoice for index {} to database: {}",
                    invoice.index(),
                    e
                );
            } else {
                // If the update was successful, send an update down the
                // subscriber channel.
                self.publisher.send_updates(&invoice).await;
                debug!(
                    "Published invoice update for subaddress index {}",
                    invoice.index()
                );
            }
        }

        // Update last scanned height in the database.
        let cache_height = self.block_cache.lock().await.height();
        self.store.upsert_height(cache_height).await?;

        // Flush changes to the database.
        self.store.flush().await?;

        Ok(())
    }

    async fn update_invoices(
        &self,
        transfers: Vec<(SubIndex, Transfer)>,
        blocks_updated: usize,
    ) -> Result<Vec<Invoice>, ScannerError> {
        let block_cache_height = self.block_cache.lock().await.height();
        let deepest_update = block_cache_height - blocks_updated as u64 + 1;

        let updated_invoices = Arc::new(Mutex::new(Vec::new()));
        let cloned_invoices = updated_invoices.clone();
        // TODO: Find a way to parallelize this.
        self.store.try_for_each_invoice(move |invoice_or_err| {
            // Retrieve old invoice object.
            let old_invoice = match invoice_or_err {
                Ok(p) => p,
                Err(e) => {
                    error!(
                        "Failed to retrieve old invoice object from database while iterating through database: {}", e
                    );
                    // Return OK here because we still want to process the others.
                    return Ok(());
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
                cloned_invoices.lock().unwrap_or_else(PoisonError::into_inner).push(invoice);
            }

            Ok(())
        }).await?;
        let updated_invoices = updated_invoices
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .to_vec();
        Ok(updated_invoices)
    }

    async fn update_caches(&self) -> Result<(usize, Vec<Transaction>), ScannerError> {
        // Update block cache.
        let mut block_cache = self.block_cache.lock().await;
        let blocks_updated = block_cache.update().await?;

        // Update txpool.
        let mut txpool_cache = self.txpool_cache.lock().await;
        let new_transactions = txpool_cache.update().await?;

        Ok((blocks_updated, new_transactions))
    }

    /// Scan the block cache up to `updated_blocks` deep.
    ///
    /// Returns a vector of tuples containing [`Transfer`]s and their associated
    /// subaddress indices.
    async fn scan_blocks(
        &self,
        sub_key_checker: &SubKeyChecker<'_>,
        mut blocks_updated: usize,
    ) -> Result<Vec<(SubIndex, Transfer)>, ScannerError> {
        let block_cache = self.block_cache.lock().await;

        // If this is the first scan, we want to scan all the blocks in the cache.
        if self.first_scan {
            blocks_updated = block_cache.blocks().len();
        }

        let mut transfers = Vec::new();

        // Scan updated blocks.
        for i in (0..blocks_updated).rev() {
            let transactions = &block_cache.blocks()[i].transactions;
            let amounts_received = self
                .scan_transactions(transactions, sub_key_checker)
                .await?;
            trace!(
                "Scanned {} transactions from block {}, and found {} transactions to tracked invoices",
                transactions.len(),
                block_cache.blocks()[i].height,
                amounts_received.len(),
            );

            let block_cache_height: u64 = block_cache.height() - i as u64;

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
    ) -> Result<Vec<(SubIndex, Transfer)>, ScannerError> {
        let mut txpool_cache = self.txpool_cache.lock().await;

        // Transfers previously discovered in the txpool (no reason to scan the same
        // transactions twice).
        let discovered_transfers = txpool_cache.discovered_transfers();

        // Scan txpool.
        let amounts_received = self
            .scan_transactions(new_transactions, sub_key_checker)
            .await?;
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

    async fn scan_transactions(
        &self,
        transactions: &[monero::Transaction],
        sub_key_checker: &SubKeyChecker<'_>,
    ) -> Result<HashMap<monero::Hash, Vec<OwnedAmount>>, ScannerError> {
        let mut amounts_received = HashMap::new();

        let owned_outputs_per_tx: Vec<(monero::Hash, Vec<OwnedTxOut<'_>>)> = transactions
            .par_iter()
            .filter(|tx| {
                // Ensure the time lock is zero.
                if tx.prefix().unlock_time == VarInt(0) {
                    true
                } else {
                    debug!("Saw time locked transaction with hash {}", tx.hash());
                    false
                }
            })
            .try_fold(Vec::new, |mut outputs_per_tx, tx| {
                let outputs = tx.check_outputs_with(sub_key_checker)?;
                outputs_per_tx.push((tx.hash(), outputs));
                Ok::<Vec<(monero::Hash, Vec<OwnedTxOut<'_>>)>, ScannerError>(outputs_per_tx)
            })
            .try_reduce(Vec::new, |mut outputs, mut other_outputs| {
                outputs.append(&mut other_outputs);
                Ok(outputs)
            })?;

        for (tx_hash, owned_outputs) in owned_outputs_per_tx {
            for output in &owned_outputs {
                if !self.output_key_is_unique(output, tx_hash).await? {
                    debug!(
                        "Owned output #{} in transaction {} has duplicate public key.",
                        output.index(),
                        tx_hash
                    );
                    continue;
                }

                let sub_index = SubIndex::from(output.sub_index());

                // If this invoice is being tracked, add the amount and subindex to the result
                // set.
                if self.store.contains_sub_index(sub_index).await? {
                    let amount = OwnedAmount {
                        sub_index,
                        amount: output.amount().ok_or(ScannerError::Unblind(sub_index))?,
                    };
                    amounts_received
                        .entry(tx_hash)
                        .or_insert_with(Vec::new)
                        .push(amount);
                }
            }
        }

        Ok(amounts_received.into_iter().collect())
    }

    /// Returns `true` if the output key is unique to this output, or false if
    /// the key has been used by a previous output (indicating an instance of
    /// the burning bug).
    async fn output_key_is_unique(
        &self,
        output: &OwnedTxOut<'_>,
        tx_hash: monero::Hash,
    ) -> Result<bool, ScannerError> {
        let key = match output.out().target {
            TxOutTarget::ToKey { key } | TxOutTarget::ToTaggedKey { key, view_tag: _ } => key,
        };
        let output_id = OutputId {
            tx_hash: tx_hash.to_bytes(),
            index: u8::try_from(output.index()).map_err(|_| ScannerError::OutputIndex)?,
        };
        let maybe_stored_output_id = self.store.get_output_key_id(OutputPubKey(key)).await?;
        if let Some(stored_output_id) = maybe_stored_output_id {
            if stored_output_id != output_id {
                return Ok(false);
            }
        } else {
            self.store
                .insert_output_key(OutputPubKey(key), output_id)
                .await?;
        }
        Ok(true)
    }

    pub(crate) async fn is_synchronized(&self) -> bool {
        self.block_cache.lock().await.is_synchronized()
    }

    pub(crate) async fn cache_height(&self) -> u64 {
        self.block_cache.lock().await.height()
    }

    pub(crate) async fn daemon_height(&self) -> u64 {
        self.block_cache.lock().await.daemon_height()
    }
}

pub(crate) struct ScannerHandle(JoinHandle<Result<(), ScannerError>>);

impl ScannerHandle {
    pub(crate) async fn join(self) -> Result<(), ScannerError> {
        self.0.await?
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.0.is_finished()
    }
}

impl From<JoinHandle<Result<(), ScannerError>>> for ScannerHandle {
    fn from(inner: JoinHandle<Result<(), ScannerError>>) -> Self {
        ScannerHandle(inner)
    }
}

struct OwnedAmount {
    sub_index: SubIndex,
    amount: Amount,
}

async fn last_height<S: Storage + 'static>(
    store: &StorageClient<S>,
) -> Result<Option<u64>, ScannerError> {
    if let Some(h) = store.get_height().await? {
        info!("Last block scanned: {}", h);
        return Ok(Some(h));
    }

    // Fall back on this for compatibility in case the user's height table
    // wasn't populated yet.
    if let Some(h) = store.lowest_invoice_height().await? {
        info!(
            "Pending invoices found in AcceptXMR database. Height of lowest invoice: {}",
            h
        );
        return Ok(Some(h));
    };

    Ok(None)
}

/// Errors specific to the blockchain scanner.
#[derive(Error, Debug)]
pub enum ScannerError {
    /// Failure to unblind the amount of an owned output.
    #[error("unable to unblind amount of owned output sent to subaddress index {0}")]
    Unblind(SubIndex),
    /// An error storing/retrieving data from the storage layer.
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
    /// Payment gateway encountered an error while trying to join the scanning
    /// thread.
    #[error("payment gateway encountered an error while trying to join the scanning thread: {0}")]
    Threading(#[from] JoinError),
    /// Failure to check if output is owned.
    #[error("failed to check if output is owned: {0}")]
    OwnedOutputCheck(#[from] monero::blockdata::transaction::Error),
    /// Output has unsupported target. This could mean that `AcceptXMR` is
    /// connected to an outdated daemon.
    #[error("unsupported output target")]
    OutputTarget,
    /// Output index was too large.
    #[error("output index too large")]
    OutputIndex,
    /// An error originating from a daemon RPC call.
    #[error("RPC error: {0}")]
    Rpc(#[from] MonerodRpcError),
    /// An error with the block cache.
    #[error("Block cache error: {0}")]
    BlockCache(#[from] BlockCacheError),
    /// An error with the txpool cache.
    #[error("Txpool cache error: {0}")]
    TxpoolCache(#[from] TxpoolCacheError),
}
