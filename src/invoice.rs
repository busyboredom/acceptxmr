use std::fmt;
use std::{
    cmp::{self, Ordering},
    fmt::Display,
};

use monero::cryptonote::subaddress;
use serde::{Deserialize, Serialize};

/// Representation of an invoice. `Invoice`s are created by the [`PaymentGateway`](crate::PaymentGateway), and are
/// initially unpaid.
///
/// `Invoice`s have an expiration block, after which they are considered expired. However, note that
/// the payment gateway by default will continue updating invoices even after expiration.
///
/// To receive updates for a given `Invoice`, use a [`Subscriber`](crate::subscriber::Subscriber).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Invoice {
    address: String,
    index: SubIndex,
    creation_height: u64,
    amount_requested: u64,
    pub(crate) amount_paid: u64,
    pub(crate) paid_height: Option<u64>,
    confirmations_required: u64,
    pub(crate) current_height: u64,
    expiration_height: u64,
    pub(crate) transfers: Vec<Transfer>,
    description: String,
}

impl Invoice {
    pub(crate) fn new(
        address: &str,
        index: SubIndex,
        creation_height: u64,
        amount_requested: u64,
        confirmations_required: u64,
        expiration_in: u64,
        description: &str,
    ) -> Invoice {
        let expiration_height = creation_height + expiration_in;
        Invoice {
            address: address.to_string(),
            index,
            creation_height,
            amount_requested,
            amount_paid: 0,
            /// The height at which the `Invoice` was fully paid. Will be `None` if not yet fully
            /// paid, or if the required XMR is still in the txpool (which has no height).
            paid_height: None,
            confirmations_required,
            current_height: 0,
            expiration_height,
            transfers: Vec::new(),
            description: description.to_string(),
        }
    }

    /// Returns `true` if the `Invoice` has received the required number of confirmations.
    #[must_use]
    pub fn is_confirmed(&self) -> bool {
        self.confirmations().map_or(false, |confirmations| {
            confirmations >= self.confirmations_required
        })
    }

    /// Returns `true` if the `Invoice`'s current block is greater than or equal to its expiration
    /// block.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        // At or passed the expiration block, AND not paid in full.
        (self.current_height >= self.expiration_height) && self.paid_height.is_none()
    }

    /// Returns the base 58 encoded subaddress of this `Invoice`.
    #[must_use]
    pub fn address(&self) -> String {
        self.address.clone()
    }

    /// Returns the ID of this invoice.
    #[must_use]
    pub fn id(&self) -> InvoiceId {
        InvoiceId {
            sub_index: self.index,
            creation_height: self.creation_height,
        }
    }

    /// Returns the [subaddress index](SubIndex) of this `Invoice`.
    #[must_use]
    pub fn index(&self) -> SubIndex {
        self.index
    }

    /// Returns the blockchain height at which the `Invoice` was created.
    #[must_use]
    pub fn creation_height(&self) -> u64 {
        self.creation_height
    }

    /// Returns the amount of monero requested, in piconeros.
    #[must_use]
    pub fn amount_requested(&self) -> u64 {
        self.amount_requested
    }

    /// Returns the amount of monero paid, in piconeros.
    #[must_use]
    pub fn amount_paid(&self) -> u64 {
        self.amount_paid
    }

    /// Returns the number of confirmations this `Invoice` requires before it is considered fully confirmed.
    #[must_use]
    pub fn confirmations_required(&self) -> u64 {
        self.confirmations_required
    }

    /// Returns the number of confirmations this `Invoice` has received since it was paid in full.
    /// Returns `None` if the `Invoice` has not yet been paid in full.
    #[must_use]
    pub fn confirmations(&self) -> Option<u64> {
        if self.amount_paid > self.amount_requested {
            self.paid_height.map_or(Some(0), |paid_at| {
                Some(self.current_height.saturating_sub(paid_at) + 1)
            })
        } else {
            None
        }
    }

    /// Returns the last height at which this `Invoice` was updated.
    #[must_use]
    pub fn current_height(&self) -> u64 {
        self.current_height
    }

    /// Returns the height at which this `Invoice` will expire.
    #[must_use]
    pub fn expiration_height(&self) -> u64 {
        self.expiration_height
    }

    /// Returns the number of blocks before expiration.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// #
    /// # use acceptxmr::PaymentGatewayBuilder;
    /// #
    /// # let private_view_key = "ad2093a5705b9f33e6f0f0c1bc1f5f639c756cdfc168c8f2ac6127ccbdab3a03";
    /// # let public_spend_key = "7388a06bd5455b793a82b90ae801efb9cc0da7156df8af1d5800e4315cc627b4";
    /// #
    /// # let payment_gateway = PaymentGatewayBuilder::new(private_view_key, public_spend_key)
    /// #    .build();
    /// #
    /// # payment_gateway.run().await?;
    /// #
    /// // Create a new `Invoice` requiring 3 confirmations, and expiring in 5 blocks.
    /// let invoice_id = payment_gateway.new_invoice(10000, 3, 5, "for pizza").await?;
    /// let mut subscriber = payment_gateway.subscribe(invoice_id)?.expect("invoice ID not found");
    /// let invoice = subscriber.recv()?;
    ///
    /// assert_eq!(invoice.expiration_in(), 5);
    /// #   Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn expiration_in(&self) -> u64 {
        self.expiration_height.saturating_sub(self.current_height)
    }

    /// Returns the description of this invoice.
    #[must_use]
    pub fn description(&self) -> String {
        self.description.clone()
    }
}

