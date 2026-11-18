use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, patch, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use crate::{
    metadata::{fetch_tv_episodes, search_tmdb, TmdbSearchResult},
    parser::{scan_directory, ScanSummary},
    AppState,
};
use crate::indexers::{search_all_indexers, IndexerConfig, ReleaseCandidate};
use crate::vpn::{load_vpn_settings, VpnStatusResponse};
use crate::models::{Quality, QualityProfile, QualityProfileItem};

//VPN
#[derive(Deserialize)]
pub struct UpdateVpnPayload {
    pub enabled: bool,
    pub provider_name: String,
    pub wg_config: String,
}

//Indexers
#[derive(Deserialize)]
pub struct AddIndexerPayload {
    pub name: String,
    pub url: String,
    pub provider: String, // "yts" or "eztv"
}

//TMDB Search
#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
}

#[derive(Deserialize)]
pub struct CreateRequestPayload {
    pub tmdb_id: i64,
    pub media_type: String,
    pub title: String,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub requested_by: Option<String>,
    pub quality_profile_id: Option<i64>,
}

#[derive(Serialize)]
pub struct RequestResponse {
    pub media_item_id: i64,
    pub status: String,
}

//Library scan
#[derive(Deserialize)]
pub struct ScanPayload {
    pub path: String,
}

//Library
#[derive(Serialize, FromRow)]
pub struct LibraryItem {
    pub id: i64,
    pub tmdb_id: i64,
    pub media_type: String,
    pub title: String,
    pub year: Option<i32>,
    pub poster_path: Option<String>,
    pub status: String,
    pub root_folder: Option<String>,
    pub quality_profile_id: Option<i64>,
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


//Wanted queue
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

//Torrent
use crate::downloaders::DownloadStatus;

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

//Quality Profiles
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

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/search", get(handle_search))
        .route("/requests", post(handle_create_request))
        .route("/scan", post(handle_scan))
        .route("/library", get(handle_library))
        .route("/library/:id/episodes", get(handle_series_episodes))
        .route("/wanted", get(handle_wanted))
        .route("/indexers", get(handle_list_indexers).post(handle_add_indexer))
        .route("/indexers/:id", delete(handle_delete_indexer))
        .route("/releases", get(handle_search_releases))
        .route("/downloads", get(handle_list_downloads).post(handle_start_download))
        .route("/downloads/:id", delete(handle_remove_download))
        .route("/vpn", get(handle_get_vpn).post(handle_update_vpn))
        .route("/qualities", get(handle_list_available_qualities))
        .route("/quality-profiles", get(handle_list_profiles).post(handle_create_profile))
        .route("/quality-profiles/:id", put(handle_update_profile).delete(handle_delete_profile),)
        .route("/library/:id/quality-profile", patch(handle_assign_media_profile))
}

async fn handle_search(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> Result<Json<Vec<TmdbSearchResult>>, (StatusCode, String)> {
    let results = search_tmdb(&state.http, &state.tmdb_api_key, &params.q)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;

    Ok(Json(results))
}

async fn handle_create_request(
    State(state): State<AppState>,
    Json(payload): Json<CreateRequestPayload>,
) -> Result<(StatusCode, Json<RequestResponse>), (StatusCode, String)> {
    let existing: Option<(i64, String)> =
        sqlx::query_as("SELECT id, status FROM media_items WHERE tmdb_id = ?")
            .bind(payload.tmdb_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let (media_id, current_status) = match existing {
        Some((id, status)) => {
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
            let id: i64 = sqlx::query_scalar(
                r#"
                INSERT INTO media_items (tmdb_id, media_type, title, year, overview, poster_path, status, quality_profile_id)
                VALUES (?, ?, ?, ?, ?, ?, 'requested', ?)
                RETURNING id
                "#,
            )
            .bind(payload.tmdb_id)
            .bind(&payload.media_type)
            .bind(&payload.title)
            .bind(payload.year)
            .bind(&payload.overview)
            .bind(&payload.poster_path)
            .bind(payload.quality_profile_id.unwrap_or(1))
            .fetch_one(&state.db)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

            (id, "requested".to_string())
        }
    };

    if payload.media_type == "tv" {
        if let Ok(episodes) =
            fetch_tv_episodes(&state.http, &state.tmdb_api_key, payload.tmdb_id).await
        {
            for ep in episodes {
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

async fn handle_scan(
    State(state): State<AppState>,
    Json(payload): Json<ScanPayload>,
) -> Result<Json<ScanSummary>, (StatusCode, String)> {
    let summary = scan_directory(&state.db, &state.http, &state.tmdb_api_key, &payload.path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(summary))
}

async fn handle_library(
    State(state): State<AppState>,
) -> Result<Json<Vec<LibraryItem>>, (StatusCode, String)> {
    let items = sqlx::query_as::<_, LibraryItem>(
        "SELECT id, tmdb_id, media_type, title, year, poster_path, status, root_folder, quality_profile_id FROM media_items ORDER BY title ASC",
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

//Download
async fn handle_start_download(
    State(state): State<AppState>,
    Json(payload): Json<StartDownloadPayload>,
) -> Result<(StatusCode, Json<StartDownloadResponse>), (StatusCode, String)> {
    let info_hash = state
        .downloader
        .add_release(&payload.download_url)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;

    if let Some(media_id) = payload.media_item_id {
        sqlx::query(
            r#"
            INSERT INTO downloads (media_item_id, episode_id, protocol, download_id, title, status)
            VALUES (?, ?, 'torrent', ?, ?, 'downloading')
            "#,
        )
        .bind(media_id)
        .bind(payload.episode_id)
        .bind(&info_hash)
        .bind(&payload.title)
        .execute(&state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

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

async fn handle_remove_download(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
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

    Ok(StatusCode::NO_CONTENT)
}

//Indexers
async fn handle_list_indexers(
    State(state): State<AppState>,
) -> Result<Json<Vec<IndexerConfig>>, (StatusCode, String)> {
    let list = sqlx::query_as::<_, IndexerConfig>(
        "SELECT id, name, url, api_key, enabled FROM indexers ORDER BY id ASC",
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
    sqlx::query("INSERT INTO indexers (name, url, api_key, enabled) VALUES (?, ?, ?, 1)")
        .bind(&payload.name)
        .bind(&payload.url)
        .bind(&payload.provider)
        .execute(&state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::CREATED)
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
    let releases = search_all_indexers(&state.db, &state.http, &params.q)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(releases))
}

//VPN
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

//Quality Profile Functions
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
    // Clear references on media_items before deleting
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