use std::{
    fs::{create_dir_all, File},
    io::{BufReader, Write},
};

use log::warn;
use rcgen::generate_simple_self_signed;
use rustls::{Certificate, PrivateKey as RustlsPrivateKey, ServerConfig as RustlsServerConfig};
use rustls_pemfile::{certs, pkcs8_private_keys};

use crate::config::TlsConfig;

// Attempt to load TLS, falling back on self-signed certificate if necessary.
pub fn prepare_tls(config: &TlsConfig) -> RustlsServerConfig {
    // Init server config builder with safe defaults.
    let rustls_config = RustlsServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth();

    let (cert_chain, mut keys) = match (config.cert.try_exists(), config.key.try_exists()) {
        (Ok(true), Ok(true)) => load_tls(config),
        (Ok(false), Ok(false)) => {
            warn!("TLS certificate not found. Generating self-signed certificate instead.");
            generate_tls(config);
            load_tls(config)
        }
        (Ok(true), Ok(false)) => panic!("TLS certificate found, but private key is missing"),
        (Ok(false), Ok(true)) => panic!("TLS privatekey found, but certificate is missing"),
        (Err(e), _) => panic!("failed to check for TLS certificate existance: {e}"),
        (_, Err(e)) => panic!("failed to check for TLS private key existance: {e}"),
    };

    // Exit if no keys could be parsed
    if keys.is_empty() {
        eprintln!("Could not locate PKCS 8 private keys.");
        std::process::exit(1);
    }

    rustls_config
        .with_single_cert(cert_chain, keys.remove(0))
        .unwrap()
}

fn load_tls(config: &TlsConfig) -> (Vec<Certificate>, Vec<RustlsPrivateKey>) {
    let cert_file =
        &mut BufReader::new(File::open(&config.cert).expect("failed to load TLS certificate file"));
    let key_file =
        &mut BufReader::new(File::open(&config.key).expect("failed to load TLS key file"));

    // Convert files to key/cert objects
    let cert_chain = certs(cert_file)
        .unwrap()
        .into_iter()
        .map(Certificate)
        .collect();
    let keys: Vec<RustlsPrivateKey> = pkcs8_private_keys(key_file)
        .unwrap()
        .into_iter()
        .map(RustlsPrivateKey)
        .collect();

    (cert_chain, keys)
}

fn generate_tls(config: &TlsConfig) {
    let cert = generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    if let Some(path) = config.cert.parent() {
        create_dir_all(path).expect("failed to create TLS certificate directory");
    }
    if let Some(path) = config.key.parent() {
        create_dir_all(path).expect("failed to create TLS private key directory");
    }
    let mut cert_file = File::create(&config.cert).expect("failed to create TLS certificate");
    let mut key_file = File::create(&config.key).expect("failed to create TLS private key");
    write!(cert_file, "{}", cert.serialize_pem().unwrap())
        .expect("failed to write self-signed TLS certificate");
    write!(key_file, "{}", cert.serialize_private_key_pem())
        .expect("failed to write self-signed TLS private key");
}
