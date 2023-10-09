use std::{ops::Deref, sync::Arc};

use acceptxmr::{storage::stores::Sqlite, PaymentGateway};

pub(crate) struct State(Arc<StateInner>);

impl State {
    pub(crate) fn new(payment_gateway: PaymentGateway<Sqlite>) -> Self {
        Self(Arc::new(StateInner { payment_gateway }))
    }
}

pub(crate) struct StateInner {
    pub(crate) payment_gateway: PaymentGateway<Sqlite>,
}

impl Deref for State {
    type Target = StateInner;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Clone for State {
    fn clone(&self) -> Self {
        State(self.0.clone())
    }
}
