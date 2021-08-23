#[derive(Debug)]
pub enum Error {
    RpcError(reqwest::Error),
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Self::RpcError(e)
    }
}
