use std::{
    cmp::Ordering,
    string::ToString,
    sync::{
        atomic::{self, AtomicU32},
        Arc, Mutex, PoisonError,
    },
};

use hyper::{
    header::{HeaderValue, WWW_AUTHENTICATE},
    http::uri::PathAndQuery,
    Method, Response, Uri,
};
use log::trace;
use md5::{Digest, Md5};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha12Rng;
use strum::Display;
use thiserror::Error;

/// Digest authentication info.
#[derive(Debug, Clone)]
pub(crate) struct AuthInfo {
    username: String,
    password: String,
    pub counter: Arc<AtomicU32>,
    pub last_auth_params: Arc<Mutex<Option<AuthParams>>>,
    rng: ChaCha12Rng,
}

impl AuthInfo {
    pub fn new(username: String, password: String, seed: Option<u64>) -> AuthInfo {
        // If a seed is supplied, seed the random number generator with it.
        let mut rng = ChaCha12Rng::from_entropy();
        if let Some(s) = seed {
            rng = ChaCha12Rng::seed_from_u64(s);
        }
        AuthInfo {
            username,
            password,
            counter: Arc::new(AtomicU32::new(1)),
            last_auth_params: Arc::new(Mutex::new(None)),
            rng,
        }
    }

    /// Build `AUTHORIZATION` header value by re-using most recent auth parameters.
    ///
    /// Returns `None` if no recent parameters are available.
    #[allow(clippy::similar_names)]
    pub fn authenticate(
        &mut self,
        uri: &Uri,
        method: &Method,
    ) -> Result<Option<HeaderValue>, AuthError> {
        let maybe_auth_params = &*self
            .last_auth_params
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let Some(auth_params) = maybe_auth_params else {
            return Ok(None)
        };
        let mut cnonce_bytes: [u8; 16] = [0; 16]; // 128 bits
        self.rng.fill(&mut cnonce_bytes[..]);

        let path_and_query = uri
            .path_and_query()
            .map_or(uri.path(), PathAndQuery::as_str);
        let nc = format!("{:08x}", self.counter.load(atomic::Ordering::Relaxed));
        let qop = auth_params.qop.iter().max().ok_or(AuthError::Unsupported)?;
        let nonce = &auth_params.nonce;
        let realm = &auth_params.realm;
        let opaque = &auth_params.opaque;
        let cnonce = hex::encode(cnonce_bytes);
        let algorithm = &auth_params.algorithm;

        trace!(
            "Performing digest authentication with qop: {}, algorithm: {}, nc: {}.",
            qop,
            algorithm,
            nc
        );

        let ha1_input = format!("{}:{}:{}", &self.username, realm, &self.password);
        let mut ha1 = md5_str(ha1_input);
        if algorithm.is_sess() {
            ha1 = md5_str(format!("{ha1}:{nonce}:{cnonce}"));
        }
        let ha2_input = format!("{method}:{path_and_query}");
        let ha2 = md5_str(ha2_input);
        let response_input = format!("{ha1}:{nonce}:{nc}:{cnonce}:{qop}:{ha2}");
        let response = md5_str(response_input);

        let mut auth_header = format!(
            "Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\", qop={}, nc={}, cnonce=\"{}\", response=\"{}\", algorithm={}",
            self.username,
            realm,
            nonce,
            path_and_query,
            qop,
            nc,
            cnonce,
            response,
            algorithm,
        );
        if let Some(opaque_val) = opaque {
            let opaque_str = format!(", opaque={opaque_val}");
            auth_header.push_str(&opaque_str);
        }

        self.counter.fetch_add(1, atomic::Ordering::Relaxed);
        Ok(Some(HeaderValue::from_str(&auth_header)?))
    }

    /// Build `AUTHORIZATION` header value given a `Response` containing `WWW-AUTHENTICATE`
    /// header(s).
    pub fn authenticate_with_resp<T>(
        &mut self,
        response: &Response<T>,
        uri: &Uri,
        method: &Method,
    ) -> Result<HeaderValue, AuthError> {
        let headers = response.headers();
        let authenticate_headers = headers
            .get_all(WWW_AUTHENTICATE)
            .into_iter()
            .map(|h| {
                h.to_str().map_err(|_| {
                    AuthError::InvalidHeader("header could not be parsed to String".to_string())
                })
            })
            .collect::<Result<Vec<&str>, AuthError>>()?;
        let digest_headers = authenticate_headers
            .into_iter()
            .filter_map(|h| h.strip_prefix("Digest "));
        let mut auth_choices = digest_headers
            .map(parse_header)
            .collect::<Result<Vec<AuthParams>, AuthError>>()?;
        auth_choices.sort_unstable(); // AuthParams Ord implementation attempts to select the best available authentication options.
        let auth_params = auth_choices.last().ok_or(AuthError::Unsupported)?;

        *self
            .last_auth_params
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(auth_params.clone());
        self.counter.store(1, atomic::Ordering::Relaxed);
        self.authenticate(uri, method)
            .transpose()
            .ok_or(AuthError::Unsupported)?
    }
}

