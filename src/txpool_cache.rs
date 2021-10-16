use std::collections::HashMap;

use log::trace;
use monero::cryptonote::hash::Hashable;
use tokio::join;

use crate::{rcp, SubIndex, Transfer};

pub(crate) struct TxpoolCache {
    transactions: HashMap<monero::Hash, monero::Transaction>,
    discovered_transfers: HashMap<monero::Hash, Vec<(SubIndex, Transfer)>>,
}

impl TxpoolCache {
    pub async fn init(url: &str) -> TxpoolCache {
        let txs = rcp::retry(url, 2000, rcp::get_txpool).await;
        let transactions = txs.iter().map(|tx| (tx.hash(), tx.clone())).collect();

        TxpoolCache {
            transactions,
            discovered_transfers: HashMap::new(),
        }
    }

    /// Update the txpool cache with newest tansactions from daemon txpool. Returns
    /// transactions received.
    pub async fn update(&mut self, url: &str) -> Vec<monero::Transaction> {
        trace!("Checking for new transactions in txpool");
        let retry_millis = 2000;

        let txpool_hashes = rcp::retry(url, retry_millis, rcp::txpool_hashes).await;
        trace!("Transactions in txpool: {}", txpool_hashes.len());
        let mut new_hashes = Vec::new();
        for hash in &txpool_hashes {
            if !self.transactions.contains_key(hash) {
                new_hashes.push(*hash);
            }
        }

        let (new_transactions, _) = join!(
            rcp::retry_vec(url, &new_hashes, retry_millis, rcp::transactions_by_hashes,),
            async {
                self.transactions.retain(|k, _| txpool_hashes.contains(k));
                self.discovered_transfers
                    .retain(|k, _| txpool_hashes.contains(k));
            }
        );

        self.transactions
            .extend(new_transactions.iter().map(|tx| (tx.hash(), tx.clone())));

        new_transactions
    }

    pub fn discovered_transfers(&self) -> &HashMap<monero::Hash, Vec<(SubIndex, Transfer)>> {
        &self.discovered_transfers
    }

    pub fn insert_transfers(
        &mut self,
        transfers: &HashMap<monero::Hash, Vec<(SubIndex, Transfer)>>,
    ) {
        self.discovered_transfers.extend(transfers.clone());
        trace!(
            "Txpool contains {} transfers for tracked payments",
            self.discovered_transfers.len(),
        );
    }
}
