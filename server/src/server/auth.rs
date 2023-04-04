use actix_web::{dev::ServiceRequest, web, Error as ActixError};
use actix_web_httpauth::extractors::{
    bearer::{self, BearerAuth},
    AuthenticationError,
};
use log::{debug, trace, warn};
use secrecy::ExposeSecret;

use crate::config::ServerConfig;

#[allow(clippy::unused_async)]
pub async fn bearer_auth_validator(
    req: ServiceRequest,
    credentials: BearerAuth,
) -> Result<ServiceRequest, (ActixError, ServiceRequest)> {
    if let Some(server_config) = req.app_data::<web::Data<ServerConfig>>() {
        if let Some(expected_token) = &server_config.token {
            if credentials.token() != expected_token.expose_secret() {
                let bearer_config = req
                    .app_data::<bearer::Config>()
                    .cloned()
                    .unwrap_or_default();

                debug!("Authentication denied. Bearer auth token mismatch.");
                return Err((AuthenticationError::from(bearer_config).into(), req));
            }
        } else {
            trace!("Bearer auth token not set. Not enforcing bearer auth.");
        }
    } else {
        warn!("No server configuration found while attempting to evaluate bearer auth policy.");
    }
    Ok(req)
}
