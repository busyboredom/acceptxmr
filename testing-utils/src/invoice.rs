use std::cmp::max;

use acceptxmr::{Invoice, SubIndex};

#[derive(Clone)]
pub struct MockInvoice {
    pub address: Option<String>,
    pub index: SubIndex,
    pub creation_height: u64,
    pub amount_requested: u64,
    pub amount_paid: u64,
    pub paid_height: Option<u64>,
    pub confirmations_required: u64,
    pub current_height: u64,
    pub expiration_height: u64,
    pub description: String,

    // Calculated fields.
    pub is_expired: bool,
    pub expires_in: u64,
    pub is_confirmed: bool,
    pub confirmations: Option<u64>,
}

impl MockInvoice {
    #[must_use]
    pub fn new(
        address: Option<String>,
        index: SubIndex,
        creation_height: u64,
        amount_requested: u64,
        confirmations_required: u64,
        expires_in: u64,
        description: String,
    ) -> MockInvoice {
        MockInvoice {
            address,
            index,
            creation_height,
            amount_requested,
            amount_paid: 0,
            paid_height: None,
            confirmations_required,
            current_height: creation_height,
            expiration_height: creation_height + expires_in,
            description,

            is_expired: false,
            expires_in,
            is_confirmed: false,
            confirmations: None,
        }
    }

    pub fn assert_eq(&self, update: &Invoice) {
        if let Some(address) = &self.address {
            assert_eq!(update.address(), address);
        }
        assert_eq!(update.index(), self.index);
        assert_eq!(update.creation_height(), self.creation_height);
        assert_eq!(update.amount_requested(), self.amount_requested);
        assert_eq!(update.amount_paid(), self.amount_paid);
        assert_eq!(update.confirmations_required(), self.confirmations_required);
        assert_eq!(update.current_height(), self.current_height);
        assert_eq!(update.expiration_height(), self.expiration_height);
        assert_eq!(update.expiration_height(), self.expiration_height);
        assert_eq!(
            update.expiration_height() - max(update.creation_height(), update.current_height()),
            self.expires_in
        );
        assert_eq!(update.description(), self.description);

        // Calculated fields.
        assert_eq!(update.is_expired(), self.is_expired);
        assert_eq!(update.expiration_in(), self.expires_in);
        assert_eq!(update.is_confirmed(), self.is_confirmed);
        assert_eq!(update.confirmations(), self.confirmations);
    }
}
