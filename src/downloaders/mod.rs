pub mod torrent;

use async_trait::async_trait;
use serde::Serialize;

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
    /// Starts a download from a magnet URI or .torrent/.nzb URL and returns its unique ID (e.g., info_hash)
    async fn add_release(&self, url: &str) -> anyhow::Result<String>;

    /// Returns live progress for all active downloads in this client
    async fn list_downloads(&self) -> anyhow::Result<Vec<DownloadStatus>>;

    /// Removes a download (and optionally deletes its files from disk)
    async fn remove_download(&self, id: &str, delete_files: bool) -> anyhow::Result<()>;
}
