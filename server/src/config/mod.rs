mod callback;
mod daemon;
mod database;
mod logging;
mod server;
mod wallet;

use std::{
    env::{self, VarError},
    fs::File,
    io,
    io::{ErrorKind as IoErrorKind, Write},
    path::PathBuf,
};

pub(crate) use callback::CallbackConfig;
use clap::{Arg, ArgAction, Command};
pub(crate) use daemon::DaemonConfig;
pub(crate) use database::DatabaseConfig;
use dotenv::dotenv;
use log::info;
pub(crate) use logging::LoggingConfig;
use secrecy::Secret;
use serde::{Deserialize, Serialize};
use serde_yaml::Error as YamlError;
pub(crate) use server::{ServerConfig, TlsConfig};
use thiserror::Error;
pub(crate) use wallet::WalletConfig;

/// AcceptXMR-Server configuration.
#[derive(Deserialize, PartialEq, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct Config {
    /// Config for the client-facing API.
    pub external_api: ServerConfig,
    /// Config for the internal API.
    pub internal_api: ServerConfig,
    /// Config for callback functionality.
    pub callback: CallbackConfig,
    /// Monero wallet configuration.
    pub wallet: WalletConfig,
    /// Monero daemon configuration.
    pub daemon: DaemonConfig,
    /// Database configuration.
    pub database: DatabaseConfig,
    /// Logging configuration.
    pub logging: LoggingConfig,
}

impl Config {
    /// Default configuration file path.
    pub const DEFAULT_PATH: &'static str = "acceptxmr.yaml";

    /// Get config file path from CLI argument, env variable, or default (in
    /// that order).
    #[allow(clippy::missing_panics_doc)]
    #[must_use]
    pub fn get_path() -> PathBuf {
        let cli_matches = Command::new("AcceptXMR-Server")
            .arg(
                Arg::new("config-file")
                    .short('f')
                    .long("config-file")
                    .action(ArgAction::Set)
                    .value_name("FILE")
                    .env("CONFIG_FILE")
                    .default_value(Self::DEFAULT_PATH)
                    .help("Specifies the config file to use. Defaults to ./acceptxmr.yaml"),
            )
            .get_matches();

        // This `unwrap` is safe because args with a default never return `None`.
        PathBuf::from(cli_matches.get_one::<String>("config-file").unwrap())
    }

    /// Creates config from file. If the file is not found, creates it
    /// and populates it from defaults.
    fn from_file(path: &PathBuf) -> Result<Config, ConfigError> {
        let config_file = match File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == IoErrorKind::NotFound => {
                info!(
                    "Config file {} not found. Creating it from defaults.",
                    path.display()
                );
                let mut f = File::create(path)?;
                let config = Config::default();
                f.write_all(serde_yaml::to_string(&config)?.as_bytes())?;
                return Ok(config);
            }
            Err(e) => return Err(e)?,
        };

        Ok(serde_yaml::from_reader(config_file)?)
    }

    fn apply_env_overrides(mut self) -> Result<Config, ConfigError> {
        // Read from dotenv file if real environment variables are not set.
        dotenv().ok();

        self.wallet = self.wallet.apply_env_overrides()?;
        self.daemon = self.daemon.apply_env_overrides()?;

        match env::var("INTERNAL_API_TOKEN") {
            Ok(token) => {
                self.internal_api.token = Some(Secret::new(token));
            }
            Err(VarError::NotPresent) => {}
            Err(e) => return Err(e)?,
        }

        match env::var("EXTERNAL_API_TOKEN") {
            Ok(token) => {
                self.external_api.token = Some(Secret::new(token));
            }
            Err(VarError::NotPresent) => {}
            Err(e) => return Err(e)?,
        }

        Ok(self)
    }

    /// Validates configuration, panicking if it is invalid.
    pub fn validate(&self) {
        self.wallet.validate();
        self.daemon.validate();
        self.internal_api.validate();
        self.external_api.validate();
    }

    /// Read config and apply environment overrides.
    pub(crate) fn read(path: &PathBuf) -> Result<Config, ConfigError> {
        Self::from_file(path)?.apply_env_overrides()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            external_api: ServerConfig::default(),
            internal_api: ServerConfig {
                port: 8081,
                // Default to self-signed certs if none are provided.
                tls: Some(TlsConfig {
                    cert: PathBuf::from("./cert/certificate.pem"),
                    key: PathBuf::from("./cert/privatekey.pem"),
                }),
                ..Default::default()
            },
            callback: CallbackConfig::default(),
            wallet: WalletConfig::default(),
            daemon: DaemonConfig::default(),
            database: DatabaseConfig::default(),
            logging: LoggingConfig::default(),
        }
    }
}

