-- Movies & TV Series (Radarr + Sonarr unified)
CREATE TABLE IF NOT EXISTS media_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    tmdb_id INTEGER NOT NULL UNIQUE,
    tvdb_id INTEGER,
    media_type TEXT NOT NULL CHECK(media_type IN ('movie', 'tv')),
    title TEXT NOT NULL,
    year INTEGER,
    overview TEXT,
    poster_path TEXT,
    status TEXT NOT NULL DEFAULT 'requested', -- requested, monitored, downloading, available
    root_folder TEXT,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Individual Episodes for TV Series (Sonarr)
CREATE TABLE IF NOT EXISTS episodes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    media_item_id INTEGER NOT NULL,
    season_number INTEGER NOT NULL,
    episode_number INTEGER NOT NULL,
    title TEXT,
    air_date TEXT,
    monitored BOOLEAN NOT NULL DEFAULT 1,
    status TEXT NOT NULL DEFAULT 'missing', -- missing, downloading, available
    file_path TEXT,
    FOREIGN KEY (media_item_id) REFERENCES media_items(id) ON DELETE CASCADE,
    UNIQUE(media_item_id, season_number, episode_number)
);

-- User Requests (Overseerr / Jellyseerr)
CREATE TABLE IF NOT EXISTS requests (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    media_item_id INTEGER NOT NULL,
    requested_by TEXT NOT NULL DEFAULT 'admin',
    status TEXT NOT NULL DEFAULT 'approved', -- pending, approved, declined, available
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (media_item_id) REFERENCES media_items(id) ON DELETE CASCADE
);

-- Indexers (Prowlarr)
CREATE TABLE IF NOT EXISTS indexers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    url TEXT NOT NULL,
    api_key TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT 1
);

-- Active & Historical Downloads (Built-in Torrent Engine + future Usenet)
CREATE TABLE IF NOT EXISTS downloads (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    media_item_id INTEGER NOT NULL,
    episode_id INTEGER,
    protocol TEXT NOT NULL DEFAULT 'torrent', -- torrent, usenet
    download_id TEXT NOT NULL, -- Torrent info_hash or NZB ID
    title TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued', -- queued, downloading, completed, failed
    progress REAL NOT NULL DEFAULT 0.0,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (media_item_id) REFERENCES media_items(id) ON DELETE CASCADE,
    FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE SET NULL
);
