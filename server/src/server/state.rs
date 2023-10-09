use acceptxmr::{
    storage::{stores::Sqlite, Storage},
    MonerodClient, MonerodRpcClient, PaymentGateway,
};

use crate::config::ServerConfig;

pub(crate) struct State<S: Storage = Sqlite, M: MonerodClient = MonerodRpcClient> {
    pub(crate) payment_gateway: PaymentGateway<S, M>,
    pub(crate) config: ServerConfig,
}

impl<S: Storage, M: MonerodClient> State<S, M> {
    pub(crate) fn new(payment_gateway: PaymentGateway<S, M>, config: ServerConfig) -> Self {
        Self {
            payment_gateway,
            config,
        }
    }
}

impl<S: Storage, M: MonerodClient> Clone for State<S, M> {
    fn clone(&self) -> Self {
        Self {
            payment_gateway: self.payment_gateway.clone(),
            config: self.config.clone(),
        }
    }
}
