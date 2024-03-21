use std::{
    env::{self, VarError},
    str::FromStr,
};

use monero::{Address, PrivateKey};
use secrecy::{ExposeSecret, Secret};
use serde::{Deserialize, Serialize};

use super::ConfigError;

#[derive(Deserialize, Debug, Serialize, Default)]
#[serde(rename_all = "kebab-case")]
pub struct WalletConfig {
    /// Monero wallet's primary address. Should begin with a `4`.
    pub primary_address: Option<Address>,
    /// Monero wallet private view key. For best security, this should be set
    /// via the `PRIVATE_VIEWKEY` environment variable.
    #[serde(skip_serializing)]
    pub private_viewkey: Option<Secret<String>>,
    /// The account index to be used. Defaults to 0.
    #[serde(default)]
    pub account_index: u32,
    /// The restore height of the wallet. Defaults to the current blockchain
    /// tip.
    #[serde(default)]
    pub restore_height: Option<u64>,
}

impl WalletConfig {
    pub(super) fn apply_env_overrides(mut self) -> Result<Self, ConfigError> {
        match env::var("PRIVATE_VIEWKEY") {
            Ok(key) => {
                self.private_viewkey = Some(Secret::new(key));
            }
            Err(VarError::NotPresent) => {}
            Err(e) => return Err(e)?,
        }
        Ok(self)
    }

    pub(super) fn validate(&self) {
        assert!(
            self.primary_address.is_some(),
            "please configure your monero primary address. Your primary address should begin with a 4."
        );
        assert!(
            self.private_viewkey.is_some(),
            "please configure your monero private viewkey. For best security, set it using the PRIVATE_VIEWKEY environment variable."
        );
        if let Some(key) = &self.private_viewkey {
            PrivateKey::from_str(key.expose_secret()).expect("invalid private view key");
        }
    }
}

impl PartialEq for WalletConfig {
    fn eq(&self, other: &Self) -> bool {
        let addresses_match = self.primary_address == other.primary_address;
        let viewkeys_match = match (
            self.private_viewkey.as_ref(),
            other.private_viewkey.as_ref(),
        ) {
            (Some(viewkey), Some(other_viewkey)) => {
                viewkey.expose_secret() == other_viewkey.expose_secret()
            }
            (None, None) => true,
            _ => false,
        };
        let accounts_match = self.account_index == other.account_index;
        let restore_heights_match = self.restore_height == other.restore_height;

        addresses_match && viewkeys_match && accounts_match && restore_heights_match
    }
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
mod test {
    use std::{env, panic::catch_unwind, str::FromStr};

    use monero::Address;
    use secrecy::{ExposeSecret, Secret};
    use test_case::test_case;

    use super::WalletConfig;

    const ADDRESS_1: &str = "4613YiHLM6JMH4zejMB2zJY5TwQCxL8p65ufw8kBP5yxX9itmuGLqp1dS4tkVoTxjyH3aYhYNrtGHbQzJQP5bFus3KHVdmf";
    const ADDRESS_2: &str = "82assiV5dy7guoxxV7vSReZTyY5rGMrWg6BsfvFqiEKRcTiDs7LGMpg5dF5gXVGUWPEXQxyt8SNYx8L8HiGAzvtBK3eJ3EY";
    const ADDRESS_INVALID: &str = "5613YiHLM6JMH4zejMB2zJY5TwQCxL8p65ufw8kBP5yxX9itmuGLqp1dS4tkVoTxjyH3aYhYNrtGHbQzJQP5bFus3KHVdmf";
    const VIEWKEY_1: &str = "ad2093a5705b9f33e6f0f0c1bc1f5f639c756cdfc168c8f2ac6127ccbdab3a03";
    const VIEWKEY_INVALID: &str = "d2093a5705b9f33e6f0f0c1bc1f5f639c756cdfc168c8f2ac6127ccbdab3a03";

