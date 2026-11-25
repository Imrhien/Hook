use reqwest::Client;
use sqlx::SqlitePool;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tokio::sync::RwLock;

use crate::downloaders::{DownloadClient, DownloadStatus};
use crate::indexers::{search_all_indexers_with_profile, ReleaseCandidate};
use crate::models::{QualityProfile, QualityProfileItem, Quality};
use crate::parser::{parse_quality, parse_release_title, scan_directory, ParsedMedia, ScanSummary};

// Video and subtitle extensions to import into the library destination folder.
const VIDEO_EXTENSIONS: [&str; 6] = ["mkv", "mp4", "avi", "mov", "m4v", "webm"];
const SUBTITLE_EXTENSIONS: [&str; 7] = ["srt", "ass", "ssa", "sub", "idx", "vtt", "sup"];

/// Searches indexers for Wanted movies and missing monitored TV episodes.
/// For TV shows, tries a Season Pack first and falls back to individual episodes.
pub async fn run_automated_wanted_search(
    db: &SqlitePool,
    http: &Client,
    downloader: &Arc<dyn DownloadClient>,
) -> anyhow::Result<()> {
    println!("[WORKER] Starting automated search for Wanted media...");

    // =========================================================================
    // Reset orphaned 'queued' episodes when no active download row exists
    // =========================================================================
    sqlx::query(
        r#"
        UPDATE episodes
        SET status = 'missing'
        WHERE status = 'queued'
          AND id NOT IN (
              SELECT DISTINCT episode_id
              FROM downloads
              WHERE episode_id IS NOT NULL AND status IN ('queued', 'downloading')
          )
          AND media_item_id NOT IN (
              SELECT DISTINCT media_item_id
              FROM downloads
              WHERE media_item_id IS NOT NULL
                AND episode_id IS NULL
                AND status IN ('queued', 'downloading')
          )
        "#,
    )
    .execute(db)
    .await?;

    // =========================================================================
    // 1. Wanted Movies
    // =========================================================================
    let wanted_movies = sqlx::query_as::<_, WantedMovieRecord>(
        r#"
        SELECT id, tmdb_id, title, year, quality_profile_id
        FROM media_items
        WHERE media_type = 'movie'
          AND status = 'requested'
          AND id NOT IN (
              SELECT DISTINCT media_item_id
              FROM downloads
              WHERE media_item_id IS NOT NULL AND status IN ('queued', 'downloading')
          )
        "#,
    )
    .fetch_all(db)
    .await?;

    for movie in wanted_movies {
        let query = match movie.year {
            Some(year) => format!("{} {}", movie.title, year),
            None => movie.title.clone(),
        };

        let profile = load_quality_profile_for_media(db, movie.quality_profile_id).await?;
        // Pass `None` for `imdb_id` on movie searches (EZTV is TV-focused, and we aren't
        // selecting `imdb_id` on `WantedMovieRecord` here).
        let releases: Vec<ReleaseCandidate> = search_all_indexers_with_profile(
            db,
            http,
            &query,
            profile.as_ref().map(|p| (p, true)),
            None,
        )
        .await?;

        if let Some(best_release) = releases.into_iter().next() {
            if has_active_download_title(db, Some(movie.id), None, &best_release.title).await? {
                println!(
                    "[WORKER] -> Skipping already-active release '{}' for movie '{}'",
                    best_release.title, movie.title
                );
                continue;
            }

            println!(
                "[WORKER] -> Found movie match: {} [{}]",
                best_release.title, best_release.quality_name
            );

            queue_download_and_update_status(
                db,
                downloader,
                Some(movie.id),
                None,
                &best_release.title,
                &best_release.download_url,
            )
            .await?;

            sqlx::query("UPDATE media_items SET status = 'queued' WHERE id = ?")
                .bind(movie.id)
                .execute(db)
                .await?;
        }
    }

    // =========================================================================
    // 2. Wanted TV Shows (Season Pack first -> Individual Episode fallback)
    // =========================================================================
    // Include `m.imdb_id` so the column order matches `WantedSeasonSummary`:
    // (media_item_id, series_title, imdb_id, season_number, quality_profile_id).
    let missing_seasons = sqlx::query_as::<_, WantedSeasonSummary>(
        r#"
        SELECT DISTINCT m.id AS media_item_id, m.title AS series_title, m.imdb_id,
                        e.season_number, m.quality_profile_id
        FROM episodes e
        JOIN media_items m ON m.id = e.media_item_id
        WHERE e.monitored = 1
          AND e.status = 'missing'
          AND e.season_number > 0
          AND e.id NOT IN (
              SELECT DISTINCT episode_id
              FROM downloads
              WHERE episode_id IS NOT NULL AND status IN ('queued', 'downloading')
          )
        ORDER BY m.title ASC, e.season_number ASC
        "#,
    )
    .fetch_all(db)
    .await?;

    for season_group in missing_seasons {
        let profile =
            load_quality_profile_for_media(db, season_group.quality_profile_id).await?;

        // Strategy A: Season Pack (e.g., "Show Name S01")
        let season_query = format!(
            "{} S{:02}",
            season_group.series_title, season_group.season_number
        );
        println!("[WORKER] Searching for Season Pack: {}", season_query);

        // `season_group.imdb_id` is `Option<i64>` (which implements `Copy`), so passing it here
        // does not move it—we can still use it in the episode loop below.
        let season_releases: Vec<ReleaseCandidate> = search_all_indexers_with_profile(
            db,
            http,
            &season_query,
            profile.as_ref().map(|p| (p, false)),
            season_group.imdb_id,
        )
        .await?;

        let best_season_pack = season_releases.into_iter().find(|r| {
            matches!(
                parse_release_title(&r.title),
                ParsedMedia::SeasonPack { season, .. } if season == season_group.season_number
            )
        });

        if let Some(pack) = best_season_pack {
            if has_active_download_title(
                db,
                Some(season_group.media_item_id),
                None,
                &pack.title,
            )
            .await?
            {
                println!(
                    "[WORKER] -> Skipping already-active Season Pack '{}'",
                    pack.title
                );
                continue;
            }

            println!(
                "[WORKER] -> Found Season Pack match: {} [{}]",
                pack.title, pack.quality_name
            );

            // If queueing the season pack times out or errors, log it and fall through
            // to individual episodes rather than aborting the entire worker run.
            match queue_download_and_update_status(
                db,
                downloader,
                Some(season_group.media_item_id),
                None,
                &pack.title,
                &pack.download_url,
            )
            .await
            {
                Ok(()) => {
                    sqlx::query(
                        r#"
                        UPDATE episodes
                        SET status = 'queued'
                        WHERE media_item_id = ? AND season_number = ? AND monitored = 1 AND status = 'missing'
                        "#,
                    )
                    .bind(season_group.media_item_id)
                    .bind(season_group.season_number)
                    .execute(db)
                    .await?;

                    continue;
                }
                Err(err) => {
                    eprintln!(
                        "[WORKER] -> Failed to queue Season Pack '{}': {err:#}. Falling back to individual episodes.",
                        pack.title
                    );
                }
            }
        }

        // Strategy B: Fallback to individual missing episodes in this season
        println!(
            "[WORKER] No season pack found for {}, falling back to individual episodes.",
            season_query
        );

        let missing_episodes = sqlx::query_as::<_, WantedEpisodeRecord>(
            r#"
            SELECT id AS episode_id, season_number, episode_number
            FROM episodes
            WHERE media_item_id = ?
              AND season_number = ?
              AND monitored = 1
              AND status = 'missing'
              AND id NOT IN (
                  SELECT DISTINCT episode_id
                  FROM downloads
                  WHERE episode_id IS NOT NULL AND status IN ('queued', 'downloading')
              )
            ORDER BY episode_number ASC
            "#,
        )
        .bind(season_group.media_item_id)
        .bind(season_group.season_number)
        .fetch_all(db)
        .await?;

        for ep in missing_episodes {
            let ep_query = format!(
                "{} S{:02}E{:02}",
                season_group.series_title, ep.season_number, ep.episode_number
            );
            println!("[WORKER] Searching for episode: {}", ep_query);

            let ep_releases: Vec<ReleaseCandidate> = search_all_indexers_with_profile(
                db,
                http,
                &ep_query,
                profile.as_ref().map(|p| (p, false)),
                season_group.imdb_id,
            )
            .await?;

            let best_ep_release = ep_releases.into_iter().find(|r| {
                matches!(
                    parse_release_title(&r.title),
                    ParsedMedia::Episode {
                        season, episode, ..
                    } if season == ep.season_number && episode == ep.episode_number
                )
            });

            if let Some(best_ep) = best_ep_release {
                if has_active_download_title(
                    db,
                    Some(season_group.media_item_id),
                    Some(ep.episode_id),
                    &best_ep.title,
                )
                .await?
                {
                    println!(
                        "[WORKER] -> Skipping already-active episode release '{}'",
                        best_ep.title
                    );
                    continue;
                }

                println!(
                    "[WORKER] -> Found episode match: {} [{}]",
                    best_ep.title, best_ep.quality_name
                );

                // Only mark the episode `queued` and insert into `downloads` if
                // `queue_download_and_update_status` succeeds; on timeout/error, log
                // and continue to the next missing episode.
                if let Err(err) = queue_download_and_update_status(
                    db,
                    downloader,
                    Some(season_group.media_item_id),
                    Some(ep.episode_id),
                    &best_ep.title,
                    &best_ep.download_url,
                )
                .await
                {
                    eprintln!(
                        "[WORKER] -> Failed to queue episode '{}' (S{:02}E{:02}): {err:#}",
                        best_ep.title, ep.season_number, ep.episode_number
                    );
                    continue;
                }

                sqlx::query("UPDATE episodes SET status = 'queued' WHERE id = ?")
                    .bind(ep.episode_id)
                    .execute(db)
                    .await?;
            }
        }
    }

    println!("[WORKER] Automated wanted search completed.");
    Ok(())
}

