-- Quality Profiles
CREATE TABLE IF NOT EXISTS quality_profiles (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    upgrade_allowed BOOLEAN NOT NULL DEFAULT 1,
    cutoff_quality TEXT NOT NULL
);

-- Individual Quality Tiers & Size Limits per Profile
CREATE TABLE IF NOT EXISTS quality_profile_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    profile_id INTEGER NOT NULL,
    quality TEXT NOT NULL,
    allowed BOOLEAN NOT NULL DEFAULT 1,
    rank INTEGER NOT NULL,
    max_movie_size_mb INTEGER,
    max_episode_size_mb INTEGER,
    FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE
);

-- Link existing & future library items to a Quality Profile
ALTER TABLE media_items ADD COLUMN quality_profile_id INTEGER REFERENCES quality_profiles(id) ON DELETE SET NULL;

-- Seed a default "HD-1080p" profile so existing & newly requested items have a baseline
INSERT OR IGNORE INTO quality_profiles (id, name, upgrade_allowed, cutoff_quality)
VALUES (1, 'HD-1080p', 1, 'Bluray-1080p');

INSERT INTO quality_profile_items (profile_id, quality, allowed, rank, max_movie_size_mb, max_episode_size_mb)
VALUES
    (1, 'Remux-2160p', 0, 12, NULL, NULL),
    (1, 'Bluray-2160p', 0, 11, NULL, NULL),
    (1, 'WEBDL-2160p', 0, 10, NULL, NULL),
    (1, 'Remux-1080p', 0, 9, NULL, NULL),
    (1, 'Bluray-1080p', 1, 8, 25000, 5000),
    (1, 'WEBDL-1080p', 1, 7, 15000, 3000),
    (1, 'HDTV-1080p', 1, 6, 12000, 2500),
    (1, 'Bluray-720p', 1, 5, 10000, 2000),
    (1, 'WEBDL-720p', 1, 4, 8000, 1500),
    (1, 'HDTV-720p', 1, 3, 6000, 1200),
    (1, 'WEBDL-480p', 0, 2, 3000, 800),
    (1, 'SDTV', 0, 1, 2000, 500),
    (1, 'Unknown', 0, 0, NULL, NULL);

-- Assign existing items in the database to the default profile
UPDATE media_items SET quality_profile_id = 1 WHERE quality_profile_id IS NULL;
