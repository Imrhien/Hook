// ============================================================================
// src/indexers/mod.rs
// Handles indexer configuration, querying external torrent sources (ApiBay,
// YTS, EZTV), decoding HTML entities in release titles, scoring candidates,
// and filtering/ranking results against a user's QualityProfile.
// ============================================================================

use std::sync::OnceLock;

use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::models::{Quality, QualityProfile};
use crate::parser::{parse_quality, parse_release_title, ParsedMedia};

#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct IndexerConfig {
    pub id: i64,
    pub name: String,
    pub url: String,
    pub driver: String,  // "apibay", "yts", "eztv", etc.
    pub api_key: String, // Optional credential (empty for public indexers)
    pub enabled: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct ReleaseCandidate {
    pub indexer_name: String,
    pub title: String,
    pub download_url: String, // magnet:?xt=...
    pub size_bytes: u64,
    pub seeders: u32,
    pub quality: Quality,
    pub quality_name: String, // Canonical Quality::as_str() label
    pub score: i32,
    pub profile_rank: i32,
    pub matches_profile: bool,
    pub rejection_reason: Option<String>,
}

const PUBLIC_TRACKERS: &[&str] = &[
    "udp://tracker.opentrackr.org:1337/announce",
    "udp://open.stealth.si:80/announce",
    "udp://tracker.torrent.eu.org:451/announce",
    "udp://tracker.bittor.pw:1337/announce",
    "udp://public.popcorn-tracker.org:6969/announce",
];

/// Converts HTML entities (such as `&eacute;` -> `é` or `&amp;` -> `&`) into
/// clean UTF-8 text and trims surrounding whitespace.
///
/// Rust tip: `html_escape::decode_html_entities` returns a `Cow<'_, str>`
/// ("Clone-on-Write"). If no entities exist in the string, no extra heap
/// allocation happens until `.into_owned()` builds the final `String`.
fn decode_title(raw: &str) -> String {
    html_escape::decode_html_entities(raw).trim().to_string()
}

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

/// Returns a lazily-initialized `Regex` that matches low-quality/cam releases.
///
/// Rust tip: `OnceLock` compiles the regex once on first use and reuses it
/// safely across async tasks without paying regex-compilation cost every time.
fn bad_release_regex() -> &'static Regex {
    static BAD_RE: OnceLock<Regex> = OnceLock::new();
    BAD_RE.get_or_init(|| Regex::new(r"(?i)\b(cam|ts|telesync|hdcam)\b").unwrap())
}

/// Parses canonical Quality via `crate::parser::parse_quality` and computes a score
/// from quality tier, source bonuses, season pack bonus, spam penalties, and seeders.
fn analyze_and_score_release(title: &str, seeders: u32) -> (Quality, String, i32) {
    let quality = parse_quality(title);
    let quality_name = quality.as_str().to_string();
    let lower = title.to_lowercase();

    let mut score = match quality {
        Quality::Remux2160p | Quality::Bluray2160p | Quality::WEBDL2160p => 110,
        Quality::Remux1080p | Quality::Bluray1080p | Quality::WEBDL1080p => 100,
        Quality::HDTV1080p => 85,
        Quality::Bluray720p | Quality::WEBDL720p => 75,
        Quality::HDTV720p => 65,
        Quality::WEBDL480p | Quality::SDTV => 35,
        Quality::Unknown => 10,
    };

    if matches!(parse_release_title(title), ParsedMedia::SeasonPack { .. }) {
        score += 30;
    }

    if lower.contains("remux") {
        score += 20;
    } else if lower.contains("bluray") || lower.contains("blu-ray") {
        score += 15;
    } else if lower.contains("web-dl") || lower.contains("webdl") || lower.contains("webrip") {
        score += 10;
    }

    if bad_release_regex().is_match(&lower) {
        score -= 500;
    }

    score += (seeders.min(50)) as i32;

    (quality, quality_name, score)
}

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

    let items: Vec<ApibayItem> = http
        .get(&url)
        .query(&[("q", query), ("cat", "200")])
        .send()
        .await?
        .json()
        .await?;

    let mut results = Vec::new();
    for item in items {
        if item.info_hash == "0000000000000000000000000000000000000000" || item.info_hash.is_empty()
        {
            continue;
        }

        let seeders: u32 = item.seeders.parse().unwrap_or(0);
        if seeders == 0 {
            continue;
        }

        // Decode HTML entities (like `&eacute;` -> `é`) right away so scoring,
        // magnet naming, and UI display all use the clean title.
        let clean_title = decode_title(&item.name);
        if clean_title.is_empty() {
            continue;
        }

        let size_bytes: u64 = item.size.parse().unwrap_or(0);
        let (quality, quality_name, score) = analyze_and_score_release(&clean_title, seeders);

        results.push(ReleaseCandidate {
            indexer_name: indexer.name.clone(),
            download_url: build_magnet(&item.info_hash, &clean_title),
            title: clean_title,
            size_bytes,
            seeders,
            quality,
            quality_name,
            score,
            profile_rank: 0,
            matches_profile: true,
            rejection_reason: None,
        });
    }

    Ok(results)
}

