use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sqlx::FromRow;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
impl Serialize for Quality {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Quality {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(Quality::from_str(&s))
    }
}
impl Quality {
    /// Canonical string label stored in SQLite (`quality_profile_items.quality`)
    /// and matched by `parser::evaluate_release`.
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

    /// Returns all quality tiers ordered roughly from highest quality/rank to lowest.
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

impl FromStr for Quality {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Quality::from_str(s))
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QualityProfile {
    pub id: i64,
    pub name: String,
    pub upgrade_allowed: bool,
    pub cutoff_quality: String,
    pub items: Vec<QualityProfileItem>,
}