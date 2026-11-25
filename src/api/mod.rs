use std::collections::BTreeMap;
use std::path::{Path as StdPath, PathBuf};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, patch, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::{
    downloaders::DownloadStatus,
    indexers::{search_all_indexers_with_profile, IndexerConfig, ReleaseCandidate},
    metadata::{fetch_imdb_id, fetch_tv_episodes, search_tmdb, TmdbSearchResult},
    models::{Quality, QualityProfile, QualityProfileItem},
    parser::{scan_directory, ScanSummary},
    vpn::{load_vpn_settings, VpnStatusResponse},
    worker::run_automated_wanted_search,
    AppState,
};

// ============================================================================
// Payloads & DTOs
// ============================================================================

#[derive(Deserialize)]
pub struct UpdateVpnPayload {
    pub enabled: bool,
    pub provider_name: String,
    pub wg_config: String,
}

#[derive(Deserialize)]
pub struct UpdateTmdbPayload {
    pub api_key: String,
}

#[derive(Serialize)]
pub struct TmdbSettingsResponse {
    pub configured: bool,
    pub masked_key: Option<String>,
}

#[derive(Deserialize)]
pub struct AddIndexerPayload {
    pub name: String,
    pub url: String,
    pub driver: String,
    pub api_key: String,
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
pub struct UpdateIndexerPayload {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub driver: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    #[serde(default)]
    imdb_id: Option<i64>,
}

#[derive(Deserialize)]
pub struct CreateRequestPayload {
    pub tmdb_id: i64,
    pub media_type: String, // "movie" or "tv"
    pub title: String,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub requested_by: Option<String>,
    pub quality_profile_id: Option<i64>,
    pub seasons: Option<Vec<i32>>,
}

#[derive(Serialize)]
pub struct SeasonSummary {
    pub season_number: i32,
    pub episode_count: usize,
}

#[derive(Serialize)]
pub struct RequestResponse {
    pub media_item_id: i64,
    pub status: String,
}

#[derive(Deserialize)]
pub struct ScanPayload {
    pub path: String,
}

#[derive(Serialize, FromRow, Debug, Clone)]
pub struct LibraryItem {
    pub id: i64,
    pub tmdb_id: i64,
    pub imdb_id: Option<i64>,
    pub media_type: String,
    pub title: String,
    pub year: Option<i32>,
    pub poster_path: Option<String>,
    pub status: String,
    pub root_folder: Option<String>,
    pub quality_profile_id: Option<i64>,
}

#[derive(Serialize, FromRow)]
pub struct LibrarySeason {
    pub season_number: i32,
    pub monitored: bool,
    pub total_episodes: i64,
    pub available_episodes: i64,
}

#[derive(Deserialize)]
pub struct ToggleSeasonMonitorPayload {
    pub monitored: bool,
}

#[derive(Serialize, FromRow)]
pub struct EpisodeItem {
    pub id: i64,
    pub season_number: i32,
    pub episode_number: i32,
    pub title: Option<String>,
    pub air_date: Option<String>,
    pub status: String,
    pub file_path: Option<String>,
}

#[derive(Serialize)]
pub struct WantedQueue {
    pub movies: Vec<LibraryItem>,
    pub episodes: Vec<WantedEpisode>,
}

#[derive(Serialize, FromRow)]
pub struct WantedEpisode {
    pub episode_id: i64,
    pub media_item_id: i64,
    pub tmdb_id: i64,
    pub series_title: String,
    pub season_number: i32,
    pub episode_number: i32,
    pub episode_title: Option<String>,
}

#[derive(Deserialize)]
pub struct StartDownloadPayload {
    pub media_item_id: Option<i64>,
    pub episode_id: Option<i64>,
    pub title: String,
    pub download_url: String,
}

#[derive(Serialize)]
pub struct StartDownloadResponse {
    pub download_id: String,
    pub status: String,
}

#[derive(Deserialize)]
pub struct SaveQualityProfilePayload {
    pub name: String,
    pub upgrade_allowed: bool,
    pub cutoff_quality: String,
    pub items: Vec<QualityProfileItem>,
}

#[derive(Deserialize)]
pub struct AssignProfilePayload {
    pub quality_profile_id: Option<i64>,
}

#[derive(FromRow)]
struct QualityProfileRow {
    id: i64,
    name: String,
    upgrade_allowed: bool,
    cutoff_quality: String,
}

// ============================================================================
// Router
// ============================================================================

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/search", get(handle_search))
        .route("/tv/:tmdb_id/seasons", get(handle_tv_seasons))
        .route("/requests", post(handle_create_request))
        .route("/scan", post(handle_scan))
        .route("/library", get(handle_library))
        .route("/library/:id", delete(handle_delete_library_item))
        .route("/library/:id/episodes", get(handle_series_episodes))
        .route("/library/:id/seasons", get(handle_library_seasons))
        .route("/library/:id/seasons/:season_number/monitor", patch(handle_toggle_season_monitor),)
        .route("/library/:id/quality-profile", patch(handle_assign_media_profile))
        .route("/wanted", get(handle_wanted))
        .route("/wanted/search", post(handle_trigger_wanted_search))
        .route("/indexers", get(handle_list_indexers).post(handle_add_indexer))
        .route("/indexers/:id", put(handle_update_indexer).delete(handle_delete_indexer),)
        .route("/releases", get(handle_search_releases))
        .route("/downloads", get(handle_list_downloads).post(handle_start_download))
        .route("/downloads/:id", delete(handle_remove_download))
        .route("/vpn", get(handle_get_vpn).post(handle_update_vpn))
        .route("/settings/tmdb", get(handle_get_tmdb_settings).post(handle_update_tmdb_settings))
        .route("/qualities", get(handle_list_available_qualities))
        .route("/quality-profiles", get(handle_list_profiles).post(handle_create_profile))
        .route("/quality-profiles/:id", put(handle_update_profile).delete(handle_delete_profile))
}