#[derive(Error, Debug)]
pub(crate) enum ConfigError {
    #[error("Failed to read config value from environment: {0}")]
    Env(#[from] VarError),
    #[error("Failed to read/write config file: {0}")]
    Io(#[from] io::Error),
    #[error("Error (de)serializing config file: {0}")]
    Yaml(#[from] YamlError),
}

#[cfg(test)]
mod test {
    use std::{
        env,
        net::{Ipv4Addr, Ipv6Addr},
        panic::catch_unwind,
        path::PathBuf,
        str::FromStr,
        time::Duration,
    };

    use hyper::Uri;
    use log::LevelFilter;
    use monero::{Address, PrivateKey};
    use secrecy::Secret;

    use super::{Config, DaemonConfig, LoggingConfig, ServerConfig, TlsConfig, WalletConfig};
    use crate::config::{daemon::DaemonLoginConfig, CallbackConfig, DatabaseConfig};

    #[test]
    fn default() {
        let config = Config::default();

        let expected_config = Config {
            external_api: ServerConfig {
                port: 8080,
                ipv4: Ipv4Addr::LOCALHOST,
                ipv6: Some(Ipv6Addr::LOCALHOST),
                token: None,
                tls: None,
                static_dir: PathBuf::from("./server/static/"),
            },
            internal_api: ServerConfig {
                port: 8081,
                ipv4: Ipv4Addr::LOCALHOST,
                ipv6: Some(Ipv6Addr::LOCALHOST),
                token: None,
                tls: Some(TlsConfig {
                    cert: PathBuf::from("./cert/certificate.pem"),
                    key: PathBuf::from("./cert/privatekey.pem"),
                }),
                static_dir: PathBuf::from("./server/static/"),
            },
            callback: CallbackConfig {
                queue_size: 1_000,
                max_retries: Some(50),
            },
            wallet: WalletConfig {
                primary_address: None,
                private_viewkey: None,
                account_index: 0,
                restore_height: None,
            },
            daemon: DaemonConfig {
                url: Uri::from_static("https://xmr-node.cakewallet.com:18081"),
                login: None,
                rpc_timeout: Duration::from_secs(30),
                connection_timeout: Duration::from_secs(20),
            },
            database: DatabaseConfig {
                path: PathBuf::from_str("AcceptXMR_DB/").unwrap(),
                delete_expired: true,
            },
            logging: LoggingConfig {
                verbosity: LevelFilter::Info,
            },
        };

        assert_eq!(config, expected_config);
    }

    #[test]
    fn from_yaml() {
        let yaml = include_str!("../../tests/testdata/config/config_full.yaml");

        let expected_config = expected_config();

        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config, expected_config);
        config.validate();
    }

    #[test]
    fn from_yaml_and_env() {
        let expected_config = expected_config();
        env::set_var(
            "CONFIG_FILE",
            "tests/testdata/config/config_no_secrets.yaml",
        );

        let config_path = Config::get_path();
        let config_without_secrets = Config::from_file(&config_path).unwrap();
        assert_ne!(config_without_secrets, expected_config);
        catch_unwind(|| config_without_secrets.validate())
            .expect_err("config without secrets should be invalid");

        env::set_var("DAEMON_PASSWORD", "supersecretpassword");
        env::set_var(
            "PRIVATE_VIEWKEY",
            "ad2093a5705b9f33e6f0f0c1bc1f5f639c756cdfc168c8f2ac6127ccbdab3a03",
        );
        env::set_var("INTERNAL_API_TOKEN", "supersecrettoken");
        let config = Config::read(&config_path).unwrap();
        assert_eq!(config, expected_config);
        config.validate();
    }

    fn expected_config() -> Config {
        Config {
            external_api: ServerConfig::default(),
            internal_api: ServerConfig {
                port: 8081,
                ipv4: Ipv4Addr::LOCALHOST,
                ipv6: None,
                token: Some(Secret::new("supersecrettoken".to_string())),
                tls: Some(TlsConfig {
                    cert: PathBuf::from_str("/path/to/cert.pem").unwrap(),
                    key: PathBuf::from_str("/path/to/key.pem").unwrap(),
                }),
                static_dir: PathBuf::from("./server/static/"),
            },
            callback: CallbackConfig {
                queue_size: 500,
                max_retries: Some(25),
            },
            wallet: WalletConfig {
                primary_address: Some(Address::from_str("4613YiHLM6JMH4zejMB2zJY5TwQCxL8p65ufw8kBP5yxX9itmuGLqp1dS4tkVoTxjyH3aYhYNrtGHbQzJQP5bFus3KHVdmf").unwrap()),
                private_viewkey: Some(Secret::new(PrivateKey::from_str("ad2093a5705b9f33e6f0f0c1bc1f5f639c756cdfc168c8f2ac6127ccbdab3a03").unwrap().to_string())),
                account_index: 0,
                restore_height: Some(2_947_000),
            },
            daemon: DaemonConfig {
                url: Uri::from_static("https://node.example.com:18081"),
                login: Some(DaemonLoginConfig {
                    username: "pinkpanther".to_string(),
                    password: Some(Secret::new("supersecretpassword".to_string())),
                }),
                rpc_timeout: Duration::from_secs(20),
                connection_timeout: Duration::from_secs(10),
            },
            database: DatabaseConfig {
                path: PathBuf::from_str("server/tests/AcceptXMR_DB/").unwrap(),
                delete_expired: true,
            },
            logging: LoggingConfig {
                verbosity: LevelFilter::Debug,
            },
        }
    }
}