fn parse_header(header: &str) -> Result<AuthParams, AuthError> {
    let str_params = split_header(header);

    let realm = find_string_value(&str_params, "realm").unwrap_or_default();
    let qop = find_string_value(&str_params, "qop")
        .unwrap_or_default()
        .split(',')
        .map(|s| match s.trim() {
            "" | "auth" => Ok(Qop::Auth),
            q => Err(AuthError::InvalidHeader(format!(
                "unknown QoP directive: {q}"
            ))),
        })
        .collect::<Result<Vec<Qop>, AuthError>>()?;
    let algorithm = match find_string_value(&str_params, "algorithm")
        .unwrap_or_default()
        .trim()
    {
        "" | "MD5" => Algorithm::Md5,
        "MD5-sess" => Algorithm::Md5Sess,
        a => return Err(AuthError::InvalidHeader(format!("unknown algorithm: {a}"))),
    };
    let nonce = find_string_value(&str_params, "nonce")
        .ok_or_else(|| AuthError::InvalidHeader("no nonce provided".to_string()))?;
    let opaque = find_string_value(&str_params, "opaque");

    Ok(AuthParams {
        realm,
        qop,
        algorithm,
        nonce,
        opaque,
    })
}

fn split_header(header_str: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut last_split = 0;
    let mut char_iterator = header_str.char_indices().peekable();
    while let Some((i, c)) = char_iterator.next() {
        match c {
            '\'' => in_single_quote = !in_single_quote,
            '\"' => in_double_quote = !in_double_quote,
            ',' => {
                if !in_single_quote && !in_double_quote {
                    parts.push(header_str[last_split..i].trim_start_matches(',').trim());
                    last_split = i;
                }
            }
            _ => {}
        }
        if char_iterator.peek().is_none() {
            parts.push(header_str[last_split..].trim_start_matches(',').trim());
        }
    }
    parts
}

fn find_string_value(parts: &Vec<&str>, field: &'static str) -> Option<String> {
    for &p in parts {
        if p.starts_with(field) {
            let formatted = format!("{field}=");
            return Some(
                p.replace(&formatted, "")
                    .trim_start_matches('\"')
                    .trim_end_matches('\"')
                    .to_string(),
            );
        }
    }
    None
}

fn md5_str(input: String) -> String {
    let mut digest = Md5::new();
    let input_bytes = input.into_bytes();
    digest.update(&input_bytes);

    hex::encode(digest.finalize())
}

/// Parameters that may appear in WWW-AUTHENTICATE header.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct AuthParams {
    realm: String,
    qop: Vec<Qop>,
    algorithm: Algorithm,
    nonce: String,
    opaque: Option<String>,
}

impl Ord for AuthParams {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.qop.iter().max().cmp(&other.qop.iter().max()) {
            Ordering::Equal => self.algorithm.cmp(&other.algorithm),
            ord => ord,
        }
    }
}

impl PartialOrd for AuthParams {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Quality of protection directives in order of preference with the best option last.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Display, Debug)]
#[strum(serialize_all = "kebab-case")]
enum Qop {
    Auth,
}

/// Digest algorithms in order of preference with the best option last.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Display, Debug)]
enum Algorithm {
    #[strum(serialize = "MD5")]
    Md5,
    #[strum(serialize = "MD5-sess")]
    Md5Sess,
}

impl Algorithm {
    fn is_sess(self) -> bool {
        matches!(self, Algorithm::Md5Sess)
    }
}