// ============================================================================
// Helpers
// ============================================================================

/// Creates `<movies_dir>/<NAME (YEAR)>` or `<shows_dir>/<NAME (YEAR)>` on disk
/// and returns its string path for `media_items.root_folder`.
pub async fn ensure_media_folder(
    movies_dir: &str,
    shows_dir: &str,
    media_type: &str,
    title: &str,
    year: Option<i32>,
) -> anyhow::Result<String> {
    let base_dir = if media_type == "tv" {
        shows_dir
    } else {
        movies_dir
    };

    let raw_name = match year {
        Some(y) => format!("{} ({})", title.trim(), y),
        None => title.trim().to_string(),
    };

    let safe_name = sanitize_filename::sanitize(&raw_name);
    let full_path = PathBuf::from(base_dir).join(safe_name);

    tokio::fs::create_dir_all(&full_path).await?;
    Ok(full_path.to_string_lossy().to_string())
}

fn mask_secret(key: &str) -> Option<String> {
    let trimmed = key.trim();
    if trimmed.is_empty() || trimmed == "YOUR_TMDB_KEY_HERE" {
        return None;
    }
    if trimmed.len() <= 8 {
        return Some("••••••••".to_string());
    }
    Some(format!(
        "{}••••{}",
        &trimmed[..4],
        &trimmed[trimmed.len() - 4..]
    ))
}

// ============================================================================
// Search & Requests
// ============================================================================

