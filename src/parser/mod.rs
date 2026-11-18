use regex::Regex;
use reqwest::Client;
use serde::Serialize;
use sqlx::SqlitePool;
use std::path::Path;
use walkdir::WalkDir;

use crate::metadata::{fetch_tv_episodes, search_tmdb};
use crate::models::{Quality, QualityProfile};

const VIDEO_EXTENSIONS: &[&str] = &["mkv", "mp4", "avi", "mov", "wmv", "m4v", "webm", "ts"];

#[derive(Debug, Serialize)]
pub struct ScanSummary {
    pub files_found: usize,
    pub imported_or_updated: usize,
    pub unmatched: Vec<String>,
}

#[derive(Debug)]
enum ParsedMedia {
    Movie {
        title: String,
        year: Option<i32>,
    },
    Episode {
        series_title: String,
        season: i32,
        episode: i32,
    },
}

fn parse_media_path(path: &Path) -> Option<ParsedMedia> {
    let file_name = path.file_name()?.to_str()?;
    let stem = Path::new(file_name).file_stem()?.to_str()?;

    let clean = |s: &str| {
        s.replace('.', " ")
            .replace('_', " ")
            .trim_matches(|c: char| c.is_whitespace() || c == '-')
            .to_string()
    };

    // 1. Standard TV pattern: S01E02 or s1e2
    let s_e_re =
        Regex::new(r"(?i)^(?P<title>.+?)[.\s_-]+S(?P<season>\d{1,2})[.\s_-]*E(?P<episode>\d{1,3})")
            .ok()?;
    if let Some(caps) = s_e_re.captures(stem) {
        return Some(ParsedMedia::Episode {
            series_title: clean(caps.name("title")?.as_str()),
            season: caps.name("season")?.as_str().parse().ok()?,
            episode: caps.name("episode")?.as_str().parse().ok()?,
        });
    }

    // 2. Reversed TV pattern: E02S01 or e02.s01
    let e_s_re =
        Regex::new(r"(?i)^(?P<title>.+?)[.\s_-]+E(?P<episode>\d{1,3})[.\s_-]*S(?P<season>\d{1,2})")
            .ok()?;
    if let Some(caps) = e_s_re.captures(stem) {
        return Some(ParsedMedia::Episode {
            series_title: clean(caps.name("title")?.as_str()),
            season: caps.name("season")?.as_str().parse().ok()?,
            episode: caps.name("episode")?.as_str().parse().ok()?,
        });
    }

    // 3. Alt TV pattern: 1x02
    let x_re =
        Regex::new(r"(?i)^(?P<title>.+?)[.\s_-]+(?P<season>\d{1,2})x(?P<episode>\d{1,3})").ok()?;
    if let Some(caps) = x_re.captures(stem) {
        return Some(ParsedMedia::Episode {
            series_title: clean(caps.name("title")?.as_str()),
            season: caps.name("season")?.as_str().parse().ok()?,
            episode: caps.name("episode")?.as_str().parse().ok()?,
        });
    }

    // 4. Folder-based TV fallback: /Show Name/Season 01/E02.mkv
    if let Some(parent) = path.parent() {
        let parent_name = parent.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let season_dir_re =
            Regex::new(r"(?i)^(?:season|series|s)[.\s_-]*(?P<season>\d{1,2})$").ok()?;

        if let Some(s_caps) = season_dir_re.captures(parent_name) {
            let season: i32 = s_caps.name("season")?.as_str().parse().ok()?;
            let show_dir = parent.parent()?.file_name()?.to_str()?;
            let ep_only_re =
                Regex::new(r"(?i)^(?:E|Ep|Episode)?[.\s_-]*(?P<episode>\d{1,3})(?:\b|[.\s_-])")
                    .ok()?;

            if let Some(e_caps) = ep_only_re.captures(stem) {
                let episode: i32 = e_caps.name("episode")?.as_str().parse().ok()?;
                return Some(ParsedMedia::Episode {
                    series_title: clean(show_dir),
                    season,
                    episode,
                });
            }
        }
    }

    // 5. Movie with Year: "Inception (2010)" or "Inception.2010.1080p"
    let movie_year_re =
        Regex::new(r"(?i)^(?P<title>.+?)[.\s_(-]+(?P<year>(?:19|20)\d{2})").ok()?;
    if let Some(caps) = movie_year_re.captures(stem) {
        return Some(ParsedMedia::Movie {
            title: clean(caps.name("title")?.as_str()),
            year: caps.name("year")?.as_str().parse().ok(),
        });
    }

    // 6. Fallback: Movie without year
    let clean_title = clean(stem);
    if !clean_title.is_empty() {
        Some(ParsedMedia::Movie {
            title: clean_title,
            year: None,
        })
    } else {
        None
    }
}

