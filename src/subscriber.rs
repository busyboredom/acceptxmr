use std::time::Duration;

use sled::Event;

use crate::{payments_db::PaymentStorageError, AcceptXMRError, Payment};

pub struct Subscriber(sled::Subscriber);

impl Subscriber {
    pub fn new(sled_subscriber: sled::Subscriber) -> Subscriber {
        Subscriber(sled_subscriber)
    }

    pub fn recv(&mut self) -> Result<Payment, AcceptXMRError> {
        let maybe_event = self.0.next();
        match maybe_event {
            Some(Event::Insert { value, .. }) => bincode::deserialize(&value)
                .map_err(|e| AcceptXMRError::from(PaymentStorageError::from(e))),
            Some(Event::Remove { .. }) => self.recv(),
            None => Err(AcceptXMRError::SubscriberRecv),
        }
    }
}

impl Iterator for Subscriber {
    type Item = Result<Payment, AcceptXMRError>;

    fn next(&mut self) -> Option<Result<Payment, AcceptXMRError>> {
        // TODO: This shouldn't be using a timeout, but I am unaware of a better way to do it
        // given the limited options made available by sled.
        match self.0.next_timeout(Duration::from_nanos(0)) {
            Ok(Event::Insert { value, .. }) => Some(
                bincode::deserialize(&value)
                    .map_err(|e| AcceptXMRError::from(PaymentStorageError::from(e))),
            ),
            _ => None,
        }
    }
}