// ============================================================================
// Helper Structs & Database Functions
// ============================================================================

#[derive(sqlx::FromRow)]
struct WantedMovieRecord {
    id: i64,
    #[allow(dead_code)]
    tmdb_id: i64,
    title: String,
    year: Option<i32>,
    quality_profile_id: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct WantedSeasonSummary {
    pub media_item_id: i64,
    pub series_title: String,
    pub imdb_id: Option<i64>,
    pub season_number: i32,
    pub quality_profile_id: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct WantedEpisodeRecord {
    episode_id: i64,
    season_number: i32,
    episode_number: i32,
}

#[derive(sqlx::FromRow)]
struct QualityProfileHeaderRow {
    id: i64,
    name: String,
    upgrade_allowed: bool,
    cutoff_quality: String,
}

async fn load_quality_profile_for_media(
    db: &SqlitePool,
    profile_id: Option<i64>,
) -> anyhow::Result<Option<QualityProfile>> {
    let header: Option<QualityProfileHeaderRow> = match profile_id {
        Some(id) => {
            sqlx::query_as(
                r#"
                SELECT id, name, upgrade_allowed, cutoff_quality
                FROM quality_profiles
                WHERE id = ?
                "#,
            )
            .bind(id)
            .fetch_optional(db)
            .await?
        }
        None => {
            sqlx::query_as(
                r#"
                SELECT id, name, upgrade_allowed, cutoff_quality
                FROM quality_profiles
                WHERE name = 'HD'
                LIMIT 1
                "#,
            )
            .fetch_optional(db)
            .await?
        }
    };

    let Some(h) = header else {
        return Ok(None);
    };

    let items: Vec<QualityProfileItem> = sqlx::query_as(
        r#"
        SELECT quality, allowed, rank, max_movie_size_mb, max_episode_size_mb
        FROM quality_profile_items
        WHERE profile_id = ?
        ORDER BY rank DESC
        "#,
    )
    .bind(h.id)
    .fetch_all(db)
    .await?;

    Ok(Some(QualityProfile {
        id: h.id,
        name: h.name,
        upgrade_allowed: h.upgrade_allowed,
        cutoff_quality: h.cutoff_quality,
        items,
    }))
}

async fn has_active_download_title(
    db: &SqlitePool,
    media_item_id: Option<i64>,
    episode_id: Option<i64>,
    title: &str,
) -> anyhow::Result<bool> {
    let existing: Option<i64> = sqlx::query_scalar(
        r#"
        SELECT id
        FROM downloads
        WHERE status IN ('queued', 'downloading')
          AND title = ?
          AND (? IS NULL OR media_item_id = ?)
          AND (? IS NULL OR episode_id = ?)
        LIMIT 1
        "#,
    )
    .bind(title)
    .bind(media_item_id)
    .bind(media_item_id)
    .bind(episode_id)
    .bind(episode_id)
    .fetch_optional(db)
    .await?;

    Ok(existing.is_some())
}

async fn queue_download_and_update_status(
    db: &SqlitePool,
    downloader: &Arc<dyn DownloadClient>,
    media_item_id: Option<i64>,
    episode_id: Option<i64>,
    title: &str,
    download_url: &str,
) -> anyhow::Result<()> {
    println!(
        "[DOWNLOADER] Adding release '{}' (url_prefix='{}')",
        title,
        &download_url.chars().take(60).collect::<String>()
    );

    // `tokio::time::timeout` returns `Result<Result<String, ...>, tokio::time::error::Elapsed>`:
    // - Outer `Err(_)` = the 15-second timer expired before `add_release` finished.
    // - Inner `Err(e)` = `add_release` finished in time but returned its own error.
    let info_hash = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        downloader.add_release(download_url),
    )
    .await
    .map_err(|_| anyhow::anyhow!("add_release timed out after 15s for '{}'", title))?
    .map_err(|e| anyhow::anyhow!("add_release failed for '{}': {e:#}", title))?;

    sqlx::query(
        r#"
        INSERT INTO downloads (media_item_id, episode_id, protocol, download_id, title, status)
        VALUES (?, ?, 'torrent', ?, ?, 'downloading')
        "#,
    )
    .bind(media_item_id)
    .bind(episode_id)
    .bind(&info_hash)
    .bind(title)
    .execute(db)
    .await?;

    Ok(())
}

// ============================================================================
// Worker: Import Completed Downloads, Move Subtitles, Clean Downloads & Rescan
// ============================================================================

#[derive(sqlx::FromRow)]
struct DownloadRecord {
    #[allow(dead_code)]
    id: i64,
    media_item_id: Option<i64>,
    episode_id: Option<i64>,
    download_id: String,
    title: String,
    #[allow(dead_code)]
    protocol: String,
}

#[derive(sqlx::FromRow)]
struct MediaRootInfo {
    media_type: String,
    title: String,
    year: Option<i32>,
    root_folder: Option<String>,
}

/// Builds a fallback `NAME (YEAR)` folder path when `media_items.root_folder` is `None`.
fn build_default_media_root(
    media_type: &str,
    title: &str,
    year: Option<i32>,
    movies_dir: &Path,
    shows_dir: &Path,
) -> PathBuf {
    let base = if media_type == "tv" {
        shows_dir
    } else {
        movies_dir
    };

    let raw_name = match year {
        Some(y) => format!("{} ({})", title.trim(), y),
        None => title.trim().to_string(),
    };

    base.join(sanitize_filename::sanitize(&raw_name))
}

/// Checks if `dir` already contains at least one video file (used when a download's
/// source path in `downloads_dir` was already moved on an earlier pass).
async fn has_any_video_in_dir(dir: &Path) -> anyhow::Result<bool> {
    if !fs::try_exists(dir).await? {
        return Ok(false);
    }
    let mut videos = Vec::new();
    collect_files_by_extensions(dir, &VIDEO_EXTENSIONS, &mut videos).await?;
    Ok(!videos.is_empty())
}

/// Checks active downloads (optionally filtered to a single `info_hash`), moves completed
/// video + subtitle files into `root_folder`, deletes remaining download files for that release,
/// removes the torrent from the client, rescans `root_folder`, and removes the `downloads` row.
pub async fn process_completed_downloads_for(
    db: &SqlitePool,
    http: &Client,
    tmdb_api_key: &Arc<RwLock<String>>,
    downloader: &Arc<dyn DownloadClient>,
    downloads_dir: &Path,
    movies_dir: &Path,
    shows_dir: &Path,
    filter_hash: Option<&str>,
) -> anyhow::Result<()> {
    let active_records = sqlx::query_as::<_, DownloadRecord>(
        r#"
        SELECT id, media_item_id, episode_id, download_id, title, protocol
        FROM downloads
        WHERE status = 'downloading'
        "#,
    )
    .fetch_all(db)
    .await?;

    let records_to_check: Vec<DownloadRecord> = match filter_hash {
        Some(hash) => active_records
            .into_iter()
            .filter(|r| r.download_id.eq_ignore_ascii_case(hash))
            .collect(),
        None => active_records,
    };

    if records_to_check.is_empty() {
        return Ok(());
    }

    let live_downloads = downloader.list_downloads().await?;

    for record in records_to_check {
        let Some(live) = live_downloads
            .iter()
            .find(|d| d.id.eq_ignore_ascii_case(&record.download_id))
        else {
            continue;
        };

        let is_done =
            live.state.eq_ignore_ascii_case("completed") || live.progress_pct >= 100.0;
        if !is_done {
            continue;
        }

        println!("[IMPORT] Completed download detected: '{}'", record.title);

        let mut imported_any_media = false;

        if let Some(media_id) = record.media_item_id {
            let media_info: Option<MediaRootInfo> = sqlx::query_as(
                "SELECT media_type, title, year, root_folder FROM media_items WHERE id = ?",
            )
            .bind(media_id)
            .fetch_optional(db)
            .await?;

            if let Some(info) = media_info {
                let media_root = match info.root_folder.as_deref().map(str::trim) {
                    Some(existing) if !existing.is_empty() => PathBuf::from(existing),
                    _ => build_default_media_root(
                        &info.media_type,
                        &info.title,
                        info.year,
                        movies_dir,
                        shows_dir,
                    ),
                };

                fs::create_dir_all(&media_root).await?;

                let root_str = media_root.to_string_lossy().to_string();
                sqlx::query("UPDATE media_items SET root_folder = ? WHERE id = ?")
                    .bind(&root_str)
                    .bind(media_id)
                    .execute(db)
                    .await?;

                let source_path = resolve_download_source(downloads_dir, live, &record.title);

                if let Some(ref src) = source_path {
                    if info.media_type == "movie" {
                        if let Some(moved_file) =
                            move_largest_video_and_subtitles(src, &media_root).await?
                        {
                            println!(
                                "[IMPORT] Moved movie file to: {}",
                                moved_file.display()
                            );
                            imported_any_media = true;
                        } else {
                            println!(
                                "[IMPORT] WARNING: No video file found in '{}' for '{}'",
                                src.display(),
                                record.title
                            );
                        }
                    } else if info.media_type == "tv" {
                        if let Some(ep_id) = record.episode_id {
                            let ep_info: Option<(i32, i32)> = sqlx::query_as(
                                "SELECT season_number, episode_number FROM episodes WHERE id = ?",
                            )
                            .bind(ep_id)
                            .fetch_optional(db)
                            .await?;

                            if let Some((season_num, _ep_num)) = ep_info {
                                let season_dir =
                                    media_root.join(format!("Season {:02}", season_num));
                                fs::create_dir_all(&season_dir).await?;

                                if let Some(moved_file) =
                                    move_largest_video_and_subtitles(src, &season_dir).await?
                                {
                                    println!(
                                        "[IMPORT] Moved episode file to: {}",
                                        moved_file.display()
                                    );
                                    imported_any_media = true;
                                }
                            }
                        } else {
                            let fallback_season = match parse_release_title(&record.title) {
                                ParsedMedia::SeasonPack { season, .. } => Some(season),
                                ParsedMedia::Episode { season, .. } => Some(season),
                                _ => None,
                            };

                            let moved_files = move_tv_pack_videos_and_subtitles(
                                src,
                                &media_root,
                                fallback_season,
                            )
                            .await?;

                            if !moved_files.is_empty() {
                                println!(
                                    "[IMPORT] Moved {} TV file(s) for '{}'",
                                    moved_files.len(),
                                    record.title
                                );
                                imported_any_media = true;
                            } else {
                                println!(
                                    "[IMPORT] WARNING: No TV video files found in '{}' for '{}'",
                                    src.display(),
                                    record.title
                                );
                            }
                        }
                    }

                    if imported_any_media {
                        cleanup_download_source(downloads_dir, src).await?;

                        if let Err(rem_err) =
                            downloader.remove_download(&record.download_id, false).await
                        {
                            eprintln!(
                                "[IMPORT] Warning removing completed torrent '{}' from client: {}",
                                record.download_id, rem_err
                            );
                        }
                    }
                } else {
                    let already_in_library = if info.media_type == "tv" {
                        if let Some(ep_id) = record.episode_id {
                            let ep_info: Option<(i32, i32)> = sqlx::query_as(
                                "SELECT season_number, episode_number FROM episodes WHERE id = ?",
                            )
                            .bind(ep_id)
                            .fetch_optional(db)
                            .await?;

                            if let Some((season_num, _)) = ep_info {
                                has_any_video_in_dir(
                                    &media_root.join(format!("Season {:02}", season_num)),
                                )
                                .await?
                            } else {
                                has_any_video_in_dir(&media_root).await?
                            }
                        } else {
                            has_any_video_in_dir(&media_root).await?
                        }
                    } else {
                        has_any_video_in_dir(&media_root).await?
                    };

                    if already_in_library {
                        println!(
                            "[IMPORT] Media already present in '{}'; cleaning up stale download entry '{}'",
                            root_str, record.title
                        );
                        if let Err(rem_err) =
                            downloader.remove_download(&record.download_id, false).await
                        {
                            eprintln!(
                                "[IMPORT] Warning removing completed torrent '{}' from client: {}",
                                record.download_id, rem_err
                            );
                        }
                        imported_any_media = true;
                    } else {
                        println!(
                            "[IMPORT] WARNING: Could not locate downloaded files in '{}' for '{}'",
                            downloads_dir.display(),
                            record.title
                        );
                    }
                }

                // Detect quality from the torrent title if known so it isn't lost if the
                // moved file on disk has a generic/stripped name, while never overwriting with Unknown.
                let torrent_quality = if info.media_type == "movie" {
                    Some(parse_quality(&record.title))
                        .filter(|&q| q != Quality::Unknown)
                        .map(|q| q.as_str().to_string())
                } else {
                    None
                };

                let tmdb_key = tmdb_api_key.read().await.clone();
                if let Err(scan_err) = scan_directory(db, http, &tmdb_key, &root_str).await {
                    eprintln!(
                        "[IMPORT] Rescan warning for '{}': {}",
                        root_str, scan_err
                    );
                }

                if let Some(ref q_str) = torrent_quality {
                    sqlx::query(
                        r#"
                        UPDATE media_items
                        SET current_quality = COALESCE(?, current_quality)
                        WHERE id = ?
                        "#,
                    )
                    .bind(q_str)
                    .bind(media_id)
                    .execute(db)
                    .await?;
                }
            }
        } else {
            imported_any_media = true;
        }

        if imported_any_media {
            sqlx::query("DELETE FROM downloads WHERE download_id = ?")
                .bind(&record.download_id)
                .execute(db)
                .await?;

            println!("[IMPORT] Finished importing '{}'", record.title);
        } else {
            println!(
                "[IMPORT] Keeping download record '{}' for retry on next tick",
                record.title
            );
        }
    }

    Ok(())
}

fn normalize_match_key(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// Resolves the specific file or torrent directory inside `downloads_dir`.
fn resolve_download_source(
    downloads_dir: &Path,
    live: &DownloadStatus,
    record_title: &str,
) -> Option<PathBuf> {
    // 1. Exact matches on live download name or record title
    let by_live_name = downloads_dir.join(&live.name);
    if by_live_name.exists() {
        return Some(by_live_name);
    }

    let by_record_title = downloads_dir.join(record_title);
    if by_record_title.exists() {
        return Some(by_record_title);
    }

    // 2. Exact match with common video extensions appended
    for candidate_name in [&live.name, record_title] {
        for ext in VIDEO_EXTENSIONS {
            let with_ext = downloads_dir.join(format!("{}.{}", candidate_name, ext));
            if with_ext.exists() {
                return Some(with_ext);
            }
        }
    }

    // 3. Normalized alphanumeric comparison against entries directly in `downloads_dir`
    //    (handles sanitized characters like `:` -> `_` or dots/spaces differences).
    let target_keys: Vec<String> = [&live.name, record_title]
        .into_iter()
        .map(|s| normalize_match_key(s))
        .filter(|k| !k.is_empty())
        .collect();

    let entries: Vec<PathBuf> = std::fs::read_dir(downloads_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();

    for path in &entries {
        let Some(stem_or_name) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let entry_key = normalize_match_key(stem_or_name);
        if entry_key.is_empty() {
            continue;
        }

        if target_keys
            .iter()
            .any(|tk| tk.contains(&entry_key) || entry_key.contains(tk))
        {
            return Some(path.clone());
        }
    }

    // 4. Last resort for root-level single video files when nothing else matched
    let root_videos: Vec<PathBuf> = entries
        .into_iter()
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| VIDEO_EXTENSIONS.contains(&e.to_lowercase().as_str()))
                    .unwrap_or(false)
        })
        .collect();

