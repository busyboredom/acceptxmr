use std::collections::HashMap;

use log::trace;
use monero::cryptonote::hash::Hashable;
use tokio::join;

use crate::{invoice::Transfer, rpc::RpcClient, AcceptXmrError, SubIndex};

pub(crate) struct TxpoolCache {
    rpc_client: RpcClient,
    transactions: HashMap<monero::Hash, monero::Transaction>,
    discovered_transfers: HashMap<monero::Hash, Vec<(SubIndex, Transfer)>>,
}

impl TxpoolCache {
    pub(crate) async fn init(rpc_client: RpcClient) -> Result<TxpoolCache, AcceptXmrError> {
        let txs = rpc_client.txpool().await?;
        let transactions = txs.iter().map(|tx| (tx.hash(), tx.clone())).collect();

        Ok(TxpoolCache {
            rpc_client,
            transactions,
            discovered_transfers: HashMap::new(),
        })
    }

    /// Update the txpool cache with newest [tansactions](monero::Transaction)
    /// from daemon txpool. Returns transactions received.
    pub(crate) async fn update(&mut self) -> Result<Vec<monero::Transaction>, AcceptXmrError> {
        trace!("Checking for new transactions in txpool");

        let txpool_hashes = self.rpc_client.txpool_hashes().await?;
        trace!("Transactions in txpool: {}", txpool_hashes.len());
        let mut new_hashes = Vec::new();
        for hash in &txpool_hashes {
            if !self.transactions.contains_key(hash) {
                new_hashes.push(*hash);
            }
        }

        // Cloning RPC client because async block below requires unique access to
        // `self`.
        //
        // TODO: Find a way to do this without cloning.
        let rpc_client = self.rpc_client.clone();
        let (new_transactions, ()) = join!(rpc_client.transactions_by_hashes(&new_hashes), async {
            self.transactions.retain(|k, _| txpool_hashes.contains(k));
            self.discovered_transfers
                .retain(|k, _| txpool_hashes.contains(k));
        });
        let new_transactions = new_transactions?;

        self.transactions
            .extend(new_transactions.iter().map(|tx| (tx.hash(), tx.clone())));

        Ok(new_transactions)
    }

    pub(crate) fn discovered_transfers(&self) -> &HashMap<monero::Hash, Vec<(SubIndex, Transfer)>> {
        &self.discovered_transfers
    }

    pub(crate) fn insert_transfers(
        &mut self,
        transfers: &HashMap<monero::Hash, Vec<(SubIndex, Transfer)>>,
    ) {
        self.discovered_transfers.extend(transfers.clone());
        trace!(
            "Txpool contains {} transfers for tracked invoices",
            self.discovered_transfers.len(),
        );
    }
}
