use std::time::Duration;

use sled::Event;

use crate::{payments_db::PaymentStorageErrorKind, Error, Payment};

pub struct Subscriber(sled::Subscriber);

impl Subscriber {
    pub fn new(sled_subscriber: sled::Subscriber) -> Subscriber {
        Subscriber(sled_subscriber)
    }
}

impl Iterator for Subscriber {
    type Item = Result<Payment, Error>;

    fn next(&mut self) -> Option<Result<Payment, Error>> {
        // TODO: This shouldn't be using a timeout, but I am unaware of a better way to do it
        // given the limited options made available by sled.
        match self.0.next_timeout(Duration::from_nanos(0)) {
            Ok(Event::Insert { value, .. }) => Some(
                bincode::deserialize(&value)
                    .map_err(|e| Error::from(PaymentStorageErrorKind::from(e))),
            ),
            _ => None,
        }
    }
}