    if root_videos.len() == 1 {
        return root_videos.into_iter().next();
    }

    None
}

/// Moves the largest video file and matching subtitle files from `source` into `dest_dir`.
async fn move_largest_video_and_subtitles(
    source: &Path,
    dest_dir: &Path,
) -> anyhow::Result<Option<PathBuf>> {
    if !fs::try_exists(source).await? {
        return Ok(None);
    }

    let Some(largest_video) = find_largest_video(source).await? else {
        return Ok(None);
    };

    let moved_video = move_single_file(&largest_video, dest_dir).await?;
    move_matching_subtitles_for_video(source, &largest_video, dest_dir).await?;

    Ok(Some(moved_video))
}

/// Moves all video files in a TV season/multi-episode release into `Season XX` folders
/// along with subtitles matching each video file.
async fn move_tv_pack_videos_and_subtitles(
    source: &Path,
    media_root: &Path,
    fallback_season: Option<i32>,
) -> anyhow::Result<Vec<PathBuf>> {
    if !fs::try_exists(source).await? {
        return Ok(Vec::new());
    }

    let mut video_files = Vec::new();
    collect_files_by_extensions(source, &VIDEO_EXTENSIONS, &mut video_files).await?;
    video_files.sort();

    let mut moved_videos = Vec::new();

    for video_path in video_files {
        let file_name_str = video_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();

        let season_num = match parse_release_title(file_name_str) {
            ParsedMedia::Episode { season, .. } => Some(season),
            ParsedMedia::SeasonPack { season, .. } => Some(season),
            _ => fallback_season,
        };

        let dest_dir = match season_num {
            Some(s) => media_root.join(format!("Season {:02}", s)),
            None => media_root.to_path_buf(),
        };

        fs::create_dir_all(&dest_dir).await?;
        let moved_video = move_single_file(&video_path, &dest_dir).await?;
        move_matching_subtitles_for_video(source, &video_path, &dest_dir).await?;
        moved_videos.push(moved_video);
    }

    Ok(moved_videos)
}