#[derive(Deserialize)]
struct YtsResponse {
    data: Option<YtsData>,
}

#[derive(Deserialize)]
struct YtsData {
    #[serde(default)]
    movies: Vec<YtsMovie>,
}

// Using `#[serde(default)]` prevents a single null/missing field from failing the entire JSON parse.
#[derive(Deserialize)]
struct YtsMovie {
    #[serde(default)]
    title: String,
    #[serde(default)]
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

    // Fetch raw text first so we can inspect/debug bad responses if needed,
    // while still deserializing cleanly with serde_json.
    let resp = http
        .get(&url)
        .query(&[("query_term", query), ("limit", "15")])
        .send()
        .await?
        .error_for_status()?; // Surfaces HTTP errors like 403, 404, 502, 503

    let body_text = resp.text().await?;
    let parsed: YtsResponse = serde_json::from_str(&body_text).map_err(|e| {
        // Include a short snippet of the body in the error to spot HTML/challenge pages fast
        let preview: String = body_text.chars().take(200).collect();
        anyhow::anyhow!("YTS JSON parse error: {e} | body preview: {preview}")
    })?;

    let mut results = Vec::new();
    if let Some(data) = parsed.data {
        for movie in data.movies {
            // Fall back to `movie.title` if `title_long` was empty/null, and decode HTML entities
            let raw_base = if movie.title_long.trim().is_empty() {
                movie.title.as_str()
            } else {
                movie.title_long.as_str()
            };
            let base_title = decode_title(raw_base);

            for t in movie.torrents {
                if t.seeds == 0 || t.hash.trim().is_empty() {
                    continue;
                }

                let quality_clean = decode_title(&t.quality);
                let rip_clean = decode_title(&t.rip_type);

                // Avoid empty `[web]` / `[]` brackets if `rip_type` wasn't provided
                let release_title = if rip_clean.is_empty() {
                    format!("{} [{}]", base_title, quality_clean)
                } else {
                    format!("{} [{}] [{}]", base_title, quality_clean, rip_clean)
                };

                let (quality, quality_name, score) =
                    analyze_and_score_release(&release_title, t.seeds);

                results.push(ReleaseCandidate {
                    indexer_name: indexer.name.clone(),
                    title: release_title.clone(),
                    download_url: build_magnet(&t.hash, &release_title),
                    size_bytes: t.size_bytes.unwrap_or(0),
                    seeders: t.seeds,
                    quality,
                    quality_name,
                    score,
                    profile_rank: 0,
                    matches_profile: true,
                    rejection_reason: None,
                });
            }
        }
    }

    Ok(results)
}

fn deserialize_u32_flexible<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum NumOrStr {
        Num(u32),
        Str(String),
        Null,
    }

    match NumOrStr::deserialize(deserializer)? {
        NumOrStr::Num(n) => Ok(n),
        NumOrStr::Str(s) => Ok(s.trim().parse::<u32>().unwrap_or(0)),
        NumOrStr::Null => Ok(0),
    }
}

#[derive(Deserialize)]
struct EztvResponse {
    #[serde(default)]
    torrents: Vec<EztvTorrent>,
}

