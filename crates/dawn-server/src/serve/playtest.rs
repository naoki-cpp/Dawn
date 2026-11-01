//! Configuration for the disposable single-node Render playtest.

use crate::ws_server::InviteAccess;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

#[derive(Debug)]
pub(super) struct PlaytestConfig {
    pub address: SocketAddr,
    pub access: Option<Arc<InviteAccess>>,
}

impl PlaytestConfig {
    pub fn from_environment() -> anyhow::Result<Self> {
        let port = std::env::var("PORT").ok();
        let tokens = std::env::var("DAWN_INVITE_TOKENS").ok();
        Self::parse(port.as_deref(), tokens.as_deref())
    }

    fn parse(port: Option<&str>, tokens: Option<&str>) -> anyhow::Result<Self> {
        match port {
            None => Ok(Self {
                address: (Ipv4Addr::LOCALHOST, 7878).into(),
                access: tokens.map(InviteAccess::parse).transpose()?.map(Arc::new),
            }),
            Some(port) => {
                let port: u16 = port
                    .parse()
                    .map_err(|_| anyhow::anyhow!("PORT must be 1..=65535"))?;
                anyhow::ensure!(port != 0, "PORT must be 1..=65535");
                let access = InviteAccess::parse(tokens.ok_or_else(|| {
                    anyhow::anyhow!("DAWN_INVITE_TOKENS is required when PORT is set")
                })?)?;
                Ok(Self {
                    address: (Ipv4Addr::UNSPECIFIED, port).into(),
                    access: Some(Arc::new(access)),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_local_and_cloud_requires_invites_and_a_valid_port() {
        assert_eq!(
            PlaytestConfig::parse(None, None)
                .unwrap()
                .address
                .to_string(),
            "127.0.0.1:7878"
        );
        assert!(PlaytestConfig::parse(Some("10000"), None).is_err());
        let tokens = format!("[\"{}\"]", "a".repeat(64));
        for port in ["0", "65536", "-1", "", "invalid"] {
            assert!(PlaytestConfig::parse(Some(port), Some(&tokens)).is_err());
        }
        let config = PlaytestConfig::parse(Some("10000"), Some(&tokens)).unwrap();
        assert_eq!(config.address.to_string(), "0.0.0.0:10000");
        assert!(config.access.is_some());
    }
}