/// Moves subtitle files within `source` that match `video_path` by stem or `SxxExx` tag.
async fn move_matching_subtitles_for_video(
    source: &Path,
    video_path: &Path,
    dest_dir: &Path,
) -> anyhow::Result<()> {
    if !fs::try_exists(source).await? {
        return Ok(());
    }

    let Some(video_stem) = video_path.file_stem().and_then(|s| s.to_str()) else {
        return Ok(());
    };
    let video_stem_lower = video_stem.to_lowercase();
    let video_ep_tag = extract_episode_tag(&video_stem_lower);

    let mut subtitle_files = Vec::new();
    collect_files_by_extensions(source, &SUBTITLE_EXTENSIONS, &mut subtitle_files).await?;

    for sub_path in subtitle_files {
        let Some(sub_stem) = sub_path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let sub_stem_lower = sub_stem.to_lowercase();

        let exact_or_lang_match = sub_stem_lower == video_stem_lower
            || sub_stem_lower.starts_with(&format!("{video_stem_lower}."));

        let episode_tag_match = match (video_ep_tag.as_deref(), extract_episode_tag(&sub_stem_lower)) {
            (Some(v_tag), Some(s_tag)) => v_tag == s_tag,
            _ => false,
        };

        if exact_or_lang_match || episode_tag_match {
            if let Ok(moved_sub) = move_single_file(&sub_path, dest_dir).await {
                println!("[IMPORT] Moved subtitle file to: {}", moved_sub.display());
            }
        }
    }

    Ok(())
}

