use std::convert::TryInto;

use monero;

use crate::util;

pub struct BlockCache {
    pub height: u64,
    pub blocks: Vec<(monero::Hash, monero::Block, Vec<monero::Transaction>)>,
}

impl BlockCache {
    pub async fn init(
        url: &str,
        cache_size: u64,
        initial_height: u64,
    ) -> Result<BlockCache, reqwest::Error> {
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

    pub async fn update(&mut self, url: &str) -> Result<(), reqwest::Error> {
        // If the cache is behind, get a new block and drop the oldest.
        println!("Checking for block cache updates.");
        if self.height < util::get_current_height(url).await? {
            let (block_id, block) = util::get_block(url, self.height + 1).await?;
            let transactions = util::get_block_transactions(url, &block).await?;
            self.blocks.insert(0, (block_id, block, transactions));
            self.blocks.remove(self.blocks.len() - 1);
            self.height += 1;
            println!("Cache height updated, new height is: {}", self.height);
        }

        // Check for reorgs, and update blocks if one has occurred.
        for i in 0..(self.blocks.len() - 1) {
            if self.blocks[i].1.header.prev_id != self.blocks[i + 1].0 {
                println!(
                    "Blocks in cache not consecutive! A reorg may have occurd; repairing now."
                );
                let (block_id, block) = util::get_block(url, self.height + 1).await?;
                let transactions = util::get_block_transactions(url, &block).await?;
                self.blocks[i + 1] = (block_id, block, transactions);
            }
        }
        Ok(())
    }
}
