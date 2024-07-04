use std::{env, env::VarError, time::Duration};

use hyper::Uri;
use log::warn;
use secrecy::{ExposeSecret, Secret};
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr, DurationSeconds};

use super::ConfigError;

#[serde_as]
#[derive(Deserialize, PartialEq, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct DaemonConfig {
    /// URL of monero daemon.
    #[serde_as(as = "DisplayFromStr")]
    pub url: Uri,
    /// Monero daemon login credentials, if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login: Option<DaemonLoginConfig>,
    /// Timeout in seconds for RPC calls to the daemon.
    #[serde_as(as = "DurationSeconds")]
    pub rpc_timeout: Duration,
    /// Timeout in seconds for making an RPC connection to the daemon.
    #[serde_as(as = "DurationSeconds")]
    pub connection_timeout: Duration,
}

impl DaemonConfig {
    pub(super) fn apply_env_overrides(mut self) -> Result<Self, ConfigError> {
        match env::var("DAEMON_PASSWORD") {
            Ok(password) => {
                if let Some(login) = self.login.as_mut() {
                    login.password = Some(Secret::new(password));
                } else {
                    warn!("Environment variable DAEMON_PASSWORD was set, but no username was found in the configuration file");
                }
            }
            Err(VarError::NotPresent) => {}
            Err(e) => return Err(e)?,
        }
        Ok(self)
    }

    pub(super) fn validate(&self) {
        if let Some(login) = self.login.as_ref() {
            assert!(
                login.password.is_some(),
                "daemon login exists in config, but a password was not set. For best security, set it using the DAEMON_PASSWORD environment variable."
            );
        }
    }
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            url: Uri::from_static("https://xmr-node.cakewallet.com:18081"),
            login: None,
            rpc_timeout: Duration::from_secs(30),
            connection_timeout: Duration::from_secs(20),
        }
    }
}

/// Username and password of monero daemon.
#[derive(Deserialize, Debug, Serialize)]
pub struct DaemonLoginConfig {
    pub username: String,
    /// Daemon login password. For best security, this should be set via the
    /// `DAEMON_PASSWORD` environment variable.
    #[serde(skip_serializing)]
    pub password: Option<Secret<String>>,
}

impl PartialEq for DaemonLoginConfig {
    fn eq(&self, other: &Self) -> bool {
        let usernames_match = self.username == other.username;
        let passwords_match = match (self.password.as_ref(), other.password.as_ref()) {
            (Some(password), Some(other_password)) => {
                password.expose_secret() == other_password.expose_secret()
            }
            (None, None) => true,
            _ => false,
        };

        usernames_match && passwords_match
    }
}

#[cfg(test)]
mod test {
    use std::{env, panic::catch_unwind};

    use hyper::Uri;
    use secrecy::{ExposeSecret, Secret};
    use test_case::test_case;

    use super::{DaemonConfig, DaemonLoginConfig};

    #[test_case(None => Some("supersecretpassword".to_string()); "env var password only")]
    #[test_case(Some("configpass") => Some("supersecretpassword".to_string()); "password override")]
    fn apply_env_overrides(config_pass: Option<&str>) -> Option<String> {
        let mut config = DaemonConfig {
            login: Some(DaemonLoginConfig {
                username: "jsmith".to_string(),
                password: config_pass.map(|pass| Secret::new(pass.to_string())),
            }),
            ..Default::default()
        };

        env::set_var("DAEMON_PASSWORD", "supersecretpassword");

        config = config.apply_env_overrides().unwrap();
        config
            .login
            .unwrap()
            .password
            .map(|pass| pass.expose_secret().clone())
    }

    #[test_case(&DaemonConfig::default() => true; "default")]
    #[test_case(
        &DaemonConfig {
            url: Uri::from_static("http://example.com"), 
            login: Some(DaemonLoginConfig {username: "jsmith".to_string(), password: None}),
            ..Default::default()
        } => false; "missing password"
    )]
    #[test_case(
        &DaemonConfig {
            url: Uri::from_static("http://example.com"), 
            login: Some(DaemonLoginConfig {username: "jsmith".to_string(), password: Some(Secret::new("p455w0rd".to_string()))}),
            ..Default::default()
        }
        => true; "with password"
    )]
    fn validate(config: &DaemonConfig) -> bool {
        catch_unwind(|| config.validate()).is_ok()
    }

    #[test_case("jsmith", None, "jsmith", None => true; "match no password")]
    #[test_case("jsmith", Some("pass"), "jsmith", Some("pass") => true; "match with password")]
    #[test_case("jsmith", None, "jgalt", None => false; "mismatch no password")]
    #[test_case("jsmith", Some("pass"), "jsmith", Some("pass2") => false; "mismatch with password")]
    #[test_case("jsmith", Some("pass"), "jsmith", None => false; "mismatch one password")]
    fn eq(user1: &str, pass1: Option<&str>, user2: &str, pass2: Option<&str>) -> bool {
        let login1 = DaemonLoginConfig {
            username: user1.to_string(),
            password: pass1.map(|pass| Secret::new(pass.to_string())),
        };

        let login2 = DaemonLoginConfig {
            username: user2.to_string(),
            password: pass2.map(|pass| Secret::new(pass.to_string())),
        };

        login1 == login2
    }
}