impl fmt::Display for Invoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let confirmations = match self.confirmations() {
            Some(height) => height.to_string(),
            None => "N/A".to_string(),
        };
        let mut str = format!(
            "Index {}: \
            \nPaid: {}/{} \
            \nConfirmations: {} \
            \nStarted at: {} \
            \nCurrent height: {} \
            \nExpiration at: {} \
            \nDescription: \"{}\" \
            \ntransfers: \
            \n[",
            self.index,
            monero::Amount::from_pico(self.amount_paid).as_xmr(),
            monero::Amount::from_pico(self.amount_requested).as_xmr(),
            confirmations,
            self.creation_height,
            self.current_height,
            self.expiration_height,
            self.description,
        );
        for transfer in &self.transfers {
            let height = match transfer.height {
                Some(h) => h.to_string(),
                None => "N/A".to_string(),
            };
            str.push_str(&format!(
                "\n   {{Amount: {}, Height: {:?}}}",
                transfer.amount, height
            ));
        }
        if self.transfers.is_empty() {
            str.push(']');
        } else {
            str.push_str("\n]");
        }
        write!(f, "{}", str)
    }
}

/// An invoice ID consists uniquely identifies a given invoice by the combination of its subaddress
/// index and creation height.
#[derive(Debug, Copy, Clone, Hash, Serialize, Deserialize, PartialEq, Eq)]
pub struct InvoiceId {
    /// The [subaddress index](SubIndex) of the invoice.
    pub sub_index: SubIndex,
    /// The creation height of the invoice.
    pub creation_height: u64,
}

impl InvoiceId {
    /// Create a new `InvoiceId` from subaddress index and creation height.
    #[must_use]
    pub fn new(sub_index: SubIndex, creation_height: u64) -> InvoiceId {
        InvoiceId {
            sub_index,
            creation_height,
        }
    }
}

impl Display for InvoiceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({},{})", self.sub_index, self.creation_height)
    }
}

/// A subaddress index.
#[derive(Debug, Copy, Clone, Hash, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubIndex {
    /// Subadress major index.
    pub major: u32,
    /// Subaddress minor index.
    pub minor: u32,
}

impl SubIndex {
    /// Create a new subaddress index from major and minor indexes.
    #[must_use]
    pub fn new(major: u32, minor: u32) -> SubIndex {
        SubIndex { major, minor }
    }
}

impl Ord for SubIndex {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.major.cmp(&other.major) {
            Ordering::Equal => self.minor.cmp(&other.minor),
            Ordering::Greater => Ordering::Greater,
            Ordering::Less => Ordering::Less,
        }
    }
}

impl PartialOrd for SubIndex {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for SubIndex {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        write!(formatter, "{}/{}", self.major, self.minor)
    }
}

impl From<subaddress::Index> for SubIndex {
    fn from(index: subaddress::Index) -> SubIndex {
        SubIndex {
            major: index.major,
            minor: index.minor,
        }
    }
}

impl From<SubIndex> for subaddress::Index {
    fn from(index: SubIndex) -> subaddress::Index {
        subaddress::Index {
            major: index.major,
            minor: index.minor,
        }
    }
}

/// A `Transfer` represents a sum of owned outputs at a given height. When part of an `Invoice`, it
/// specifically represents the sum of owned outputs for that invoice's subaddress, at a given
/// height.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Copy)]
pub(crate) struct Transfer {
    /// Amount transferred in piconeros.
    pub amount: u64,
    /// Block height of the transfer, or None if the outputs are in the txpool.
    pub height: Option<u64>,
}

impl Transfer {
    pub(crate) fn new(amount: u64, height: Option<u64>) -> Transfer {
        Transfer { amount, height }
    }

    /// Compare two transfers by height. Newer is greater.
    pub(crate) fn cmp_by_height(&self, other: &Self) -> cmp::Ordering {
        match self.height {
            Some(height) => match other.height {
                Some(other_height) => height.cmp(&other_height),
                None => cmp::Ordering::Less,
            },
            None => match other.height {
                Some(_) => cmp::Ordering::Greater,
                None => cmp::Ordering::Equal,
            },
        }
    }
}
