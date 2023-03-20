use std::{
    cmp::{max, min},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use log::{debug, trace, warn};

use crate::{rpc::RpcClient, storage::InvoiceStorage, AcceptXmrError};

pub(crate) struct BlockCache {
    pub height: Arc<AtomicU64>,
    pub daemon_height: Arc<AtomicU64>,
    pub blocks: Vec<(monero::Hash, u64, monero::Block, Vec<monero::Transaction>)>,
    cache_size: usize,
    rpc_client: RpcClient,
}

impl BlockCache {
    pub async fn init<S: InvoiceStorage>(
        rpc_client: RpcClient,
        cache_size: usize,
        initial_height: Arc<AtomicU64>,
        daemon_height: Arc<AtomicU64>,
    ) -> Result<BlockCache, AcceptXmrError<S::Error>> {
        let mut blocks = Vec::with_capacity(cache_size);
        // TODO: Get blocks concurrently.
        for i in 0..cache_size {
            let height = initial_height.load(Ordering::Relaxed) - i as u64;
            let (block_id, block) = rpc_client.block(height).await?;
            let transactions = rpc_client.block_transactions(&block).await?;
            blocks.push((block_id, height, block, transactions));
        }

        let mut block_cache_summary = String::new();
        for (i, block) in blocks.iter().enumerate() {
            block_cache_summary += &format!(
                "Index in cache: {}\nHeight: {}\nNumber of transactions: {}\nID: {}\n\n",
                i,
                block.1,
                block.3.len(),
                block.0
            );
        }
        trace!("Block cache initialized. Summary:\n{}", block_cache_summary);

        Ok(BlockCache {
            height: initial_height,
            daemon_height,
            blocks,
            cache_size,
            rpc_client,
        })
    }

    /// Update the block cache with newest blocks from daemon and apply reorg if
    /// one has occurred. Returns number of blocks updated.
    pub async fn skip_ahead<S: InvoiceStorage>(
        &mut self,
    ) -> Result<usize, AcceptXmrError<S::Error>> {
        trace!("Checking for block cache updates");
        let mut updated = 0;
        let cache_height = self.height.load(Ordering::Relaxed);
        let blockchain_height = self.rpc_client.daemon_height().await?;
        self.daemon_height
            .store(blockchain_height, Ordering::Relaxed);
        if cache_height < blockchain_height - 1 {
            for i in (0..min(
                blockchain_height.saturating_sub(cache_height + 1),
                self.cache_size as u64,
            ))
                .rev()
            {
                let height = blockchain_height - 1 - i;
                let (block_id, block) = self.rpc_client.block(height).await?;
                let transactions = self.rpc_client.block_transactions(&block).await?;
                self.blocks
                    .insert(0, (block_id, height, block, transactions));
                self.blocks.remove(self.blocks.len() - 1);
                self.height.store(height, Ordering::Relaxed);
                updated += 1;
            }
            debug!(
            "Cache top block height updated to {}, blockchain top block height is {}, blockchain height is {}",
            self.height.load(Ordering::Relaxed),
            blockchain_height - 1,
            blockchain_height,
        );
            updated = max(updated, self.check_and_fix_reorg::<S>().await?);
            self.log_cache_summary();
        }
        Ok(updated)
    }

    /// Advance block cache by 1 block if new block is available and apply reorg
    /// if one has occurred. Returns number of blocks updated.
    pub async fn update<S: InvoiceStorage>(&mut self) -> Result<usize, AcceptXmrError<S::Error>> {
        trace!("Checking for block cache updates");
        let mut updated = 0;
        let blockchain_height = self.rpc_client.daemon_height().await?;
        self.daemon_height
            .store(blockchain_height, Ordering::Relaxed);
        if self.height.load(Ordering::Relaxed) < blockchain_height - 1 {
            let (block_id, block) = self
                .rpc_client
                .block(self.height.load(Ordering::Relaxed) + 1)
                .await?;
            let transactions = self.rpc_client.block_transactions(&block).await?;
            self.blocks.insert(
                0,
                (
                    block_id,
                    self.height.load(Ordering::Relaxed) + 1,
                    block,
                    transactions,
                ),
            );
            self.blocks.remove(self.blocks.len() - 1);
            self.height.fetch_add(1, Ordering::Relaxed);
            debug!(
                "Cache top block height updated to {}, blockchain top block height is {}, blockchain height is {}",
                self.height.load(Ordering::Relaxed),
                blockchain_height - 1,
                blockchain_height,
            );
            self.log_cache_summary();
            updated += 1;
        }
        updated = max(updated, self.check_and_fix_reorg::<S>().await?);

        Ok(updated)
    }

    /// Check for reorgs, and update blocks if one has occurred.
    async fn check_and_fix_reorg<S: InvoiceStorage>(
        &mut self,
    ) -> Result<usize, AcceptXmrError<S::Error>> {
        let mut updated = 0;
        let cache_height = self.height.load(Ordering::Relaxed);
        for i in 0..self.blocks.len() - 1 {
            if self.blocks[i].2.header.prev_id != self.blocks[i + 1].0 {
                warn!("Blocks in cache not consecutive! A reorg may have occurred; repairing now");
                let (block_id, block) = self.rpc_client.block(cache_height - 1 - i as u64).await?;
                let transactions = self.rpc_client.block_transactions(&block).await?;
                self.blocks[i + 1] = (block_id, cache_height - 1 - i as u64, block, transactions);
                updated = max(updated, 1);
                updated += 1;
            }
        }
        Ok(updated)
    }

    fn log_cache_summary(&self) {
        let mut block_cache_summary = String::new();
        for i in 0..self.blocks.len() {
            block_cache_summary += &format!(
                "Index in cache: {}\nHeight: {}\nNumber of transactions: {}\nID: {}\nPrevious ID: {}\n\n",
                i,
                self.blocks[i].1,
                self.blocks[i].3.len(),
                self.blocks[i].0,
                self.blocks[i].2.header.prev_id,
            );
        }
        trace!("Block cache summary:\n{}", block_cache_summary);
    }
}