#[allow(clippy::module_name_repetitions)]
#[derive(Error, Debug)]
pub enum AuthError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("invalid WWW-AUTHENTICATE header: {0}")]
    InvalidHeader(String),
    #[error("failed to constuct AUTHORIZATION header")]
    HeaderConstruction(#[from] hyper::header::InvalidHeaderValue),
    #[error("no supported authentication method")]
    Unsupported,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use hyper::{
        header::{HeaderValue, WWW_AUTHENTICATE},
        Method, Response, Uri,
    };

    use super::{parse_header, Algorithm, AuthInfo, AuthParams, Qop};

    #[test]
    fn test_parse_header() {
        let header = "Digest qop=\"auth\",algorithm=MD5-sess,realm=\"monero-rpc\", nonce=\"kVmRYw+lSQ80tTK3zj6/aA==\", stale=false";
        let auth_params = parse_header(header).expect("failed to parse header");
        let expected_auth_params = AuthParams {
            realm: "monero-rpc".to_string(),
            qop: vec![Qop::Auth],
            algorithm: Algorithm::Md5Sess,
            nonce: "kVmRYw+lSQ80tTK3zj6/aA==".to_string(),
            opaque: None,
        };
        assert_eq!(auth_params, expected_auth_params);
    }

    #[test]
    fn md5_auth() {
        let mut auth_info = AuthInfo::new(
            "test user".to_string(),
            "test password".to_string(),
            Some(1),
        );
        let response = Response::builder()
            .header(WWW_AUTHENTICATE, "Digest qop=\"auth\",algorithm=MD5,realm=\"monero-rpc\",nonce=\"JmNFnqfRJdOr/vFZ2CpDQg==\",stale=false")
            .body(()).expect("wailed to build WWW_AUTHENTICATE response");
        let authorization_header = auth_info
            .authenticate_with_resp(
                &response,
                &Uri::from_static("https://busyboredom.com:18089/json_rpc"),
                &Method::POST,
            )
            .expect("failed to create AUTHORIZATION header");
        assert_eq!(authorization_header, HeaderValue::from_static("Digest username=\"test user\", realm=\"monero-rpc\", nonce=\"JmNFnqfRJdOr/vFZ2CpDQg==\", uri=\"/json_rpc\", qop=auth, nc=00000001, cnonce=\"611830d3641a68f94a690dcc25d1f4b0\", response=\"af7810760defeed31054108ed35d400d\", algorithm=MD5"));
    }

    #[test]
    fn md5_sess_auth() {
        let mut auth_info = AuthInfo::new(
            "test user".to_string(),
            "test password".to_string(),
            Some(1),
        );
        let response = Response::builder()
            .header(WWW_AUTHENTICATE, "Digest qop=\"auth\",algorithm=MD5,realm=\"monero-rpc\",nonce=\"JmNFnqfRJdOr/vFZ2CpDQg==\",stale=false")
            .header(WWW_AUTHENTICATE, "Digest qop=\"auth\",algorithm=MD5-sess,realm=\"monero-rpc\",nonce=\"JmNFnqfRJdOr/vFZ2CpDQg==\",stale=false")
            .body(()).expect("wailed to build WWW_AUTHENTICATE response");
        let authorization_header = auth_info
            .authenticate_with_resp(
                &response,
                &Uri::from_static("https://busyboredom.com:18089/json_rpc"),
                &Method::POST,
            )
            .expect("failed to create AUTHORIZATION header");
        assert_eq!(authorization_header, HeaderValue::from_static("Digest username=\"test user\", realm=\"monero-rpc\", nonce=\"JmNFnqfRJdOr/vFZ2CpDQg==\", uri=\"/json_rpc\", qop=auth, nc=00000001, cnonce=\"611830d3641a68f94a690dcc25d1f4b0\", response=\"b0a351e720384a0160042ff2898ab24a\", algorithm=MD5-sess"));
    }

    #[test]
    fn auth_with_opaque() {
        let mut auth_info = AuthInfo::new(
            "test user".to_string(),
            "test password".to_string(),
            Some(1),
        );
        let response = Response::builder()
            .header(WWW_AUTHENTICATE, "Digest qop=\"auth\",algorithm=MD5,realm=\"monero-rpc\",nonce=\"JmNFnqfRJdOr/vFZ2CpDQg==\",stale=false")
            .header(WWW_AUTHENTICATE, "Digest qop=\"auth\",algorithm=MD5-sess,realm=\"monero-rpc\",nonce=\"JmNFnqfRJdOr/vFZ2CpDQg==\",stale=false,opaque=5PCCDS2k5PCCDS2k")
            .body(()).expect("wailed to build WWW_AUTHENTICATE response");
        let authorization_header = auth_info
            .authenticate_with_resp(
                &response,
                &Uri::from_static("https://busyboredom.com:18089/json_rpc"),
                &Method::POST,
            )
            .expect("failed to create AUTHORIZATION header");
        assert_eq!(authorization_header, HeaderValue::from_static("Digest username=\"test user\", realm=\"monero-rpc\", nonce=\"JmNFnqfRJdOr/vFZ2CpDQg==\", uri=\"/json_rpc\", qop=auth, nc=00000001, cnonce=\"611830d3641a68f94a690dcc25d1f4b0\", response=\"b0a351e720384a0160042ff2898ab24a\", algorithm=MD5-sess, opaque=5PCCDS2k5PCCDS2k"));
    }
}
