use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct IndexerConfig {
    pub id: i64,
    pub name: String,
    pub url: String,
    pub api_key: String, // "apibay", "yts", or "eztv"
    pub enabled: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct ReleaseCandidate {
    pub indexer_name: String,
    pub title: String,
    pub download_url: String, // magnet:?xt=...
    pub size_bytes: u64,
    pub seeders: u32,
    pub quality: String,
    pub score: i32,
}

const PUBLIC_TRACKERS: &[&str] = &[
    "udp://tracker.opentrackr.org:1337/announce",
    "udp://open.stealth.si:80/announce",
    "udp://tracker.torrent.eu.org:451/announce",
    "udp://tracker.bittor.pw:1337/announce",
    "udp://public.popcorn-tracker.org:6969/announce",
];

fn build_magnet(info_hash: &str, name: &str) -> String {
    let encoded_name: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '+' })
        .collect();
    let trackers: String = PUBLIC_TRACKERS
        .iter()
        .map(|t| format!("&tr={}", t))
        .collect();

    format!(
        "magnet:?xt=urn:btih:{}&dn={}{}",
        info_hash.trim(),
        encoded_name,
        trackers
    )
}

fn score_release(title: &str, seeders: u32) -> (String, i32) {
    let lower = title.to_lowercase();
    let (quality, mut score) = if lower.contains("2160p") || lower.contains("4k") {
        ("2160p".to_string(), 85)
    } else if lower.contains("1080p") {
        ("1080p".to_string(), 100)
    } else if lower.contains("720p") {
        ("720p".to_string(), 65)
    } else {
        ("SD".to_string(), 35)
    };

    if lower.contains("bluray") || lower.contains("remux") {
        score += 20;
    } else if lower.contains("web-dl") || lower.contains("webrip") || lower.contains("web") {
        score += 15;
    }

    let bad_re = Regex::new(r"(?i)\b(cam|ts|telesync|hdcam)\b").unwrap();
    if bad_re.is_match(&lower) {
        score -= 500;
    }

    score += (seeders.min(50)) as i32;
    (quality, score)
}

// ==========================================
// Provider 1: Apibay / ThePirateBay (Movies & TV)
// ==========================================
#[derive(Deserialize)]
struct ApibayItem {
    name: String,
    info_hash: String,
    seeders: String,
    size: String,
}

async fn search_apibay(
    http: &Client,
    indexer: &IndexerConfig,
    query: &str,
) -> anyhow::Result<Vec<ReleaseCandidate>> {
    let base = indexer.url.trim_end_matches('/');
    let url = format!("{}/q.php", base);

    // cat=200 searches all Video categories (Movies + TV)
    let items: Vec<ApibayItem> = http
        .get(&url)
        .query(&[("q", query), ("cat", "200")])
        .send()
        .await?
        .json()
        .await?;

    let mut results = Vec::new();
    for item in items {
        // Apibay returns a dummy "No results returned" item with all-zero hash when empty
        if item.info_hash == "0000000000000000000000000000000000000000" || item.info_hash.is_empty()
        {
            continue;
        }

        let seeders: u32 = item.seeders.parse().unwrap_or(0);
        if seeders == 0 {
            continue;
        }

        let size_bytes: u64 = item.size.parse().unwrap_or(0);
        let (quality, score) = score_release(&item.name, seeders);

        results.push(ReleaseCandidate {
            indexer_name: indexer.name.clone(),
            download_url: build_magnet(&item.info_hash, &item.name),
            title: item.name,
            size_bytes,
            seeders,
            quality,
            score,
        });
    }

    Ok(results)
}

// ==========================================
// Provider 2: YTS (Movies)
// ==========================================
#[derive(Deserialize)]
struct YtsResponse {
    data: Option<YtsData>,
}

#[derive(Deserialize)]
struct YtsData {
    #[serde(default)]
    movies: Vec<YtsMovie>,
}

#[derive(Deserialize)]
struct YtsMovie {
    title_long: String,
    #[serde(default)]
    torrents: Vec<YtsTorrent>,
}

#[derive(Deserialize)]
struct YtsTorrent {
    hash: String,
    quality: String,
    #[serde(rename = "type", default)]
    rip_type: String,
    seeds: u32,
    size_bytes: Option<u64>,
}

