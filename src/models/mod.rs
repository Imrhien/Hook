use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Quality {
    Unknown,
    SDTV,
    WEBDL480p,
    HDTV720p,
    WEBDL720p,
    Bluray720p,
    HDTV1080p,
    WEBDL1080p,
    Bluray1080p,
    WEBDL2160p,
    Bluray2160p,
    Remux1080p,
    Remux2160p,
}

impl Quality {
    pub fn as_str(&self) -> &'static str {
        match self {
            Quality::Unknown => "Unknown",
            Quality::SDTV => "SDTV",
            Quality::WEBDL480p => "WEBDL-480p",
            Quality::HDTV720p => "HDTV-720p",
            Quality::WEBDL720p => "WEBDL-720p",
            Quality::Bluray720p => "Bluray-720p",
            Quality::HDTV1080p => "HDTV-1080p",
            Quality::WEBDL1080p => "WEBDL-1080p",
            Quality::Bluray1080p => "Bluray-1080p",
            Quality::WEBDL2160p => "WEBDL-2160p",
            Quality::Bluray2160p => "Bluray-2160p",
            Quality::Remux1080p => "Remux-1080p",
            Quality::Remux2160p => "Remux-2160p",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "SDTV" => Quality::SDTV,
            "WEBDL-480p" => Quality::WEBDL480p,
            "HDTV-720p" => Quality::HDTV720p,
            "WEBDL-720p" => Quality::WEBDL720p,
            "Bluray-720p" => Quality::Bluray720p,
            "HDTV-1080p" => Quality::HDTV1080p,
            "WEBDL-1080p" => Quality::WEBDL1080p,
            "Bluray-1080p" => Quality::Bluray1080p,
            "WEBDL-2160p" => Quality::WEBDL2160p,
            "Bluray-2160p" => Quality::Bluray2160p,
            "Remux-1080p" => Quality::Remux1080p,
            "Remux-2160p" => Quality::Remux2160p,
            _ => Quality::Unknown,
        }
    }

    /// Useful for populating the UI builder with all available quality tiers
    pub fn all_default_order() -> Vec<Quality> {
        vec![
            Quality::Remux2160p,
            Quality::Bluray2160p,
            Quality::WEBDL2160p,
            Quality::Remux1080p,
            Quality::Bluray1080p,
            Quality::WEBDL1080p,
            Quality::HDTV1080p,
            Quality::Bluray720p,
            Quality::WEBDL720p,
            Quality::HDTV720p,
            Quality::WEBDL480p,
            Quality::SDTV,
            Quality::Unknown,
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct QualityProfileItem {
    pub quality: String,
    pub allowed: bool,
    pub rank: i32,
    pub max_movie_size_mb: Option<i64>,
    pub max_episode_size_mb: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityProfile {
    pub id: i64,
    pub name: String,
    pub upgrade_allowed: bool,
    pub cutoff_quality: String,
    pub items: Vec<QualityProfileItem>,
}
