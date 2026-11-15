//! HTTP behind a trait, so tests need no network.

use std::time::Duration;

use thiserror::Error;

/// Why a fetch failed.
#[derive(Debug, Error)]
pub enum FetchError {
    /// The URL is not HTTPS.
    #[error("refusing a non-HTTPS URL")]
    NotHttps,
    /// The response was larger than the limit.
    #[error("response is larger than {0} bytes")]
    TooLarge(u64),
    /// Transport or status failure.
    #[error("{0}")]
    Network(String),
}

/// Fetch a URL into memory.
pub trait Fetcher {
    /// GET `url` and return the body, failing when it exceeds `limit` bytes.
    fn get(&self, url: &str, limit: u64) -> Result<Vec<u8>, FetchError>;
}

/// The real fetcher: HTTPS only, bounded time and size.
#[derive(Debug, Clone, Copy)]
pub struct UreqFetcher {
    timeout: Duration,
}

impl UreqFetcher {
    /// A fetcher whose whole exchange may take `timeout`.
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        Self { timeout }
    }
}

impl Fetcher for UreqFetcher {
    fn get(&self, url: &str, limit: u64) -> Result<Vec<u8>, FetchError> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(self.timeout))
            .https_only(true)
            .user_agent(concat!("slopvac/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        let mut resp = agent.get(url).call().map_err(|e| match e {
            ureq::Error::RequireHttpsOnly(_) => FetchError::NotHttps,
            other => FetchError::Network(other.to_string()),
        })?;
        let mut buf = Vec::new();
        let n = std::io::Read::read_to_end(
            &mut resp.body_mut().with_config().limit(limit + 1).reader(),
            &mut buf,
        )
        .map_err(|e| FetchError::Network(e.to_string()))?;
        if n as u64 > limit {
            return Err(FetchError::TooLarge(limit));
        }
        Ok(buf)
    }
}