async fn search_yts(
    http: &Client,
    indexer: &IndexerConfig,
    query: &str,
) -> anyhow::Result<Vec<ReleaseCandidate>> {
    let base = indexer.url.trim_end_matches('/');
    let url = format!("{}/api/v2/list_movies.json", base);

    let resp: YtsResponse = http
        .get(&url)
        .query(&[("query_term", query), ("limit", "15")])
        .send()
        .await?
        .json()
        .await?;

    let mut results = Vec::new();
    if let Some(data) = resp.data {
        for movie in data.movies {
            for t in movie.torrents {
                if t.seeds == 0 {
                    continue;
                }
                let release_title = format!("{} [{}] [{}]", movie.title_long, t.quality, t.rip_type);
                let (quality, score) = score_release(&release_title, t.seeds);

                results.push(ReleaseCandidate {
                    indexer_name: indexer.name.clone(),
                    title: release_title.clone(),
                    download_url: build_magnet(&t.hash, &release_title),
                    size_bytes: t.size_bytes.unwrap_or(0),
                    seeders: t.seeds,
                    quality,
                    score,
                });
            }
        }
    }

    Ok(results)
}

// ==========================================
// Provider 3: EZTV (TV Shows)
// ==========================================
#[derive(Deserialize)]
struct EztvResponse {
    #[serde(default)]
    torrents: Vec<EztvTorrent>,
}

#[derive(Deserialize)]
struct EztvTorrent {
    title: String,
    magnet_url: String,
    seeds: u32,
    size_bytes: Option<String>,
}

async fn search_eztv(
    http: &Client,
    indexer: &IndexerConfig,
    query: &str,
) -> anyhow::Result<Vec<ReleaseCandidate>> {
    let base = indexer.url.trim_end_matches('/');
    let url = format!("{}/api/get-torrents", base);

    let resp: EztvResponse = http
        .get(&url)
        .query(&[("limit", "100"), ("page", "1")])
        .send()
        .await?
        .json()
        .await?;

    let query_tokens: Vec<String> = query
        .to_lowercase()
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();

    let mut results = Vec::new();
    for t in resp.torrents {
        if t.seeds == 0 || t.magnet_url.is_empty() {
            continue;
        }

        let title_lower = t.title.to_lowercase();
        if !query_tokens.iter().all(|tok| title_lower.contains(tok)) {
            continue;
        }

        let (quality, score) = score_release(&t.title, t.seeds);
        let size_bytes = t
            .size_bytes
            .as_deref()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);

        results.push(ReleaseCandidate {
            indexer_name: indexer.name.clone(),
            title: t.title,
            download_url: t.magnet_url,
            size_bytes,
            seeders: t.seeds,
            quality,
            score,
        });
    }

    Ok(results)
}

// ==========================================
// Unified Search Across All Enabled Indexers
// ==========================================
pub async fn search_all_indexers(
    db: &SqlitePool,
    http: &Client,
    query: &str,
) -> anyhow::Result<Vec<ReleaseCandidate>> {
    let indexers = sqlx::query_as::<_, IndexerConfig>(
        "SELECT id, name, url, api_key, enabled FROM indexers WHERE enabled = 1",
    )
    .fetch_all(db)
    .await?;

    let mut all_releases = Vec::new();

    for indexer in indexers {
        let provider = indexer.api_key.trim().to_lowercase();
        let found = match provider.as_str() {
            "apibay" => search_apibay(http, &indexer, query).await,
            "yts" => search_yts(http, &indexer, query).await,
            "eztv" => search_eztv(http, &indexer, query).await,
            _ => {
                eprintln!(
                    "[INDEXER] Unknown provider '{}' for {}",
                    provider, indexer.name
                );
                Ok(Vec::new())
            }
        };

        match found {
            Ok(mut items) => all_releases.append(&mut items),
            Err(e) => eprintln!(
                "[INDEXER] Error searching {} ({}): {:#}",
                indexer.name, indexer.url, e
            ),
        }
    }

    all_releases.sort_by(|a, b| b.score.cmp(&a.score));
    Ok(all_releases)
}