/// Extracts a normalized `sXXeYY` tag if present in a lowercase filename/stem.
fn extract_episode_tag(lower_name: &str) -> Option<String> {
    let bytes = lower_name.as_bytes();
    for i in 0..bytes.len().saturating_sub(5) {
        if bytes[i] == b's'
            && bytes[i + 1].is_ascii_digit()
            && bytes[i + 2].is_ascii_digit()
            && bytes[i + 3] == b'e'
            && bytes[i + 4].is_ascii_digit()
            && bytes[i + 5].is_ascii_digit()
        {
            return Some(lower_name[i..i + 6].to_string());
        }
    }
    None
}

fn is_cross_device_error(err: &std::io::Error) -> bool {
    if err.kind() == ErrorKind::CrossesDevices {
        return true;
    }
    #[cfg(unix)]
    if err.raw_os_error() == Some(18) {
        return true;
    }
    false
}

/// Moves a single file into `dest_dir`, falling back to copy + remove on cross-device moves.
async fn move_single_file(src_file: &Path, dest_dir: &Path) -> anyhow::Result<PathBuf> {
    let file_name = src_file
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Missing file name for {}", src_file.display()))?;
    let target_path = dest_dir.join(file_name);

    match fs::rename(src_file, &target_path).await {
        Ok(()) => Ok(target_path),
        Err(err) if is_cross_device_error(&err) => {
            if let Err(copy_err) = fs::copy(src_file, &target_path).await {
                let _ = fs::remove_file(&target_path).await;
                return Err(copy_err.into());
            }
            fs::remove_file(src_file).await?;
            Ok(target_path)
        }
        Err(err) => Err(err.into()),
    }
}

