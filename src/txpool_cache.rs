use std::collections::HashMap;

use log::trace;
use monero::cryptonote::hash::Hashable;
use tokio::join;

use crate::util;

pub struct TxpoolCache {
    pub transactions: HashMap<monero::Hash, monero::Transaction>,
}

impl TxpoolCache {
    pub async fn init(url: &str) -> TxpoolCache {
        let txs = util::retry(url, 2000, util::get_txpool).await;
        let transactions = txs.iter().map(|tx| (tx.hash(), tx.to_owned())).collect();

        TxpoolCache { transactions }
    }

    /// Update the txpool cache with newest tansactions from daemon txpool. Returns number of transactions received.
    pub async fn update(&mut self, url: &str) -> u64 {
        trace!("Checking for new transactions in txpool");
        let retry_millis = 2000;

        let txpool_hashes = util::retry(url, retry_millis, util::get_txpool_hashes).await;
        trace!("Transactions in txpool: {}", txpool_hashes.len());
        let mut new_hashes = Vec::new();
        for hash in txpool_hashes.iter() {
            if !self.transactions.contains_key(hash) {
                new_hashes.push(*hash);
            }
        }

        let (new_transactions, _) = join!(
            util::retry_vec(
                url,
                &new_hashes,
                retry_millis,
                util::get_transactions_by_hashes,
            ),
            async { self.transactions.retain(|k, _| txpool_hashes.contains(k)) }
        );

        self.transactions
            .extend(new_transactions.iter().map(|tx| (tx.hash(), tx.to_owned())));

        new_transactions.len() as u64
    }
}