pub async fn scan_directory(
    db: &SqlitePool,
    http: &Client,
    tmdb_api_key: &str,
    root_path: &str,
) -> anyhow::Result<ScanSummary> {
    let mut files_found = 0;
    let mut imported_or_updated = 0;
    let mut unmatched = Vec::new();

    for entry in WalkDir::new(root_path).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase())
            .unwrap_or_default();

        if !VIDEO_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }

        files_found += 1;
        let full_path_str = path.to_string_lossy().to_string();

        let Some(parsed) = parse_media_path(path) else {
            unmatched.push(full_path_str);
            continue;
        };

        match parsed {
            ParsedMedia::Movie { title, year } => {
                let results = search_tmdb(http, tmdb_api_key, &title)
                    .await
                    .unwrap_or_default();
                let best_match = results
                    .into_iter()
                    .find(|r| r.media_type == "movie" && (year.is_none() || r.year == year));

                if let Some(tmdb) = best_match {
                    sqlx::query(
                        r#"
                            INSERT INTO media_items (tmdb_id, media_type, title, year, overview, poster_path, status, root_folder, quality_profile_id)
                            VALUES (?, 'movie', ?, ?, ?, ?, 'available', ?, 1)
                            ON CONFLICT(tmdb_id) DO UPDATE SET
                                status = 'available',
                                root_folder = excluded.root_folder
                        "#,
                    )
                    .bind(tmdb.id)
                    .bind(&tmdb.title)
                    .bind(tmdb.year)
                    .bind(&tmdb.overview)
                    .bind(&tmdb.poster_path)
                    .bind(&full_path_str)
                    .execute(db)
                    .await?;

                    imported_or_updated += 1;
                } else {
                    unmatched.push(full_path_str);
                }
            }
            ParsedMedia::Episode {
                series_title,
                season,
                episode,
            } => {
                let results = search_tmdb(http, tmdb_api_key, &series_title)
                    .await
                    .unwrap_or_default();
                let best_match = results.into_iter().find(|r| r.media_type == "tv");

                if let Some(tmdb) = best_match {
                    let media_id: i64 = sqlx::query_scalar(
                        r#"
                            INSERT INTO media_items (tmdb_id, media_type, title, year, overview, poster_path, status, quality_profile_id)
                            VALUES (?, 'tv', ?, ?, ?, ?, 'available', 1)
                            ON CONFLICT(tmdb_id) DO UPDATE SET status = 'available'
                            RETURNING id
                        "#,
                    )
                    .bind(tmdb.id)
                    .bind(&tmdb.title)
                    .bind(tmdb.year)
                    .bind(&tmdb.overview)
                    .bind(&tmdb.poster_path)
                    .fetch_one(db)
                    .await?;

                    // 1. Seed full episode list from TMDB
                    match fetch_tv_episodes(http, tmdb_api_key, tmdb.id).await {
                        Ok(tmdb_eps) => {
                            for ep in tmdb_eps {
                                sqlx::query(
                                    r#"
                                    INSERT OR IGNORE INTO episodes (media_item_id, season_number, episode_number, title, air_date, status)
                                    VALUES (?, ?, ?, ?, ?, 'missing')
                                    "#,
                                )
                                .bind(media_id)
                                .bind(ep.season_number)
                                .bind(ep.episode_number)
                                .bind(&ep.title)
                                .bind(&ep.air_date)
                                .execute(db)
                                .await?;
                            }
                        }
                        Err(e) => eprintln!("Failed to fetch TV episodes for {}: {}", tmdb.title, e),
                    }

                    // 2. Mark the scanned episode on disk as 'available'
                    sqlx::query(
                        r#"
                        INSERT INTO episodes (media_item_id, season_number, episode_number, status, file_path)
                        VALUES (?, ?, ?, 'available', ?)
                        ON CONFLICT(media_item_id, season_number, episode_number) DO UPDATE SET
                            status = 'available',
                            file_path = excluded.file_path
                        "#,
                    )
                    .bind(media_id)
                    .bind(season)
                    .bind(episode)
                    .bind(&full_path_str)
                    .execute(db)
                    .await?;

                    // 3. Set series status to 'available' only if 0 episodes are missing, otherwise 'partial'
                    let missing_count: (i64,) = sqlx::query_as(
                        "SELECT COUNT(*) FROM episodes WHERE media_item_id = ? AND status != 'available'",
                    )
                    .bind(media_id)
                    .fetch_one(db)
                    .await?;

                    let series_status = if missing_count.0 == 0 {
                        "available"
                    } else {
                        "partial"
                    };

                    sqlx::query("UPDATE media_items SET status = ? WHERE id = ?")
                        .bind(series_status)
                        .bind(media_id)
                        .execute(db)
                        .await?;

                    imported_or_updated += 1;
                } else {
                    unmatched.push(full_path_str);
                }
            }
        }
    }

    Ok(ScanSummary {
        files_found,
        imported_or_updated,
        unmatched,
    })
}