/// Deletes remaining files/folders for a completed release inside `downloads_dir`.
async fn cleanup_download_source(downloads_dir: &Path, source: &Path) -> anyhow::Result<()> {
    if !fs::try_exists(source).await? {
        return Ok(());
    }

    let norm_downloads =
        std::fs::canonicalize(downloads_dir).unwrap_or_else(|_| downloads_dir.to_path_buf());
    let norm_source = std::fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());

    if norm_source == norm_downloads || !norm_source.starts_with(&norm_downloads) {
        eprintln!(
            "[IMPORT] Safety guard blocked cleanup for '{}' (downloads root: '{}')",
            norm_source.display(),
            norm_downloads.display()
        );
        return Ok(());
    }

    let meta = fs::symlink_metadata(source).await?;
    if meta.is_dir() {
        fs::remove_dir_all(source).await?;
        println!(
            "[IMPORT] Removed remaining download folder: {}",
            source.display()
        );
    } else if meta.is_file() {
        fs::remove_file(source).await?;
        println!(
            "[IMPORT] Removed remaining download file: {}",
            source.display()
        );
    }

    Ok(())
}

/// Recursively collects all files under `root` whose lowercase extension matches `exts`.
async fn collect_files_by_extensions(
    root: &Path,
    exts: &[&str],
    out: &mut Vec<PathBuf>,
) -> anyhow::Result<()> {
    let meta = fs::symlink_metadata(root).await?;
    if meta.is_file() {
        if has_extension(root, exts) {
            out.push(root.to_path_buf());
        }
        return Ok(());
    }
    if !meta.is_dir() {
        return Ok(());
    }

    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries = fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let entry_meta = fs::symlink_metadata(&path).await?;
            if entry_meta.is_dir() {
                stack.push(path);
            } else if entry_meta.is_file() && has_extension(&path, exts) {
                out.push(path);
            }
        }
    }

    Ok(())
}

