pub mod torrent;

use async_trait::async_trait;
use serde::Serialize;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Serialize)]
pub struct DownloadStatus {
    pub id: String,
    pub name: String,
    pub progress_pct: f64,
    pub download_speed_bps: u64,
    pub state: String, // "initializing", "downloading", "completed", "paused", "error"
}

#[async_trait]
pub trait DownloadClient: Send + Sync {
    async fn add_release(&self, url: &str) -> anyhow::Result<String>;
    async fn list_downloads(&self) -> anyhow::Result<Vec<DownloadStatus>>;
    async fn remove_download(&self, id: &str, delete_files: bool) -> anyhow::Result<()>;

    /// Takes the completion notification channel once if supported by the client.
    fn take_completion_receiver(&self) -> Option<mpsc::UnboundedReceiver<String>> {
        None
    }
}