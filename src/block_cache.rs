use std::convert::TryInto;

use log::{debug, trace, warn};
use monero;

use crate::{util, Error};

pub struct BlockCache {
    pub height: u64,
    pub blocks: Vec<(monero::Hash, monero::Block, Vec<monero::Transaction>)>,
}

impl BlockCache {
    pub async fn init(
        url: &str,
        cache_size: u64,
        initial_height: u64,
    ) -> Result<BlockCache, Error> {
        let mut blocks = Vec::with_capacity(cache_size.try_into().unwrap());
        for i in 0..cache_size {
            let height = initial_height - i;
            let (block_id, block) = util::get_block(url, height).await?;
            let transactions = util::get_block_transactions(url, &block).await?;
            blocks.push((block_id, block, transactions));
        }

        Ok(BlockCache {
            height: initial_height,
            blocks,
        })
    }

    pub async fn update(&mut self, url: &str) -> Result<(), Error> {
        // If the cache is behind, get a new block and drop the oldest.
        trace!("Checking for block cache updates.");
        let blockchain_height = util::get_current_height(url).await?;
        if self.height < blockchain_height {
            let (block_id, block) = util::get_block(url, self.height + 1).await?;
            let transactions = util::get_block_transactions(url, &block).await?;
            self.blocks.insert(0, (block_id, block, transactions));
            self.blocks.remove(self.blocks.len() - 1);
            self.height += 1;
            debug!(
                "Cache height updated to {}, blockchain height is {}",
                self.height, blockchain_height
            );
        }

        // Check for reorgs, and update blocks if one has occurred.
        for i in 0..(self.blocks.len() - 1) {
            if self.blocks[i].1.header.prev_id != self.blocks[i + 1].0 {
                warn!("Blocks in cache not consecutive! A reorg may have occurd; repairing now.");
                let (block_id, block) = util::get_block(url, self.height + 1).await?;
                let transactions = util::get_block_transactions(url, &block).await?;
                self.blocks[i + 1] = (block_id, block, transactions);
            }
        }
        Ok(())
    }
}