async fn find_largest_video(path: &Path) -> anyhow::Result<Option<PathBuf>> {
    let meta = fs::symlink_metadata(path).await?;

    if meta.is_file() {
        return if has_extension(path, &VIDEO_EXTENSIONS) {
            Ok(Some(path.to_path_buf()))
        } else {
            Ok(None)
        };
    }

    if !meta.is_dir() {
        return Ok(None);
    }

    let mut best_path: Option<PathBuf> = None;
    let mut best_size: u64 = 0;

    let mut stack = vec![path.to_path_buf()];
    while let Some(current_dir) = stack.pop() {
        let mut entries = fs::read_dir(&current_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let entry_path = entry.path();
            let entry_meta = fs::symlink_metadata(&entry_path).await?;

            if entry_meta.is_dir() {
                stack.push(entry_path);
            } else if entry_meta.is_file() && has_extension(&entry_path, &VIDEO_EXTENSIONS) {
                let size = entry_meta.len();
                if size > best_size {
                    best_size = size;
                    best_path = Some(entry_path);
                }
            }
        }
    }

    Ok(best_path)
}

fn has_extension(path: &Path, exts: &[&str]) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| exts.contains(&ext.to_lowercase().as_str()))
        .unwrap_or(false)
}

// ============================================================================
// Background Worker Loop
// ============================================================================

