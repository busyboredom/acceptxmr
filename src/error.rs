use std::fmt;

#[derive(Debug)]
pub enum Error {
    RpcError(reqwest::Error),
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Self::RpcError(e)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::RpcError(reqwest_error) => write!(f, "RPC request error: {}", reqwest_error),
        }
    }
}