async fn handle_search(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> Result<Json<Vec<TmdbSearchResult>>, (StatusCode, String)> {
    let tmdb_key = state.tmdb_api_key.read().await.clone();
    let results = search_tmdb(&state.http, &tmdb_key, &params.q)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;

    Ok(Json(results))
}

async fn handle_tv_seasons(
    State(state): State<AppState>,
    Path(tmdb_id): Path<i64>,
) -> Result<Json<Vec<SeasonSummary>>, (StatusCode, String)> {
    let tmdb_key = state.tmdb_api_key.read().await.clone();
    let episodes = fetch_tv_episodes(&state.http, &tmdb_key, tmdb_id)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;

    let mut counts: BTreeMap<i32, usize> = BTreeMap::new();
    for ep in episodes {
        *counts.entry(ep.season_number).or_insert(0) += 1;
    }

    let summaries = counts
        .into_iter()
        .map(|(season_number, episode_count)| SeasonSummary {
            season_number,
            episode_count,
        })
        .collect();

    Ok(Json(summaries))
}

async fn handle_create_request(
    State(state): State<AppState>,
    Json(payload): Json<CreateRequestPayload>,
) -> Result<(StatusCode, Json<RequestResponse>), (StatusCode, String)> {
    let folder_path = ensure_media_folder(
        &state.movies_dir,
        &state.shows_dir,
        &payload.media_type,
        &payload.title,
        payload.year,
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let existing: Option<(i64, String, Option<String>)> = sqlx::query_as(
        "SELECT id, status, root_folder FROM media_items WHERE tmdb_id = ?",
    )
    .bind(payload.tmdb_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Clone the TMDB key once up front so we can use it to look up `imdb_id`.
    let tmdb_key = state.tmdb_api_key.read().await.clone();

    let (media_id, current_status) = match existing {
        Some((id, status, existing_root)) => {
            if existing_root.as_deref().unwrap_or("").is_empty() {
                sqlx::query("UPDATE media_items SET root_folder = ? WHERE id = ?")
                    .bind(&folder_path)
                    .bind(id)
                    .execute(&state.db)
                    .await
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            }

            if payload.media_type == "movie" && status == "available" {
                return Ok((
                    StatusCode::OK,
                    Json(RequestResponse {
                        media_item_id: id,
                        status: "already_available".to_string(),
                    }),
                ));
            }
            (id, status)
        }
        None => {
            // Fetch the IMDb ID (with "tt" stripped into `Option<i64>`) for this new item.
            // If TMDB doesn't have one or the request fails, we log a warning and store `None`
            // rather than failing the user's request.
            let imdb_id: Option<i64> = fetch_imdb_id(
                &state.http,
                &tmdb_key,
                &payload.media_type,
                payload.tmdb_id,
            )
            .await
            .unwrap_or_else(|e| {
                eprintln!(
                    "[TMDB] Failed to fetch imdb_id for {} {}: {}",
                    payload.media_type, payload.tmdb_id, e
                );
                None
            });

            // Note: we added `imdb_id` to both the column list and `VALUES (...)`,
            // and bound `imdb_id` right after `payload.tmdb_id`.
            let id: i64 = sqlx::query_scalar(
                r#"
                INSERT INTO media_items (
                    tmdb_id, imdb_id, media_type, title, year, overview, poster_path,
                    status, root_folder, quality_profile_id
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, 'requested', ?, ?)
                RETURNING id
                "#,
            )
            .bind(payload.tmdb_id)
            .bind(imdb_id)
            .bind(&payload.media_type)
            .bind(&payload.title)
            .bind(payload.year)
            .bind(&payload.overview)
            .bind(&payload.poster_path)
            .bind(&folder_path)
            .bind(payload.quality_profile_id.unwrap_or(1))
            .fetch_one(&state.db)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

            (id, "requested".to_string())
        }
    };

    if payload.media_type == "tv" {
        // Reuse `tmdb_key` from above so we don't re-read the lock or shadow the name.
        if let Ok(episodes) = fetch_tv_episodes(&state.http, &tmdb_key, payload.tmdb_id).await {
            for ep in episodes {
                let is_monitored: i32 = match &payload.seasons {
                    Some(selected_seasons) => {
                        if selected_seasons.contains(&ep.season_number) {
                            1
                        } else {
                            0
                        }
                    }
                    None => 1,
                };

                sqlx::query(
                    r#"
                    INSERT INTO episodes (media_item_id, season_number, episode_number, title, air_date, status, monitored)
                    VALUES (?, ?, ?, ?, ?, 'missing', ?)
                    ON CONFLICT(media_item_id, season_number, episode_number) DO UPDATE SET
                        monitored = excluded.monitored
                    "#,
                )
                .bind(media_id)
                .bind(ep.season_number)
                .bind(ep.episode_number)
                .bind(&ep.title)
                .bind(&ep.air_date)
                .bind(is_monitored)
                .execute(&state.db)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            }
        }
    }

    let user = payload.requested_by.unwrap_or_else(|| "admin".to_string());
    sqlx::query("INSERT INTO requests (media_item_id, requested_by, status) VALUES (?, ?, 'approved')")
        .bind(media_id)
        .bind(user)
        .execute(&state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(RequestResponse {
            media_item_id: media_id,
            status: current_status,
        }),
    ))
}

// ============================================================================
// Library & Scanning
// ============================================================================

async fn handle_scan(
    State(state): State<AppState>,
    payload: Option<Json<ScanPayload>>,
) -> Result<Json<ScanSummary>, (StatusCode, String)> {
    let tmdb_key = state.tmdb_api_key.read().await.clone();

    if let Some(Json(p)) = payload.filter(|p| !p.path.trim().is_empty()) {
        let summary = scan_directory(&state.db, &state.http, &tmdb_key, &p.path)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        return Ok(Json(summary));
    }

    let mut total_files = 0;
    let mut total_imported = 0;
    let mut all_unmatched = Vec::new();

    for dir in [&state.movies_dir, &state.shows_dir] {
        if !std::path::Path::new(dir).exists() {
            continue;
        }

        let summary = scan_directory(&state.db, &state.http, &tmdb_key, dir)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        total_files += summary.files_found;
        total_imported += summary.imported_or_updated;
        all_unmatched.extend(summary.unmatched);
    }

    Ok(Json(ScanSummary {
        files_found: total_files,
        imported_or_updated: total_imported,
        unmatched: all_unmatched,
    }))
}

async fn handle_library(
    State(state): State<AppState>,
) -> Result<Json<Vec<LibraryItem>>, (StatusCode, String)> {
    let items = sqlx::query_as::<_, LibraryItem>(
        "SELECT id, tmdb_id, imdb_id, media_type, title, year, poster_path, status, root_folder, quality_profile_id FROM media_items ORDER BY title ASC",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(items))
}

async fn handle_series_episodes(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<EpisodeItem>>, (StatusCode, String)> {
    let episodes = sqlx::query_as::<_, EpisodeItem>(
        "SELECT id, season_number, episode_number, title, air_date, status, file_path FROM episodes WHERE media_item_id = ? ORDER BY season_number ASC, episode_number ASC",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(episodes))
}

async fn handle_wanted(
    State(state): State<AppState>,
) -> Result<Json<WantedQueue>, (StatusCode, String)> {
    let movies = sqlx::query_as::<_, LibraryItem>(
        "SELECT id, tmdb_id, media_type, title, year, poster_path, status, root_folder, quality_profile_id FROM media_items WHERE media_type = 'movie' AND status = 'requested'",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let episodes = sqlx::query_as::<_, WantedEpisode>(
        r#"
        SELECT e.id AS episode_id, m.id AS media_item_id, m.tmdb_id, m.title AS series_title,
               e.season_number, e.episode_number, e.title AS episode_title
        FROM episodes e
        JOIN media_items m ON m.id = e.media_item_id
        WHERE e.monitored = 1 AND e.status = 'missing'
        ORDER BY m.title ASC, e.season_number ASC, e.episode_number ASC
        "#,
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(WantedQueue { movies, episodes }))
}

fn resolve_media_folder_to_delete(
    raw_root_folder: &str,
    movies_dir: &StdPath,
    shows_dir: &StdPath,
) -> Option<PathBuf> {
    let trimmed = raw_root_folder.trim();
    if trimmed.is_empty() {
        return None;
    }

    let path = PathBuf::from(trimmed);

    // If `root_folder` points to a file (or has a file extension when missing), step up to its parent folder.
    let target_dir = if path.is_file() || (!path.exists() && path.extension().is_some()) {
        path.parent()?.to_path_buf()
    } else {
        path
    };

    // Safety guards: never delete empty/root paths or the top-level library folders themselves.
    let norm_target = std::fs::canonicalize(&target_dir).unwrap_or_else(|_| target_dir.clone());
    let norm_movies = std::fs::canonicalize(movies_dir).unwrap_or_else(|_| movies_dir.to_path_buf());
    let norm_shows = std::fs::canonicalize(shows_dir).unwrap_or_else(|_| shows_dir.to_path_buf());

    if norm_target.as_os_str().is_empty()
        || norm_target.parent().is_none()
        || norm_target == norm_movies
        || norm_target == norm_shows
    {
        eprintln!(
            "[DELETE] Safety guard blocked removing directory '{}'",
            norm_target.display()
        );
        return None;
    }

    Some(target_dir)
}

async fn remove_media_folder_if_present(
    raw_root_folder: Option<&str>,
    movies_dir: &StdPath,
    shows_dir: &StdPath,
) {
    let Some(root_str) = raw_root_folder else {
        return;
    };
    let Some(target_dir) = resolve_media_folder_to_delete(root_str, movies_dir, shows_dir) else {
        return;
    };

    match tokio::fs::try_exists(&target_dir).await {
        Ok(true) => {
            if let Err(err) = tokio::fs::remove_dir_all(&target_dir).await {
                eprintln!(
                    "[DELETE] Failed to remove local folder '{}': {}",
                    target_dir.display(),
                    err
                );
            } else {
                println!("[DELETE] Removed media folder: {}", target_dir.display());
            }
        }
        Ok(false) => {}
        Err(err) => {
            eprintln!(
                "[DELETE] Failed to check local folder '{}': {}",
                target_dir.display(),
                err
            );
        }
    }
}

async fn handle_delete_library_item(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, (StatusCode, String)> {
    let root_folder: Option<Option<String>> =
        sqlx::query_scalar("SELECT root_folder FROM media_items WHERE id = ?")
            .bind(id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let active_downloads: Vec<(String,)> =
        sqlx::query_as("SELECT download_id FROM downloads WHERE media_item_id = ?")
            .bind(id)
            .fetch_all(&state.db)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    for (download_id,) in active_downloads {
        let _ = state.downloader.remove_download(&download_id, true).await;
    }

    sqlx::query("DELETE FROM downloads WHERE media_item_id = ?")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    sqlx::query("DELETE FROM episodes WHERE media_item_id = ?")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    sqlx::query("DELETE FROM media_items WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let movies_dir = PathBuf::from(
        std::env::var("MOVIES_DIR").unwrap_or_else(|_| "./library/movies".to_string()),
    );
    let shows_dir = PathBuf::from(
        std::env::var("SHOWS_DIR").unwrap_or_else(|_| "./library/shows".to_string()),
    );

    if let Some(folder_opt) = root_folder {
        remove_media_folder_if_present(folder_opt.as_deref(), &movies_dir, &shows_dir).await;
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn handle_library_seasons(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<LibrarySeason>>, (StatusCode, String)> {
    let rows = sqlx::query_as::<_, LibrarySeason>(
        r#"
        SELECT
            season_number,
            COALESCE(MAX(CASE WHEN monitored != 0 THEN 1 ELSE 0 END), 0) != 0 AS monitored,
            COUNT(*) AS total_episodes,
            COALESCE(SUM(CASE WHEN status = 'available' THEN 1 ELSE 0 END), 0) AS available_episodes
        FROM episodes
        WHERE media_item_id = ? AND season_number > 0
        GROUP BY season_number
        ORDER BY season_number ASC
        "#,
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(rows))
}

async fn handle_toggle_season_monitor(
    State(state): State<AppState>,
    Path((id, season_number)): Path<(i64, i32)>,
    Json(payload): Json<ToggleSeasonMonitorPayload>,
) -> Result<StatusCode, (StatusCode, String)> {
    let monitored_val: i32 = if payload.monitored { 1 } else { 0 };

    sqlx::query(
        "UPDATE episodes SET monitored = ? WHERE media_item_id = ? AND season_number = ?",
    )
    .bind(monitored_val)
    .bind(id)
    .bind(season_number)
    .execute(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

// ============================================================================
// Downloads & Indexers
// ============================================================================

async fn handle_start_download(
    State(state): State<AppState>,
    Json(payload): Json<StartDownloadPayload>,
) -> Result<(StatusCode, Json<StartDownloadResponse>), (StatusCode, String)> {
    let info_hash = state
        .downloader
        .add_release(&payload.download_url)
        .await
        .map_err(|e| {
            eprintln!(
                "[DOWNLOAD] Failed to add release '{}' ({}): {}",
                payload.title, payload.download_url, e
            );
            (StatusCode::BAD_GATEWAY, e.to_string())
        })?;

    let matched_item: Option<(i64, String, String, Option<i32>, Option<String>)> =
        match payload.media_item_id {
            Some(id) => sqlx::query_as(
                "SELECT id, media_type, title, year, root_folder FROM media_items WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?,
            None => sqlx::query_as(
                "SELECT id, media_type, title, year, root_folder FROM media_items WHERE LOWER(?) LIKE '%' || LOWER(title) || '%' ORDER BY LENGTH(title) DESC LIMIT 1",
            )
            .bind(&payload.title)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?,
        };

    let resolved_media_id: Option<i64> = match matched_item {
        Some((id, media_type, title, year, existing_root)) => {
            let folder_path = match existing_root {
                Some(ref r) if !r.trim().is_empty() => {
                    tokio::fs::create_dir_all(r)
                        .await
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                    r.clone()
                }
                _ => {
                    let created = ensure_media_folder(
                        &state.movies_dir,
                        &state.shows_dir,
                        &media_type,
                        &title,
                        year,
                    )
                    .await
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

                    sqlx::query("UPDATE media_items SET root_folder = ? WHERE id = ?")
                        .bind(&created)
                        .bind(id)
                        .execute(&state.db)
                        .await
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

                    created
                }
            };
            let _ = folder_path;
            Some(id)
        }
        None => None,
    };

    sqlx::query(
        r#"
        INSERT INTO downloads (media_item_id, episode_id, protocol, download_id, title, status)
        VALUES (?, ?, 'torrent', ?, ?, 'downloading')
        "#,
    )
    .bind(resolved_media_id)
    .bind(payload.episode_id)
    .bind(&info_hash)
    .bind(&payload.title)
    .execute(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(StartDownloadResponse {
            download_id: info_hash,
            status: "downloading".to_string(),
        }),
    ))
}

async fn handle_list_downloads(
    State(state): State<AppState>,
) -> Result<Json<Vec<DownloadStatus>>, (StatusCode, String)> {
    let list = state
        .downloader
        .list_downloads()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(list))
}

#[derive(sqlx::FromRow)]
struct RemovedDownloadInfo {
    media_item_id: Option<i64>,
    episode_id: Option<i64>,
    title: String,
}

//delete an active download and change the media status back to missing
async fn handle_remove_download(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let info: Option<RemovedDownloadInfo> = sqlx::query_as(
        "SELECT media_item_id, episode_id, title FROM downloads WHERE download_id = ?",
    )
    .bind(&id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    state
        .downloader
        .remove_download(&id, true)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    sqlx::query("DELETE FROM downloads WHERE download_id = ?")
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Some(dl) = info {
        if let Some(ep_id) = dl.episode_id {
            sqlx::query(
                r#"
                UPDATE episodes
                SET status = 'missing'
                WHERE id = ? AND status = 'queued'
                "#,
            )
            .bind(ep_id)
            .execute(&state.db)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        } else if let Some(media_id) = dl.media_item_id {
            let media_type: Option<String> =
                sqlx::query_scalar("SELECT media_type FROM media_items WHERE id = ?")
                    .bind(media_id)
                    .fetch_optional(&state.db)
                    .await
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

            if media_type.as_deref() == Some("movie") {
                sqlx::query(
                    r#"
                    UPDATE media_items
                    SET status = 'requested'
                    WHERE id = ? AND status = 'queued'
                    "#,
                )
                .bind(media_id)
                .execute(&state.db)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            } else if let crate::parser::ParsedMedia::SeasonPack { season, .. } =
                crate::parser::parse_release_title(&dl.title)
            {
                sqlx::query(
                    r#"
                    UPDATE episodes
                    SET status = 'missing'
                    WHERE media_item_id = ? AND season_number = ? AND status = 'queued'
                    "#,
                )
                .bind(media_id)
                .bind(season)
                .execute(&state.db)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            } else {
                sqlx::query(
                    r#"
                    UPDATE episodes
                    SET status = 'missing'
                    WHERE media_item_id = ? AND status = 'queued'
                    "#,
                )
                .bind(media_id)
                .execute(&state.db)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            }
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn handle_list_indexers(
    State(state): State<AppState>,
) -> Result<Json<Vec<IndexerConfig>>, (StatusCode, String)> {
    let list = sqlx::query_as::<_, IndexerConfig>(
        "SELECT id, name, url, driver, api_key, enabled FROM indexers ORDER BY id ASC",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(list))
}

async fn handle_add_indexer(
    State(state): State<AppState>,
    Json(payload): Json<AddIndexerPayload>,
) -> Result<StatusCode, (StatusCode, String)> {
    sqlx::query("INSERT INTO indexers (name, url, driver, api_key, enabled) VALUES (?, ?, ?, ?, 1)")
        .bind(&payload.name)
        .bind(&payload.url)
        .bind(&payload.driver)
        .bind(&payload.api_key)
        .execute(&state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::CREATED)
}

async fn handle_update_indexer(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateIndexerPayload>,
) -> Result<StatusCode, (StatusCode, String)> {
    let raw_driver = if !payload.driver.trim().is_empty() {
        payload.driver.trim()
    } else {
        payload.provider.trim()
    };

    let driver = if raw_driver.is_empty() {
        "apibay".to_string()
    } else {
        raw_driver.to_lowercase()
    };

    let api_key = payload.api_key.trim();
    let enabled: i64 = if payload.enabled { 1 } else { 0 };

    let result = sqlx::query(
        r#"
        UPDATE indexers
        SET name = ?, url = ?, driver = ?, api_key = ?, enabled = ?
        WHERE id = ?
        "#,
    )
    .bind(payload.name.trim())
    .bind(payload.url.trim())
    .bind(driver)
    .bind(api_key)
    .bind(enabled)
    .bind(id)
    .execute(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, format!("Indexer {id} not found")));
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn handle_delete_indexer(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, (StatusCode, String)> {
    sqlx::query("DELETE FROM indexers WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

async fn handle_search_releases(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> Result<Json<Vec<ReleaseCandidate>>, (StatusCode, String)> {
    let releases = search_all_indexers_with_profile(
        &state.db,
        &state.http,
        &params.q,
        None,
        params.imdb_id,
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(releases))
}

// ============================================================================
// VPN & TMDB Settings
// ============================================================================

async fn handle_get_vpn(
    State(state): State<AppState>,
) -> Result<Json<VpnStatusResponse>, (StatusCode, String)> {
    let settings = load_vpn_settings(&state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(VpnStatusResponse {
        enabled: settings.enabled,
        provider_name: settings.provider_name,
        interface: settings.interface_name,
        connected: state.vpn.is_connected(),
        kill_switch_active: settings.enabled,
    }))
}

async fn handle_update_vpn(
    State(state): State<AppState>,
    Json(payload): Json<UpdateVpnPayload>,
) -> Result<Json<VpnStatusResponse>, (StatusCode, String)> {
    let interface_name = "wg-hook";

    sqlx::query(
        "UPDATE vpn_settings SET enabled = ?, provider_name = ?, interface_name = ?, wg_config = ? WHERE id = 1",
    )
    .bind(payload.enabled)
    .bind(&payload.provider_name)
    .bind(interface_name)
    .bind(&payload.wg_config)
    .execute(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if payload.enabled && !payload.wg_config.trim().is_empty() {
        state
            .vpn
            .start(interface_name, &payload.wg_config)
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    } else {
        state.vpn.stop().await;
    }

    Ok(Json(VpnStatusResponse {
        enabled: payload.enabled,
        provider_name: payload.provider_name,
        interface: interface_name.to_string(),
        connected: state.vpn.is_connected(),
        kill_switch_active: payload.enabled,
    }))
}

async fn handle_get_tmdb_settings(
    State(state): State<AppState>,
) -> Json<TmdbSettingsResponse> {
    let current = state.tmdb_api_key.read().await.clone();
    let masked = mask_secret(&current);
    Json(TmdbSettingsResponse {
        configured: masked.is_some(),
        masked_key: masked,
    })
}

async fn handle_update_tmdb_settings(
    State(state): State<AppState>,
    Json(payload): Json<UpdateTmdbPayload>,
) -> Result<Json<TmdbSettingsResponse>, (StatusCode, String)> {
    let clean_key = payload.api_key.trim().to_string();

    sqlx::query(
        r#"
        INSERT INTO app_settings (key, value, updated_at)
        VALUES ('tmdb_api_key', ?, CURRENT_TIMESTAMP)
        ON CONFLICT(key) DO UPDATE SET
            value = excluded.value,
            updated_at = CURRENT_TIMESTAMP
        "#,
    )
    .bind(&clean_key)
    .execute(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    {
        let mut guard = state.tmdb_api_key.write().await;
        *guard = if clean_key.is_empty() {
            std::env::var("TMDB_API_KEY")
                .unwrap_or_else(|_| "YOUR_TMDB_KEY_HERE".to_string())
        } else {
            clean_key
        };
    }

    let active = state.tmdb_api_key.read().await.clone();
    let masked = mask_secret(&active);
    Ok(Json(TmdbSettingsResponse {
        configured: masked.is_some(),
        masked_key: masked,
    }))
}

// ============================================================================
// Quality Profiles & Wanted Search
// ============================================================================

async fn handle_list_available_qualities() -> Json<Vec<&'static str>> {
    let list = Quality::all_default_order()
        .into_iter()
        .map(|q| q.as_str())
        .collect();
    Json(list)
}

async fn handle_list_profiles(
    State(state): State<AppState>,
) -> Result<Json<Vec<QualityProfile>>, (StatusCode, String)> {
    let rows = sqlx::query_as::<_, QualityProfileRow>(
        "SELECT id, name, upgrade_allowed, cutoff_quality FROM quality_profiles ORDER BY id ASC",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut profiles = Vec::with_capacity(rows.len());
    for row in rows {
        let items = sqlx::query_as::<_, QualityProfileItem>(
            r#"
            SELECT quality, allowed, rank, max_movie_size_mb, max_episode_size_mb
            FROM quality_profile_items
            WHERE profile_id = ?
            ORDER BY rank DESC
            "#,
        )
        .bind(row.id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        profiles.push(QualityProfile {
            id: row.id,
            name: row.name,
            upgrade_allowed: row.upgrade_allowed,
            cutoff_quality: row.cutoff_quality,
            items,
        });
    }

    Ok(Json(profiles))
}

async fn handle_create_profile(
    State(state): State<AppState>,
    Json(payload): Json<SaveQualityProfilePayload>,
) -> Result<(StatusCode, Json<i64>), (StatusCode, String)> {
    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let profile_id: i64 = sqlx::query_scalar(
        "INSERT INTO quality_profiles (name, upgrade_allowed, cutoff_quality) VALUES (?, ?, ?) RETURNING id",
    )
    .bind(&payload.name)
    .bind(payload.upgrade_allowed)
    .bind(&payload.cutoff_quality)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    for item in payload.items {
        sqlx::query(
            r#"
            INSERT INTO quality_profile_items (profile_id, quality, allowed, rank, max_movie_size_mb, max_episode_size_mb)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(profile_id)
        .bind(&item.quality)
        .bind(item.allowed)
        .bind(item.rank)
        .bind(item.max_movie_size_mb)
        .bind(item.max_episode_size_mb)
        .execute(&mut *tx)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    tx.commit()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((StatusCode::CREATED, Json(profile_id)))
}

async fn handle_update_profile(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(payload): Json<SaveQualityProfilePayload>,
) -> Result<StatusCode, (StatusCode, String)> {
    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    sqlx::query(
        "UPDATE quality_profiles SET name = ?, upgrade_allowed = ?, cutoff_quality = ? WHERE id = ?",
    )
    .bind(&payload.name)
    .bind(payload.upgrade_allowed)
    .bind(&payload.cutoff_quality)
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    sqlx::query("DELETE FROM quality_profile_items WHERE profile_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    for item in payload.items {
        sqlx::query(
            r#"
            INSERT INTO quality_profile_items (profile_id, quality, allowed, rank, max_movie_size_mb, max_episode_size_mb)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(id)
        .bind(&item.quality)
        .bind(item.allowed)
        .bind(item.rank)
        .bind(item.max_movie_size_mb)
        .bind(item.max_episode_size_mb)
        .execute(&mut *tx)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    tx.commit()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

async fn handle_delete_profile(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, (StatusCode, String)> {
    sqlx::query("UPDATE media_items SET quality_profile_id = NULL WHERE quality_profile_id = ?")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    sqlx::query("DELETE FROM quality_profiles WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

async fn handle_assign_media_profile(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(payload): Json<AssignProfilePayload>,
) -> Result<StatusCode, (StatusCode, String)> {
    sqlx::query("UPDATE media_items SET quality_profile_id = ? WHERE id = ?")
        .bind(payload.quality_profile_id)
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

async fn handle_trigger_wanted_search(
    State(state): State<AppState>,
) -> Result<StatusCode, (StatusCode, String)> {
    run_automated_wanted_search(&state.db, &state.http, &state.downloader)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::OK)
}