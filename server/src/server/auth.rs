use std::marker::PhantomData;

use axum::body::HttpBody;
use hyper::http::{header, Request, Response, StatusCode};
use log::{debug, trace};
use secrecy::{ExposeSecret, Secret};
use tower_http::validate_request::ValidateRequest;

pub(crate) struct MaybeBearer<ResBody> {
    token: Option<Secret<String>>,
    _ty: PhantomData<fn() -> ResBody>,
}

impl<ResBody> MaybeBearer<ResBody> {
    pub(crate) fn new(token: Option<Secret<String>>) -> Self {
        Self {
            token,
            _ty: PhantomData,
        }
    }
}

impl<ResBody> Clone for MaybeBearer<ResBody> {
    fn clone(&self) -> Self {
        Self {
            token: self.token.clone(),
            _ty: PhantomData,
        }
    }
}

impl<B, ResBody> ValidateRequest<B> for MaybeBearer<ResBody>
where
    ResBody: HttpBody + Default,
{
    type ResponseBody = ResBody;

    fn validate(&mut self, request: &mut Request<B>) -> Result<(), Response<Self::ResponseBody>> {
        if let Some(token) = &self.token {
            match request.headers().get(header::AUTHORIZATION) {
                Some(actual) if actual == &format!("Bearer {}", token.expose_secret()) => Ok(()),
                Some(_) => {
                    debug!("Authentication denied. Bearer auth token mismatch.");
                    let mut res = Response::new(ResBody::default());
                    *res.status_mut() = StatusCode::UNAUTHORIZED;
                    Err(res)
                }
                None => {
                    debug!("Authentication denied. Bearer auth token missing.");
                    let mut res = Response::new(ResBody::default());
                    *res.status_mut() = StatusCode::UNAUTHORIZED;
                    Err(res)
                }
            }
        } else {
            trace!("Bearer auth token not set. Not enforcing bearer auth.");
            Ok(())
        }
    }
}