pub async fn start_background_worker(
    db: SqlitePool,
    http: Client,
    tmdb_api_key: Arc<RwLock<String>>,
    downloader: Arc<dyn DownloadClient>,
) {
    let mut interval_timer = tokio::time::interval(std::time::Duration::from_secs(60));
    let mut wanted_search_timer = tokio::time::interval(std::time::Duration::from_secs(3600));

    let downloads_dir = PathBuf::from(
        std::env::var("DOWNLOAD_DIR").unwrap_or_else(|_| "./library/downloads".to_string()),
    );
    let movies_dir = PathBuf::from(
        std::env::var("MOVIES_DIR").unwrap_or_else(|_| "./library/movies".to_string()),
    );
    let shows_dir = PathBuf::from(
        std::env::var("SHOWS_DIR").unwrap_or_else(|_| "./library/shows".to_string()),
    );

    loop {
        tokio::select! {
            _ = interval_timer.tick() => {
                if let Err(e) = process_completed_downloads_for(
                    &db,
                    &http,
                    &tmdb_api_key,
                    &downloader,
                    &downloads_dir,
                    &movies_dir,
                    &shows_dir,
                    None,
                )
                .await
                {
                    eprintln!("[WORKER] Error processing completed downloads: {}", e);
                }
            }
            _ = wanted_search_timer.tick() => {
                if let Err(e) = run_automated_wanted_search(&db, &http, &downloader).await {
                    eprintln!("[WORKER] Error running automated wanted search: {}", e);
                }
            }
        }
    }
}