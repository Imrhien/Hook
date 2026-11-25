use regex::Regex;
use reqwest::Client;
use serde::Serialize;
use sqlx::SqlitePool;
use std::path::Path;
use std::collections::{HashMap, HashSet};
use walkdir::WalkDir;

use crate::metadata::{fetch_tv_episodes, search_tmdb};
use crate::models::{Quality};

const VIDEO_EXTENSIONS: &[&str] = &["mkv", "mp4", "avi", "mov", "wmv", "m4v", "webm", "ts"];

#[derive(Debug, Serialize)]
pub struct ScanSummary {
    pub files_found: usize,
    pub imported_or_updated: usize,
    pub unmatched: Vec<String>,
}

/// Represents the type of media parsed from a file path or release title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedMedia {
    Movie {
        title: String,
        year: Option<i32>,
    },
    Episode {
        series_title: String,
        season: i32,
        episode: i32,
    },
    SeasonPack {
        series_title: String,
        season: i32,
    },
}

// CLEANUP - quality profiles may not be deleted
// Handle when quality profiles are removed
async fn resolve_default_quality_profile_id(db: &SqlitePool) -> anyhow::Result<Option<i64>> {
    let profile_id: Option<i64> = sqlx::query_scalar(
        r#"
        SELECT id
        FROM quality_profiles
        ORDER BY CASE WHEN name = 'Any' THEN 0 ELSE 1 END, id ASC
        LIMIT 1
        "#,
    )
    .fetch_optional(db)
    .await?;

    Ok(profile_id)
}

/// Parses a local file path to determine if it's a Movie, a Single Episode, or a Season Pack.
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

    // 4. Season Pack pattern in filename: Show.Name.S01 or Show.Name.Season.1 (without specific episodes)
    let season_pack_re =
        Regex::new(r"(?i)^(?P<title>.+?)[.\s_-]+(?:Season|S)(?P<season>\d{1,2})\b").ok()?;
    if let Some(caps) = season_pack_re.captures(stem) {
        return Some(ParsedMedia::SeasonPack {
            series_title: clean(caps.name("title")?.as_str()),
            season: caps.name("season")?.as_str().parse().ok()?,
        });
    }

    // 5. Folder-based TV fallback: /Show Name/Season 01/E02.mkv
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

    // 6. Movie with Year: "Inception (2010)" or "Inception.2010.1080p"
    let movie_year_re =
        Regex::new(r"(?i)^(?P<title>.+?)[.\s_(-]+(?P<year>(?:19|20)\d{2})").ok()?;
    if let Some(caps) = movie_year_re.captures(stem) {
        return Some(ParsedMedia::Movie {
            title: clean(caps.name("title")?.as_str()),
            year: caps.name("year")?.as_str().parse().ok(),
        });
    }

    // 7. Fallback: Movie without year
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

/// Parses an indexer release title specifically to check if it's a Season Pack or Single Episode.
pub fn parse_release_title(title: &str) -> ParsedMedia {
    let clean = |s: &str| {
        s.replace('.', " ")
            .replace('_', " ")
            .trim_matches(|c: char| c.is_whitespace() || c == '-')
            .to_string()
    };

    // Check for explicit single episode first (e.g. S01E02)
    let s_e_re = Regex::new(r"(?i)^(?P<title>.+?)[.\s_-]+S(?P<season>\d{1,2})[.\s_-]*E(?P<episode>\d{1,3})").unwrap();
    if let Some(caps) = s_e_re.captures(title) {
        if let (Some(s), Some(e)) = (caps.name("season"), caps.name("episode")) {
            if let (Ok(season), Ok(episode)) = (s.as_str().parse(), e.as_str().parse()) {
                return ParsedMedia::Episode {
                    series_title: clean(caps.name("title").unwrap().as_str()),
                    season,
                    episode,
                };
            }
        }
    }

    // Check for Season Pack in release title (e.g. Show.Name.S01 or Show.Name.Season.1)
    let season_pack_re = Regex::new(r"(?i)^(?P<title>.+?)[.\s_-]+(?:Season|S)(?P<season>\d{1,2})\b").unwrap();
    if let Some(caps) = season_pack_re.captures(title) {
        if let Some(s) = caps.name("season") {
            if let Ok(season) = s.as_str().parse() {
                return ParsedMedia::SeasonPack {
                    series_title: clean(caps.name("title").unwrap().as_str()),
                    season,
                };
            }
        }
    }

    // Default fallback to Movie or generic title
    ParsedMedia::Movie {
        title: clean(title),
        year: None,
    }
}