#[derive(Deserialize)]
struct EztvTorrent {
    // EZTV often sends `filename` rather than `title`, so we capture both
    // and fall back cleanly from `title` -> `filename`.
    #[serde(default)]
    title: String,
    #[serde(default)]
    filename: String,
    #[serde(default)]
    magnet_url: String,
    #[serde(default)]
    hash: String,
    #[serde(default, deserialize_with = "deserialize_u32_flexible")]
    seeds: u32,
    size_bytes: Option<String>,
}

// Accepts `imdb_id: Option<i64>` (stored as numeric digits in SQLite with "tt" stripped).
// When present and positive, we pass `imdb_id=<digits>` to EZTV so results are scoped
// to that specific TV show rather than only the 100 newest torrents across all shows.
async fn search_eztv(
    http: &Client,
    indexer: &IndexerConfig,
    query: &str,
    imdb_id: Option<i64>,
) -> anyhow::Result<Vec<ReleaseCandidate>> {
    let base = indexer.url.trim_end_matches('/');
    let url = format!("{}/api/get-torrents", base);

    // Only use positive IMDb IDs; convert `i64` -> `String` so `.query(...)` can borrow `&str`.
    let imdb_param: Option<String> = imdb_id.filter(|&id| id > 0).map(|id| id.to_string());

    let query_tokens: Vec<String> = query
        .to_lowercase()
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();

    let mut raw_count = 0usize;
    let mut results = Vec::new();
    let max_pages = if imdb_param.is_some() { 3 } else { 1 };

    for page in 1..=max_pages {
        let page_str = page.to_string();
        let mut params: Vec<(&str, &str)> = vec![("limit", "100"), ("page", page_str.as_str())];
        if let Some(ref id_str) = imdb_param {
            params.push(("imdb_id", id_str.as_str()));
        }

        let body_text = http
            .get(&url)
            .query(&params)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;

        let parsed: EztvResponse = serde_json::from_str(&body_text).map_err(|e| {
            let preview: String = body_text.chars().take(200).collect();
            anyhow::anyhow!("EZTV JSON parse error (page {page}): {e} | body preview: {preview}")
        })?;

        if parsed.torrents.is_empty() {
            break;
        }

        raw_count += parsed.torrents.len();

        for t in parsed.torrents {
            // Pick `title` first, fall back to `filename`, and decode HTML entities
            let raw_title = if !t.title.trim().is_empty() {
                t.title.as_str()
            } else if !t.filename.trim().is_empty() {
                t.filename.as_str()
            } else {
                continue;
            };
            let display_title = decode_title(raw_title);
            if display_title.is_empty() {
                continue;
            }

            if t.seeds == 0 {
                continue;
            }

            let download_url = if !t.magnet_url.trim().is_empty() {
                t.magnet_url.trim().to_string()
            } else if !t.hash.trim().is_empty() {
                build_magnet(&t.hash, &display_title)
            } else {
                continue;
            };

            // When `imdb_id` is passed, EZTV already filtered by show ID.
            // Without `imdb_id`, require all query words to appear in the title.
            if imdb_param.is_none() {
                let title_lower = display_title.to_lowercase();
                if !query_tokens.iter().all(|tok| title_lower.contains(tok)) {
                    continue;
                }
            }

            let (quality, quality_name, score) = analyze_and_score_release(&display_title, t.seeds);
            let size_bytes = t
                .size_bytes
                .as_deref()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);

            results.push(ReleaseCandidate {
                indexer_name: indexer.name.clone(),
                title: display_title,
                download_url,
                size_bytes,
                seeders: t.seeds,
                quality,
                quality_name,
                score,
                profile_rank: 0,
                matches_profile: true,
                rejection_reason: None,
            });
        }
    }

    eprintln!(
        "[EZTV DEBUG] query='{}' imdb_id={:?} raw_count={} matched_count={}",
        query,
        imdb_param,
        raw_count,
        results.len()
    );
    Ok(results)
}

