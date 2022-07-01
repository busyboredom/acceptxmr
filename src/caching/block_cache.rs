use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use log::{debug, trace, warn};

use crate::{rpc::RpcClient, AcceptXmrError};

pub(crate) struct BlockCache {
    pub height: Arc<AtomicU64>,
    pub blocks: Vec<(monero::Hash, u64, monero::Block, Vec<monero::Transaction>)>,
    rpc_client: RpcClient,
}

impl BlockCache {
    pub async fn init(
        rpc_client: RpcClient,
        cache_size: usize,
        initial_height: Arc<AtomicU64>,
    ) -> Result<BlockCache, AcceptXmrError> {
        let mut blocks = Vec::with_capacity(cache_size);
        // TODO: Get blocks concurrently.
        for i in 0..cache_size {
            let height = initial_height.load(Ordering::Relaxed) - i as u64;
            let (block_id, block) = rpc_client.block(height).await?;
            let transactions = rpc_client.block_transactions(&block).await?;
            blocks.push((block_id, height, block, transactions));
        }

        let mut block_cache_summary = "".to_string();
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
            rpc_client,
            height: initial_height,
            blocks,
        })
    }

    /// Update the block cache with newest blocks from daemon. Returns number of blocks updated.
    pub async fn update(&mut self) -> Result<usize, AcceptXmrError> {
        // If the cache is behind, get a new block and drop the oldest.
        trace!("Checking for block cache updates");
        let mut updated = 0;
        let blockchain_height = self.rpc_client.daemon_height().await?;
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
                "Cache height updated to {}, blockchain height is {}, blockchain top block height is {}",
                self.height.load(Ordering::Relaxed),
                blockchain_height,
                blockchain_height - 1,
            );
            let mut block_cache_summary = "".to_string();
            for i in 0..self.blocks.len() {
                block_cache_summary += &format!(
                    "Index in cache: {}\nHeight: {}\nNumber of transactions: {}\nID: {}\n\n",
                    i,
                    self.blocks[i].1,
                    self.blocks[i].3.len(),
                    self.blocks[i].0
                );
            }
            trace!("Block cache summary:\n{}", block_cache_summary);
            updated += 1;
        }

        // Check for reorgs, and update blocks if one has occurred.
        for i in 0..self.blocks.len() - 1 {
            if self.blocks[i].2.header.prev_id != self.blocks[i + 1].0 {
                warn!("Blocks in cache not consecutive! A reorg may have occurred; repairing now");
                let (block_id, block) = self
                    .rpc_client
                    .block(self.height.load(Ordering::Relaxed) - 1)
                    .await?;
                let transactions = self.rpc_client.block_transactions(&block).await?;
                self.blocks[i + 1] = (
                    block_id,
                    self.height.load(Ordering::Relaxed) + 1,
                    block,
                    transactions,
                );
                updated += 1;
            }
        }

        Ok(updated)
    }
}