/// Recalculates a TV show's `status` (`'available'` vs `'partial'`) from episodes
/// in seasons greater than 0 (ignoring Season 0 / specials).
async fn refresh_tv_series_status(db: &SqlitePool, media_id: i64) -> anyhow::Result<()> {
    let (total_eps, missing_eps): (i64, i64) = sqlx::query_as(
        r#"
        SELECT
            COUNT(*) AS total_eps,
            COALESCE(SUM(CASE WHEN status != 'available' THEN 1 ELSE 0 END), 0) AS missing_eps
        FROM episodes
        WHERE media_item_id = ? AND season_number > 0
        "#,
    )
    .bind(media_id)
    .fetch_one(db)
    .await?;

    let series_status = if total_eps > 0 && missing_eps == 0 {
        "available"
    } else {
        "partial"
    };

    sqlx::query("UPDATE media_items SET status = ? WHERE id = ?")
        .bind(series_status)
        .bind(media_id)
        .execute(db)
        .await?;

    Ok(())
}

// ============================================================================
// Scans `root_path` for video files, skips `search_tmdb` when a matching item
// already exists in `media_items`, marks found files as `'available'`, and
// flips externally removed files to `'requested'`.
// ============================================================================
pub async fn scan_directory(
    db: &SqlitePool,
    http: &Client,
    tmdb_api_key: &str,
    root_path: &str,
) -> anyhow::Result<ScanSummary> {
    let default_profile_id = resolve_default_quality_profile_id(db).await?;
    let Some(profile_id) = default_profile_id else {
        anyhow::bail!(
            "No quality_profiles row exists; cannot insert media_items due to foreign key constraint"
        );
    };

    let mut files_found = 0;
    let mut imported_or_updated = 0;
    let mut unmatched = Vec::new();
    let mut tv_episodes_cache: HashMap<i64, Vec<_>> = HashMap::new();

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
        println!("[SCAN] Processing: {}", full_path_str);

        let Some(parsed) = parse_media_path(path) else {
            unmatched.push(full_path_str);
            continue;
        };

        match parsed {
            ParsedMedia::Movie { title, year } => {
                // Prioritize the filename; fall back to the parent folder name if Unknown.
                // Ignore `Quality::Unknown` so existing `current_quality` is never overwritten.
                let file_quality = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(parse_quality)
                    .filter(|&q| q != Quality::Unknown);

                let detected_quality: Option<Quality> = file_quality.or_else(|| {
                    path.parent()
                        .and_then(|p| p.file_name())
                        .and_then(|n| n.to_str())
                        .map(parse_quality)
                        .filter(|&q| q != Quality::Unknown)
                });

                let current_quality: Option<&str> = detected_quality.map(|q| q.as_str());

                // Fast path 1: Check if `media_items` already has this movie by file path
                // (`root_folder` stores the movie file path) or by title (+ year if present).
                let existing_movie: Option<(i64, String)> = if let Some(y) = year {
                    sqlx::query_as(
                        r#"
                        SELECT id, title
                        FROM media_items
                        WHERE media_type = 'movie'
                          AND (root_folder = ? OR (LOWER(title) = LOWER(?) AND year = ?))
                        LIMIT 1
                        "#,
                    )
                    .bind(&full_path_str)
                    .bind(&title)
                    .bind(y)
                    .fetch_optional(db)
                    .await?
                } else {
                    sqlx::query_as(
                        r#"
                        SELECT id, title
                        FROM media_items
                        WHERE media_type = 'movie'
                          AND (root_folder = ? OR LOWER(title) = LOWER(?))
                        LIMIT 1
                        "#,
                    )
                    .bind(&full_path_str)
                    .bind(&title)
                    .fetch_optional(db)
                    .await?
                };

                if let Some((movie_id, _existing_title)) = existing_movie {
                    // Already in `media_items` — skip `search_tmdb` and update status, path, and quality.
                    sqlx::query(
                        r#"
                        UPDATE media_items
                        SET status = 'available',
                            root_folder = ?,
                            current_quality = COALESCE(?, current_quality)
                        WHERE id = ?
                        "#,
                    )
                    .bind(&full_path_str)
                    .bind(current_quality)
                    .bind(movie_id)
                    .execute(db)
                    .await?;

                    imported_or_updated += 1;
                    continue;
                }

                // New movie not yet in `media_items`: query TMDb and insert.
                let results = search_tmdb(http, tmdb_api_key, &title)
                    .await
                    .unwrap_or_default();
                let best_match = results
                    .into_iter()
                    .find(|r| r.media_type == "movie" && (year.is_none() || r.year == year));

                if let Some(tmdb) = best_match {
                    sqlx::query(
                        r#"
                        INSERT INTO media_items (tmdb_id, media_type, title, year, overview, poster_path, status, root_folder, quality_profile_id, current_quality)
                        VALUES (?, 'movie', ?, ?, ?, ?, 'available', ?, ?, ?)
                        ON CONFLICT(tmdb_id) DO UPDATE SET
                            status = 'available',
                            root_folder = excluded.root_folder,
                            current_quality = COALESCE(excluded.current_quality, media_items.current_quality)
                        "#,
                    )
                    .bind(tmdb.id)
                    .bind(&tmdb.title)
                    .bind(tmdb.year)
                    .bind(&tmdb.overview)
                    .bind(&tmdb.poster_path)
                    .bind(&full_path_str)
                    .bind(profile_id)
                    .bind(current_quality)
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
                // Fast path 2: Check if the TV show already exists in `media_items`
                // so we don't hit `search_tmdb` repeatedly for every episode.
                let existing_show: Option<(i64, i64, String)> = sqlx::query_as(
                    r#"
                    SELECT id, tmdb_id, title
                    FROM media_items
                    WHERE media_type = 'tv' AND LOWER(title) = LOWER(?)
                    LIMIT 1
                    "#,
                )
                .bind(&series_title)
                .fetch_optional(db)
                .await?;

                let resolved_show: Option<(i64, i64, String)> = if let Some(show) = existing_show {
                    Some(show)
                } else {
                    let results = search_tmdb(http, tmdb_api_key, &series_title)
                        .await
                        .unwrap_or_default();
                    if let Some(tmdb) = results.into_iter().find(|r| r.media_type == "tv") {
                        let media_id: i64 = sqlx::query_scalar(
                            r#"
                            INSERT INTO media_items (tmdb_id, media_type, title, year, overview, poster_path, status, quality_profile_id)
                            VALUES (?, 'tv', ?, ?, ?, ?, 'partial', ?)
                            ON CONFLICT(tmdb_id) DO UPDATE SET
                                title = excluded.title
                            RETURNING id
                            "#,
                        )
                        .bind(tmdb.id)
                        .bind(&tmdb.title)
                        .bind(tmdb.year)
                        .bind(&tmdb.overview)
                        .bind(&tmdb.poster_path)
                        .bind(profile_id)
                        .fetch_one(db)
                        .await?;

                        Some((media_id, tmdb.id, tmdb.title))
                    } else {
                        None
                    }
                };

                if let Some((media_id, tmdb_id, show_title)) = resolved_show {
                    // 1. Seed full episode list from TMDB (cached per tmdb_id during scan)
                    let tmdb_eps = if let Some(cached) = tv_episodes_cache.get(&tmdb_id) {
                        Some(cached.clone())
                    } else {
                        match fetch_tv_episodes(http, tmdb_api_key, tmdb_id).await {
                            Ok(eps) => {
                                tv_episodes_cache.insert(tmdb_id, eps.clone());
                                Some(eps)
                            }
                            Err(e) => {
                                eprintln!(
                                    "Failed to fetch TV episodes for {}: {}",
                                    show_title, e
                                );
                                None
                            }
                        }
                    };

                    if let Some(eps) = tmdb_eps {
                        for ep in eps {
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

                    // 3. Set series status to 'available' only when all season > 0 episodes are available
                    refresh_tv_series_status(db, media_id).await?;

                    imported_or_updated += 1;
                } else {
                    unmatched.push(full_path_str);
                }
            }

            ParsedMedia::SeasonPack { series_title, season } => {
                // Fast path 3: Reuse existing TV row in `media_items` before calling `search_tmdb`.
                let existing_show: Option<(i64, i64, String)> = sqlx::query_as(
                    r#"
                    SELECT id, tmdb_id, title
                    FROM media_items
                    WHERE media_type = 'tv' AND LOWER(title) = LOWER(?)
                    LIMIT 1
                    "#,
                )
                .bind(&series_title)
                .fetch_optional(db)
                .await?;

                let resolved_show: Option<(i64, i64, String)> = if let Some(show) = existing_show {
                    Some(show)
                } else {
                    let results = search_tmdb(http, tmdb_api_key, &series_title)
                        .await
                        .unwrap_or_default();
                    if let Some(tmdb) = results.into_iter().find(|r| r.media_type == "tv") {
                        let media_id: i64 = sqlx::query_scalar(
                            r#"
                            INSERT INTO media_items (tmdb_id, media_type, title, year, overview, poster_path, status, quality_profile_id)
                            VALUES (?, 'tv', ?, ?, ?, ?, 'partial', ?)
                            ON CONFLICT(tmdb_id) DO UPDATE SET
                                title = excluded.title
                            RETURNING id
                            "#,
                        )
                        .bind(tmdb.id)
                        .bind(&tmdb.title)
                        .bind(tmdb.year)
                        .bind(&tmdb.overview)
                        .bind(&tmdb.poster_path)
                        .bind(profile_id)
                        .fetch_one(db)
                        .await?;

                        Some((media_id, tmdb.id, tmdb.title))
                    } else {
                        None
                    }
                };

                if let Some((media_id, tmdb_id, show_title)) = resolved_show {
                    let tmdb_eps = if let Some(cached) = tv_episodes_cache.get(&tmdb_id) {
                        Some(cached.clone())
                    } else {
                        match fetch_tv_episodes(http, tmdb_api_key, tmdb_id).await {
                            Ok(eps) => {
                                tv_episodes_cache.insert(tmdb_id, eps.clone());
                                Some(eps)
                            }
                            Err(e) => {
                                eprintln!(
                                    "Failed to fetch TV episodes for {}: {}",
                                    show_title, e
                                );
                                None
                            }
                        }
                    };

                    if let Some(eps) = tmdb_eps {
                        for ep in eps {
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

                    sqlx::query(
                        r#"
                        UPDATE episodes
                        SET status = 'available', file_path = ?
                        WHERE media_item_id = ? AND season_number = ?
                        "#,
                    )
                    .bind(&full_path_str)
                    .bind(media_id)
                    .bind(season)
                    .execute(db)
                    .await?;

                    refresh_tv_series_status(db, media_id).await?;

                    imported_or_updated += 1;
                } else {
                    unmatched.push(full_path_str);
                }
            }
        }
    }

    // Reconcile externally deleted files under `root_path`:
    // 1. Movies whose `root_folder` file no longer exists on disk -> status = 'requested'
    let movie_rows: Vec<(i64, Option<String>)> = sqlx::query_as(
        r#"
        SELECT id, root_folder
        FROM media_items
        WHERE media_type = 'movie'
          AND status != 'requested'
          AND root_folder IS NOT NULL
        "#,
    )
    .fetch_all(db)
    .await?;

    for (movie_id, file_path_opt) in movie_rows {
        if let Some(path_str) = file_path_opt {
            if std::path::Path::new(&path_str).starts_with(root_path)
                && !std::path::Path::new(&path_str).exists()
            {
                sqlx::query(
                    r#"
                    UPDATE media_items
                    SET status = 'requested',
                        current_quality = NULL
                    WHERE id = ?
                    "#,
                )
                .bind(movie_id)
                .execute(db)
                .await?;

                imported_or_updated += 1;
            }
        }
    }

    // 2. Episodes whose `file_path` no longer exists on disk -> status = 'requested',
    //    then refresh each affected TV show's status.
    let episode_rows: Vec<(i64, i64, String)> = sqlx::query_as(
        r#"
        SELECT id, media_item_id, file_path
        FROM episodes
        WHERE status != 'requested'
          AND file_path IS NOT NULL
          AND file_path != ''
        "#,
    )
    .fetch_all(db)
    .await?;

    let mut affected_shows: HashSet<i64> = HashSet::new();

    for (ep_id, media_item_id, ep_path) in episode_rows {
        if std::path::Path::new(&ep_path).starts_with(root_path)
            && !std::path::Path::new(&ep_path).exists()
        {
            sqlx::query(
                r#"
                UPDATE episodes
                SET status = 'requested',
                    file_path = NULL
                WHERE id = ?
                "#,
            )
            .bind(ep_id)
            .execute(db)
            .await?;

            affected_shows.insert(media_item_id);
            imported_or_updated += 1;
        }
    }

    for show_id in affected_shows {
        refresh_tv_series_status(db, show_id).await?;
    }

    Ok(ScanSummary {
        files_found,
        imported_or_updated,
        unmatched,
    })
}

fn has_token(haystack: &str, token: &str) -> bool {
    haystack.split(|c: char| !c.is_ascii_alphanumeric()).any(|part| part == token)
}

/// Detects the Quality tier from a torrent title or video file name.
pub fn parse_quality(title: &str) -> Quality {
    let lower = title.to_lowercase();

    let is_2160p = has_token(&lower, "2160p")
        || has_token(&lower, "4k")
        || has_token(&lower, "uhd");
    let is_1080p = has_token(&lower, "1080p") || has_token(&lower, "1080i");
    let is_720p = has_token(&lower, "720p");
    let is_480p = has_token(&lower, "480p") || has_token(&lower, "576p");

    let is_remux = has_token(&lower, "remux");
    let is_bluray = has_token(&lower, "bluray")
        || lower.contains("blu-ray")
        || has_token(&lower, "bdrip")
        || has_token(&lower, "brrip");
    let is_webdl = lower.contains("web-dl")
        || lower.contains("webdl")
        || lower.contains("web-rip")
        || lower.contains("webrip")
        || has_token(&lower, "web");
    let is_hdtv = has_token(&lower, "hdtv") || has_token(&lower, "pdtv");
    let is_sdtv = has_token(&lower, "sdtv")
        || has_token(&lower, "dvdrip")
        || has_token(&lower, "xvid");

    if is_remux {
        if is_2160p {
            return Quality::Remux2160p;
        }
        if is_1080p {
            return Quality::Remux1080p;
        }
        return Quality::Unknown;
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
        return Quality::Unknown;
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
        return Quality::Unknown;
    }

    if is_hdtv {
        if is_1080p {
            return Quality::HDTV1080p;
        }
        if is_720p {
            return Quality::HDTV720p;
        }
        if is_480p || is_sdtv {
            return Quality::SDTV;
        }
        return Quality::Unknown;
    }

    if is_sdtv || is_480p {
        return Quality::SDTV;
    }

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