/// Pre-seeds default public indexers that require no API key or credentials.
pub async fn seed_default_indexers(db: &SqlitePool) -> anyhow::Result<()> {
    let defaults = [
        ("ApiBay", "https://apibay.org", "apibay", "", true),
        ("YTS", "https://yts.lt", "yts", "", true),
        ("EZTV", "https://eztvx.to", "eztv", "", true),
    ];

    for (name, url, driver, api_key, enabled) in defaults {
        let exists: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM indexers WHERE LOWER(name) = LOWER(?) OR url = ? LIMIT 1",
        )
        .bind(name)
        .bind(url)
        .fetch_optional(db)
        .await?;

        if exists.is_none() {
            sqlx::query(
                r#"
                INSERT INTO indexers (name, url, driver, api_key, enabled)
                VALUES (?, ?, ?, ?, ?)
                "#,
            )
            .bind(name)
            .bind(url)
            .bind(driver)
            .bind(api_key)
            .bind(enabled)
            .execute(db)
            .await?;
        }
    }

    Ok(())
}

// ============================================================================
// File: src/indexers/mod.rs (inside `impl ReleaseCandidate`)
// Evaluates a release against a `QualityProfile`. For TV season packs, scales
// `max_episode_size_mb` by the number of episodes in that season when known.
// ============================================================================
impl ReleaseCandidate {
    pub fn apply_profile(&mut self, profile: &QualityProfile, is_movie: bool) {
        self.apply_profile_with_season_episodes(profile, is_movie, None);
    }

    pub fn apply_profile_with_season_episodes(
        &mut self,
        profile: &QualityProfile,
        is_movie: bool,
        season_pack_episode_count: Option<i64>,
    ) {
        let Some(rule) = profile
            .items
            .iter()
            .find(|item| item.quality == self.quality_name)
        else {
            self.matches_profile = false;
            self.profile_rank = 0;
            self.rejection_reason = Some(format!(
                "Quality '{}' is not configured in profile '{}'",
                self.quality_name, profile.name
            ));
            return;
        };

        if !rule.allowed {
            self.matches_profile = false;
            self.profile_rank = rule.rank;
            self.rejection_reason = Some(format!(
                "Quality '{}' is not allowed in profile '{}'",
                self.quality_name, profile.name
            ));
            return;
        }

        let base_limit_mb = if is_movie {
            rule.max_movie_size_mb
        } else {
            rule.max_episode_size_mb
        };

        if let Some(limit_mb) = base_limit_mb {
            // If this release parses as a SeasonPack and we know the season's
            // episode count, scale the single-episode cap by episode count.
            let is_season_pack =
                !is_movie && matches!(parse_release_title(&self.title), ParsedMedia::SeasonPack { .. });

            let effective_limit_mb = if is_season_pack {
                if let Some(ep_count) = season_pack_episode_count.filter(|&c| c > 0) {
                    limit_mb.saturating_mul(ep_count)
                } else {
                    limit_mb
                }
            } else {
                limit_mb
            };

            let size_mb = ((self.size_bytes as f64) / (1024.0 * 1024.0)).ceil() as i64;
            if size_mb > effective_limit_mb {
                self.matches_profile = false;
                self.profile_rank = rule.rank;
                self.rejection_reason = Some(if is_season_pack {
                    format!(
                        "Season pack size {}MB exceeds max {}MB ({}MB x {} episodes) for '{}'",
                        size_mb,
                        effective_limit_mb,
                        limit_mb,
                        season_pack_episode_count.unwrap_or(1),
                        self.quality_name
                    )
                } else {
                    format!(
                        "File size {}MB exceeds max {}MB for '{}'",
                        size_mb, effective_limit_mb, self.quality_name
                    )
                });
                return;
            }
        }

        self.matches_profile = true;
        self.profile_rank = rule.rank;
        self.rejection_reason = None;
    }
}

