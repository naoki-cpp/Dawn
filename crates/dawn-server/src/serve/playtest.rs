//! Shared ingress and disposable storage for single-process Render playtests.

use crate::ws_server::InviteAccess;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use tokio::sync::mpsc;

use super::market::MarketRuntime;
use crate::ws_server::{HandshakeRequest, WsServer};

const HANDSHAKE_LIMIT: usize = 8;

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

    /// Keep the directory guard alive until after the Market connection closes.
    pub fn open_market(&self) -> anyhow::Result<(Option<tempfile::TempDir>, MarketRuntime)> {
        let directory = self
            .address
            .ip()
            .is_unspecified()
            .then(tempfile::tempdir)
            .transpose()?;
        let path = directory.as_ref().map_or_else(
            || std::path::PathBuf::from("data/market.sqlite"),
            |directory| directory.path().join("market.sqlite"),
        );
        let market = MarketRuntime::open(&path.to_string_lossy())?;
        Ok((directory, market))
    }

    pub fn accept_requests(&self, server: WsServer) -> mpsc::Receiver<HandshakeRequest> {
        let (tx, rx) = mpsc::channel(HANDSHAKE_LIMIT);
        let access = self.access.clone();
        tokio::spawn(async move {
            let handshakes = Arc::new(tokio::sync::Semaphore::new(HANDSHAKE_LIMIT));
            while !tx.is_closed() {
                if let Some((stream, addr)) = server.try_accept_raw().await {
                    let Ok(permit) = handshakes.clone().try_acquire_owned() else {
                        drop(stream);
                        continue;
                    };
                    let access = access.clone();
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        let _permit = permit;
                        let result = match access.as_deref() {
                            Some(access) => {
                                WsServer::accept_invited_handshake_request(stream, addr, access)
                                    .await
                            }
                            None => WsServer::accept_handshake_request(stream, addr).await,
                        };
                        match result {
                            Ok(request) => {
                                let _ = tx.send(request).await;
                            }
                            Err(error) if access.is_none() => {
                                eprintln!("[Server] handshake request failed: {error}")
                            }
                            // Public denials must not create an anonymous log-flood path.
                            Err(_) => {}
                        }
                    });
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        });
        rx
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
    fn cloud_market_is_fresh_per_process_and_keeps_a_directory_guard() {
        let tokens = format!("[\"{}\"]", "a".repeat(64));
        let config = PlaytestConfig::parse(Some("10000"), Some(&tokens)).unwrap();
        let (first_directory, first_market) = config.open_market().unwrap();
        let (second_directory, second_market) = config.open_market().unwrap();
        let first = first_directory.unwrap();
        let second = second_directory.unwrap();
        assert_ne!(first.path(), second.path());
        let path = first.path().join("market.sqlite");
        assert!(path.exists());
        drop(first_market);
        drop(first);
        assert!(!path.exists());
        drop(second_market);
        drop(second);
    }

    #[tokio::test]
    async fn both_serve_modes_share_a_bounded_handshake_queue() {
        let config = PlaytestConfig::parse(None, None).unwrap();
        let server = WsServer::bind("127.0.0.1:0").await.unwrap();
        let requests = config.accept_requests(server);
        assert_eq!(requests.max_capacity(), 8);
    }

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