/// Detects the Quality tier from a torrent title or video file name.
pub fn parse_quality(title: &str) -> Quality {
    let lower = title.to_lowercase();

    // 1. Detect Resolution
    let is_2160p = lower.contains("2160p") || lower.contains("4k") || lower.contains("uhd");
    let is_1080p = lower.contains("1080p") || lower.contains("1080i");
    let is_720p = lower.contains("720p");
    let is_480p = lower.contains("480p") || lower.contains("576p");

    // 2. Detect Source
    let is_remux = lower.contains("remux");
    let is_bluray = lower.contains("bluray")
        || lower.contains("blu-ray")
        || lower.contains("bdrip")
        || lower.contains("brrip");
    let is_webdl = lower.contains("web-dl")
        || lower.contains("webdl")
        || lower.contains("webrip")
        || lower.contains("web-rip")
        || lower.contains("web.");
    let is_hdtv = lower.contains("hdtv") || lower.contains("pdtv");
    let is_sdtv = lower.contains("sdtv") || lower.contains("dvdrip") || lower.contains("xvid");

    // 3. Combine Source + Resolution into a Quality tier
    if is_remux {
        if is_2160p {
            return Quality::Remux2160p;
        }
        return Quality::Remux1080p;
    }

    if is_bluray {
        if is_2160p {
            return Quality::Bluray2160p;
        }
        if is_1080p {
            return Quality::Bluray1080p;
        }
        if is_720p {
            return Quality::Bluray720p;
        }
        // Default BluRay without explicit resolution usually implies 1080p
        return Quality::Bluray1080p;
    }

    if is_webdl {
        if is_2160p {
            return Quality::WEBDL2160p;
        }
        if is_1080p {
            return Quality::WEBDL1080p;
        }
        if is_720p {
            return Quality::WEBDL720p;
        }
        if is_480p {
            return Quality::WEBDL480p;
        }
        return Quality::WEBDL1080p;
    }

    if is_hdtv {
        if is_1080p {
            return Quality::HDTV1080p;
        }
        if is_720p {
            return Quality::HDTV720p;
        }
        return Quality::SDTV;
    }

    if is_sdtv || is_480p {
        return Quality::SDTV;
    }

    // Fallback when resolution is tagged without a source (e.g., "Movie.2024.1080p.x265")
    if is_2160p {
        return Quality::WEBDL2160p;
    }
    if is_1080p {
        return Quality::WEBDL1080p;
    }
    if is_720p {
        return Quality::WEBDL720p;
    }

    Quality::Unknown
}

/// Checks if a release matches the given QualityProfile and size constraints.
/// Returns `Some(rank)` if allowed and within limits, or `None` if rejected.
pub fn evaluate_release(
    profile: &QualityProfile,
    release_title: &str,
    size_bytes: Option<u64>,
    is_movie: bool,
) -> Option<i32> {
    let detected = parse_quality(release_title);
    let detected_str = detected.as_str();

    let rule = profile
        .items
        .iter()
        .find(|item| item.quality == detected_str && item.allowed)?;

    if let Some(bytes) = size_bytes {
        let size_mb = (bytes as f64 / (1024.0 * 1024.0)).ceil() as i64;
        let max_mb = if is_movie {
            rule.max_movie_size_mb
        } else {
            rule.max_episode_size_mb
        };

        if let Some(limit) = max_mb {
            if size_mb > limit {
                return None; // Exceeds max allowed file size
            }
        }
    }

    Some(rule.rank)
}