// ============================================================================
// Looks up how many episodes exist for a given show and season in `episodes`
// (linked via `media_items`). If `imdb_id` is available, matching on `m.imdb_id`
// avoids title mismatches; otherwise falls back to case-insensitive title match.
// ============================================================================
async fn get_season_episode_count(
    db: &SqlitePool,
    series_title: &str,
    season_number: i64,
    imdb_id: Option<i64>,
) -> anyhow::Result<Option<i64>> {
    let count: Option<i64> = if let Some(imdb) = imdb_id.filter(|&id| id > 0) {
        sqlx::query_scalar(
            r#"
            SELECT COUNT(*)
            FROM episodes e
            INNER JOIN media_items m ON m.id = e.media_item_id
            WHERE m.media_type = 'tv'
              AND m.imdb_id = ?
              AND e.season_number = ?
            "#,
        )
        .bind(imdb)
        .bind(season_number)
        .fetch_optional(db)
        .await?
    } else {
        sqlx::query_scalar(
            r#"
            SELECT COUNT(*)
            FROM episodes e
            INNER JOIN media_items m ON m.id = e.media_item_id
            WHERE m.media_type = 'tv'
              AND LOWER(m.title) = LOWER(?)
              AND e.season_number = ?
            "#,
        )
        .bind(series_title)
        .bind(season_number)
        .fetch_optional(db)
        .await?
    };

    // Only treat positive episode counts as valid; if the season hasn't been
    // seeded yet (0 rows), return None so we don't multiply by 0.
    Ok(count.filter(|&n| n > 0))
}

/// Searches all enabled indexers, evaluates results against the provided `QualityProfile`,
/// filters out disallowed/oversized releases, and sorts valid releases by profile rank then score.
/// When `imdb_id` is provided (numeric IMDb ID without the `"tt"` prefix), EZTV can scope
/// its `/api/get-torrents` query directly to that show.
pub async fn search_all_indexers_with_profile(
    db: &SqlitePool,
    http: &Client,
    query: &str,
    profile: Option<(&QualityProfile, bool)>,
    imdb_id: Option<i64>,
) -> anyhow::Result<Vec<ReleaseCandidate>> {
    let indexers = sqlx::query_as::<_, IndexerConfig>(
        "SELECT id, name, url, driver, api_key, enabled FROM indexers WHERE enabled = 1",
    )
    .fetch_all(db)
    .await?;

    let mut all_releases = Vec::new();

    let is_movie_search = profile.map(|(_, is_movie)| is_movie).unwrap_or(false);

    for indexer in indexers {
        let driver = indexer.driver.trim().to_lowercase();

        // EZTV only indexes TV shows, so skip it during movie searches.
        if driver == "eztv" && is_movie_search {
            continue;
        }

        let found = match driver.as_str() {
            "apibay" => search_apibay(http, &indexer, query).await,
            "yts" => search_yts(http, &indexer, query).await,
            "eztv" => search_eztv(http, &indexer, query, imdb_id).await,
            _ => {
                eprintln!(
                    "[INDEXER] Unknown driver '{}' for {}",
                    driver, indexer.name
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

    if let Some((prof, is_movie)) = profile {
        // Cache episode counts per season so we only query `episodes` once per
        // season number during a single search, even if there are many packs.
        let mut season_counts: std::collections::HashMap<i64, Option<i64>> =
            std::collections::HashMap::new();

        for release in &mut all_releases {
            let pack_ep_count = if !is_movie {
                match parse_release_title(&release.title) {
                    ParsedMedia::SeasonPack {
                        series_title,
                        season,
                    } => {
                        let s_num = season as i64;
                        if let Some(cached) = season_counts.get(&s_num) {
                            *cached
                        } else {
                            let count =
                                get_season_episode_count(db, &series_title, s_num, imdb_id)
                                    .await
                                    .unwrap_or(None);
                            season_counts.insert(s_num, count);
                            count
                        }
                    }
                    _ => None,
                }
            } else {
                None
            };

            release.apply_profile_with_season_episodes(prof, is_movie, pack_ep_count);
        }

            // Debugging: show rejected results
        for r in &all_releases {
            if !r.matches_profile {
                eprintln!(
                    "[PROFILE REJECT] title='{}' quality='{}' reason={:?}",
                    r.title, r.quality_name, r.rejection_reason
                );
            }
        } 

        all_releases.retain(|r| r.matches_profile);

        all_releases.sort_by(|a, b| {
            b.profile_rank
                .cmp(&a.profile_rank)
                .then_with(|| b.score.cmp(&a.score))
        });
    } else {
        all_releases.sort_by(|a, b| b.score.cmp(&a.score));
    }

    Ok(all_releases)
}