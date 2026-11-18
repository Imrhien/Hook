mod api;
mod downloaders;
mod indexers;
mod metadata;
mod models;
mod parser;
mod vpn;

use axum::{
    extract::State,
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use downloaders::{torrent::RqbitDownloader, DownloadClient};
use reqwest::Client;
use rust_embed::RustEmbed;
use serde::Serialize;
use sqlx::{sqlite::{SqliteConnectOptions, SqlitePoolOptions},SqlitePool,};
use std::{env, net::SocketAddr, path::PathBuf, str::FromStr, sync::Arc};
use tower_http::cors::CorsLayer;
use vpn::{load_vpn_settings, UserspaceVpnManager};

// Embeds the compiled React frontend (`web/dist`) directly inside the Hook binary
#[derive(RustEmbed)]
#[folder = "web/dist"]
struct FrontendAssets;

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub http: Client,
    pub tmdb_api_key: String,
    pub downloader: Arc<dyn DownloadClient>,
    pub vpn: UserspaceVpnManager,
}

#[derive(Serialize)]
struct HealthResponse {
    status: String,
    app: String,
    media_count: i64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let tmdb_api_key = env::var("TMDB_API_KEY")
        .unwrap_or_else(|_| "YOUR_TMDB_KEY_HERE".to_string());

    // --- 1. DATABASE SETUP (Updated for Docker env vars & foreign keys) ---
    let db_url = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite:hook.db?mode=rwc".to_string());

    let connect_opts = SqliteConnectOptions::from_str(&db_url)?
        .create_if_missing(true)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(connect_opts)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;
    // ----------------------------------------------------------------------

    let http_client = Client::builder()
        .user_agent("Hook/0.1.0")
        .build()?;

    // 2. Initialize pure-Rust userspace WireGuard manager
    let vpn_manager = UserspaceVpnManager::new();
    let vpn_settings = load_vpn_settings(&pool).await?;

    if vpn_settings.enabled && !vpn_settings.wg_config.trim().is_empty() {
        if let Err(e) = vpn_manager
            .start(&vpn_settings.interface_name, &vpn_settings.wg_config)
            .await
        {
            eprintln!("[VPN] Startup warning: {}", e);
        }
    }

    // --- 3. DOWNLOADER SETUP (Updated for Docker DOWNLOAD_DIR env var) ---
    let iface = if vpn_settings.enabled {
        Some(vpn_settings.interface_name.clone())
    } else {
        None
    };

    let download_dir = env::var("DOWNLOAD_DIR")
        .unwrap_or_else(|_| "./downloads".to_string());

    let downloader = Arc::new(
        RqbitDownloader::new(
            PathBuf::from(download_dir),
            vpn_settings.enabled,
            iface,
            vpn_manager.clone(),
        )
        .await?,
    );
    // ----------------------------------------------------------------------

    let state = AppState {
        db: pool,
        http: http_client,
        tmdb_api_key,
        downloader,
        vpn: vpn_manager,
    };

    // 4. Mount API routes + embedded React frontend fallback
    let app = Router::new()
        .route("/api/health", get(health_check))
        .nest("/api", api::router())
        .fallback(serve_frontend)
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 7878));
    println!("Hook is live at http://localhost:7878");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn health_check(State(state): State<AppState>) -> Json<HealthResponse> {
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM media_items")
        .fetch_one(&state.db)
        .await
        .unwrap_or((0,));

    Json(HealthResponse {
        status: "ok".to_string(),
        app: "Hook".to_string(),
        media_count: count.0,
    })
}

async fn serve_frontend(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let file_path = if path.is_empty() { "index.html" } else { path };

    // Try exact asset match first, otherwise fall back to index.html for SPA routes
    let asset = FrontendAssets::get(file_path).or_else(|| FrontendAssets::get("index.html"));

    match asset {
        Some(content) => {
            let mime = match file_path.rsplit('.').next() {
                Some("html") => "text/html; charset=utf-8",
                Some("js") => "application/javascript; charset=utf-8",
                Some("css") => "text/css; charset=utf-8",
                Some("svg") => "image/svg+xml",
                Some("png") => "image/png",
                Some("ico") => "image/x-icon",
                _ => "text/html; charset=utf-8",
            };

            ([(header::CONTENT_TYPE, mime)], content.data).into_response()
        }
        None => (StatusCode::NOT_FOUND, "Frontend not built yet").into_response(),
    }
}