    #[test_case(None => Some(VIEWKEY_1.to_string()); "env var key only")]
    #[test_case(Some("configkey") => Some(VIEWKEY_1.to_string()); "key override")]
    fn apply_env_overrides(config_key: Option<&str>) -> Option<String> {
        let mut config = WalletConfig {
            private_viewkey: config_key.map(|key| Secret::new(key.to_string())),
            ..Default::default()
        };

        env::set_var("PRIVATE_VIEWKEY", VIEWKEY_1);

        config = config.apply_env_overrides().unwrap();
        config
            .private_viewkey
            .map(|key| key.expose_secret().clone())
    }

    #[test_case(None, None => false; "not configured")]
    #[test_case(None, Some(VIEWKEY_1) => false; "no address")]
    #[test_case(Some(ADDRESS_1), None => false; "no key")]
    #[test_case(Some(ADDRESS_1), Some(VIEWKEY_1)=> true; "all configured")]
    #[test_case(Some(ADDRESS_INVALID), Some(VIEWKEY_1)=> false; "invalid address")]
    #[test_case(Some(ADDRESS_1), Some(VIEWKEY_INVALID)=> false; "invalid key")]
    fn validate(address: Option<&str>, viewkey: Option<&str>) -> bool {
        catch_unwind(|| {
            let config = WalletConfig {
                primary_address: address.map(|addr| Address::from_str(addr).unwrap()),
                private_viewkey: viewkey.map(|key| Secret::new(key.to_string())),
                account_index: 123,
                restore_height: Some(12345),
            };
            config.validate();
        })
        .is_ok()
    }

    #[test_case(Some(ADDRESS_1), None, 0, None, Some(ADDRESS_1), None, 0, None => true; "match no key")]
    #[test_case(Some(ADDRESS_1), Some(VIEWKEY_1), 0, None, Some(ADDRESS_1), Some(VIEWKEY_1), 0, None => true; "match with key")]
    #[test_case(Some(ADDRESS_1), None, 0, None, Some(ADDRESS_2), None, 0, None => false; "mismatch no key")]
    #[test_case(Some(ADDRESS_1), Some(VIEWKEY_1), 0, None, Some(ADDRESS_1), Some(VIEWKEY_INVALID), 0, None => false; "mismatch with key")]
    #[test_case(Some(ADDRESS_1), Some(VIEWKEY_1), 0, None, Some(ADDRESS_1), None, 0, None => false; "mismatch one key")]
    #[test_case(None, Some(VIEWKEY_1), 0, None, None, Some(VIEWKEY_1), 0, None => true; "match no address")]
    #[test_case(Some(ADDRESS_1), Some(VIEWKEY_1), 0, None, Some(ADDRESS_1), Some(VIEWKEY_1), 123, None => false; "mismatch account index")]
    #[test_case(Some(ADDRESS_1), Some(VIEWKEY_1), 0, Some(123), Some(ADDRESS_1), Some(VIEWKEY_1), 0, Some(124) => false; "mismatch restore height")]
    fn eq(
        address1: Option<&str>,
        viewkey1: Option<&str>,
        account_index1: u32,
        restore_height1: Option<u64>,
        address2: Option<&str>,
        viewkey2: Option<&str>,
        account_index2: u32,
        restore_height2: Option<u64>,
    ) -> bool {
        let wallet1 = WalletConfig {
            primary_address: address1.map(|addr| Address::from_str(addr).unwrap()),
            private_viewkey: viewkey1.map(|key| Secret::new(key.to_string())),
            account_index: account_index1,
            restore_height: restore_height1,
        };

        let wallet2 = WalletConfig {
            primary_address: address2.map(|addr| Address::from_str(addr).unwrap()),
            private_viewkey: viewkey2.map(|key| Secret::new(key.to_string())),
            account_index: account_index2,
            restore_height: restore_height2,
        };

        wallet1 == wallet2
    }
}
