const TMDB_BASE_URL: &str = "https://api.themoviedb.org/3";

use reqwest::Client;
use serde::{Deserialize, Serialize};
use futures::future::try_join_all;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TmdbSearchResult {
    pub id: i64,
    pub media_type: String,
    pub title: String,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
}

#[derive(Deserialize)]
struct TmdbMultiResponse {
    #[serde(default)]
    results: Vec<TmdbRawItem>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct TmdbRawItem {
    id: i64,
    media_type: Option<String>,
    title: Option<String>,
    name: Option<String>,
    release_date: Option<String>,
    first_air_date: Option<String>,
    overview: Option<String>,
    poster_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TmdbEpisodeInfo {
    pub season_number: i32,
    pub episode_number: i32,
    pub title: Option<String>,
    pub air_date: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct TmdbTvDetails {
    name: Option<String>,
    seasons: Vec<TmdbSeasonSummary>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct TmdbSeasonSummary {
    season_number: Option<i32>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct TmdbSeasonDetails {
    episodes: Vec<TmdbRawEpisode>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct TmdbRawEpisode {
    season_number: Option<i32>,
    episode_number: Option<i32>,
    name: Option<String>,
    air_date: Option<String>,
}

pub async fn search_tmdb(
    client: &Client,
    api_key: &str,
    query: &str,
) -> anyhow::Result<Vec<TmdbSearchResult>> {
    let url = format!("{}/search/multi", TMDB_BASE_URL);
    let key = api_key.trim();

    let mut req = client
        .get(url)
        .header("Accept", "application/json")
        .query(&[("query", query), ("include_adult", "false")]);

    if key.len() > 40 {
        req = req.bearer_auth(key);
    } else {
        req = req.query(&[("api_key", key)]);
    }

    let response = req.send().await?;
    let status = response.status();
    let raw_text = response.text().await?;

    if !status.is_success() {
        anyhow::bail!("TMDB HTTP {}: {}", status, raw_text);
    }

    let parsed: TmdbMultiResponse = serde_json::from_str(&raw_text)
        .map_err(|e| anyhow::anyhow!("JSON parse error: {} | Body: {}", e, raw_text))?;

    let results = parsed
        .results
        .into_iter()
        .filter_map(|item| {
            let media_type = item.media_type?;
            if media_type != "movie" && media_type != "tv" {
                return None;
            }

            let title = item.title.or(item.name)?;
            let date_str = item.release_date.or(item.first_air_date).unwrap_or_default();
            let year = date_str.split('-').next().and_then(|y| y.parse::<i32>().ok());

            Some(TmdbSearchResult {
                id: item.id,
                media_type,
                title,
                year,
                overview: item.overview,
                poster_path: item.poster_path,
            })
        })
        .collect();

    Ok(results)
}

pub async fn fetch_tv_episodes(
    client: &Client,
    api_key: &str,
    tmdb_id: i64,
) -> anyhow::Result<Vec<TmdbEpisodeInfo>> {
    let key = api_key.trim();
    let tv_url = format!("{}/tv/{}", TMDB_BASE_URL, tmdb_id);

    let mut req = client.get(&tv_url).header("Accept", "application/json");
    req = if key.len() > 40 {
        req.bearer_auth(key)
    } else {
        req.query(&[("api_key", key)])
    };

    let tv_resp = req.send().await?;
    let tv_status = tv_resp.status();
    let raw_tv = tv_resp.text().await?;
    if !tv_status.is_success() {
        anyhow::bail!("TMDB TV HTTP {} at {}: {}", tv_status, tv_url, raw_tv);
    }

    let tv_details: TmdbTvDetails = serde_json::from_str(&raw_tv)
        .map_err(|e| anyhow::anyhow!("TV details parse error: {} | Body: {}", e, raw_tv))?;

    let show_title = tv_details.name.unwrap_or_else(|| "Unknown".to_string());

    // Build a list of async tasks—one per valid season
    let season_futures = tv_details
        .seasons
        .into_iter()
        .filter_map(|s| s.season_number)
        .filter(|&season_num| season_num > 0)
        .map(|season_num| async move {
            let season_url = format!("{}/tv/{}/season/{}", TMDB_BASE_URL, tmdb_id, season_num);
            let mut s_req = client.get(&season_url).header("Accept", "application/json");
            s_req = if key.len() > 40 {
                s_req.bearer_auth(key)
            } else {
                s_req.query(&[("api_key", key)])
            };

            let s_resp = s_req.send().await?;
            let s_status = s_resp.status();
            let raw_season = s_resp.text().await?;
            if !s_status.is_success() {
                anyhow::bail!("TMDB Season {} HTTP {}: {}", season_num, s_status, raw_season);
            }

            let season_data: TmdbSeasonDetails = serde_json::from_str(&raw_season)
                .map_err(|e| anyhow::anyhow!("Season {} parse error: {}", season_num, e))?;

            let episodes: Vec<TmdbEpisodeInfo> = season_data
                .episodes
                .into_iter()
                .filter_map(|ep| {
                    Some(TmdbEpisodeInfo {
                        season_number: ep.season_number?,
                        episode_number: ep.episode_number?,
                        title: ep.name,
                        air_date: ep.air_date,
                    })
                })
                .collect();

            Ok::<Vec<TmdbEpisodeInfo>, anyhow::Error>(episodes)
        });

    // Run all season requests concurrently and preserve season order
    let seasons_episodes = try_join_all(season_futures).await?;
    let all_episodes: Vec<TmdbEpisodeInfo> = seasons_episodes.into_iter().flatten().collect();

    println!(
        "[TMDB] Fetched {} total episodes for {} (tmdb_id: {})",
        all_episodes.len(),
        show_title,
        tmdb_id
    );

    Ok(all_episodes)