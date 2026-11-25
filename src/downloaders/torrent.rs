use async_trait::async_trait;
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, Session, SessionOptions,
    SessionPersistenceConfig,
};
use reqwest::{redirect::Policy, Client};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use super::{DownloadClient, DownloadStatus};
use crate::vpn::UserspaceVpnManager;

pub struct RqbitDownloader {
    session: Arc<Session>,
    http: Client,
    vpn_enabled: Arc<AtomicBool>,
    vpn_manager: UserspaceVpnManager,
}

impl RqbitDownloader {
    pub async fn new(
        download_dir: PathBuf,
        vpn_enabled: bool,
        interface_name: Option<String>,
        vpn_manager: UserspaceVpnManager,
    ) -> anyhow::Result<Self> {
        tokio::fs::create_dir_all(&download_dir).await?;

        let session_dir = download_dir
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("rqbit-session");
        tokio::fs::create_dir_all(&session_dir).await?;

        let mut opts = SessionOptions {
            fastresume: true,
            persistence: Some(SessionPersistenceConfig::Json {
                folder: Some(session_dir),
            }),
            ..Default::default()
        };

        if vpn_enabled {
            if let Some(iface) = interface_name {
                println!("[VPN] Binding BitTorrent sockets strictly to '{}'", iface);
                opts.bind_device_name = Some(iface);
            }
        }

        let session = Session::new_with_opts(download_dir, opts).await?;

        let http = Client::builder()
            .user_agent("Hook/0.1.0")
            .redirect(Policy::none())
            .build()?;

        Ok(Self {
            session,
            http,
            vpn_enabled: Arc::new(AtomicBool::new(vpn_enabled)),
            vpn_manager,
        })
    }
}

#[async_trait]
impl DownloadClient for RqbitDownloader {
    async fn add_release(&self, url: &str) -> anyhow::Result<String> {
        // Strict Kill Switch: refuse to start any torrent if VPN is enabled but tunnel is down
        if self.vpn_enabled.load(Ordering::SeqCst) && !self.vpn_manager.is_connected() {
            anyhow::bail!(
                "VPN Kill Switch triggered: WireGuard tunnel is disconnected. Refusing to start torrent."
            );
        }

        let mut current_url = url.trim().to_string();

        let add_torrent = 'resolve: {
            for _ in 0..5 {
                if current_url.starts_with("magnet:") {
                    break 'resolve AddTorrent::from_url(&current_url);
                }

                let resp = self.http.get(&current_url).send().await?;
                let status = resp.status();

                if status.is_redirection() {
                    if let Some(loc) = resp.headers().get("location").and_then(|v| v.to_str().ok()) {
                        current_url = loc.to_string();
                        continue;
                    }
                }

                if !status.is_success() {
                    anyhow::bail!("Torrent URL returned HTTP {} for {}", status, current_url);
                }

                let bytes = resp.bytes().await?;
                if bytes.starts_with(b"<!DOCTYPE") || bytes.starts_with(b"<html") {
                    anyhow::bail!("URL returned an HTML webpage instead of a .torrent file");
                }

                break 'resolve AddTorrent::from_bytes(bytes);
            }

            anyhow::bail!("Too many redirects resolving torrent URL")
        };

        let opts = AddTorrentOptions {
            overwrite: true,
            ..Default::default()
        };

        let response = self.session.add_torrent(add_torrent, Some(opts)).await?;
        let handle = match response {
            AddTorrentResponse::Added(_, handle) => handle,
            AddTorrentResponse::AlreadyManaged(_, handle) => handle,
            AddTorrentResponse::ListOnly(_) => {
                anyhow::bail!("Torrent added in list-only mode unexpectedly")
            }
        };

        Ok(handle.info_hash().as_string())
    }

    async fn list_downloads(&self) -> anyhow::Result<Vec<DownloadStatus>> {
        let results = RefCell::new(Vec::new());

        self.session.with_torrents(|torrents| {
            let mut list = results.borrow_mut();
            for (_, handle) in torrents {
                let stats = handle.stats();
                let total = stats.total_bytes.max(1) as f64;
                let progress_pct = ((stats.progress_bytes as f64 / total) * 100.0).min(100.0);

                let speed_bps = stats
                    .live
                    .as_ref()
                    .map(|l| (l.download_speed.mbps * 125_000.0) as u64)
                    .unwrap_or(0);

                let state = if stats.finished {
                    "completed"
                } else if stats.error.is_some() {
                    "error"
                } else {
                    "downloading"
                };

                list.push(DownloadStatus {
                    id: handle.info_hash().as_string(),
                    name: handle
                        .name()
                        .unwrap_or_else(|| "Fetching metadata...".to_string()),
                    progress_pct,
                    download_speed_bps: speed_bps,
                    state: state.to_string(),
                });
            }
        });

        Ok(results.into_inner())
    }

    async fn remove_download(&self, id: &str, delete_files: bool) -> anyhow::Result<()> {
        let hash = librqbit::api::TorrentIdOrHash::parse(id)
            .map_err(|e| anyhow::anyhow!("Invalid torrent hash '{}': {:?}", id, e))?;

        self.session.delete(hash, delete_files).await?;
        Ok(())
    }
}