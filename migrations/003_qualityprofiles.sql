-- Quality Profiles
CREATE TABLE IF NOT EXISTS quality_profiles (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    upgrade_allowed BOOLEAN NOT NULL DEFAULT 1,
    cutoff_quality TEXT NOT NULL,
    is_default INTEGER NOT NULL DEFAULT 0
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

-- Seed default profiles: Any, SD, HD, SD/HD, 4k
INSERT OR IGNORE INTO quality_profiles (name, upgrade_allowed, cutoff_quality)
VALUES
    ('Any',   1, 'Remux-2160p'),
    ('SD',    1, 'Bluray-720p'),
    ('HD',    1, 'Bluray-1080p'),
    ('SD/HD', 1, 'Bluray-1080p'),
    ('4k',    1, 'Remux-2160p');

-- Seed all 13 quality tiers for each profile
INSERT INTO quality_profile_items (
    profile_id,
    quality,
    allowed,
    rank,
    max_movie_size_mb,
    max_episode_size_mb
)
SELECT
    qp.id,
    q.quality,
    CASE qp.name
        WHEN 'Any' THEN 1

        WHEN 'SD' THEN CASE
            WHEN q.quality IN ('Bluray-720p', 'WEBDL-720p', 'HDTV-720p') THEN 1
            ELSE 0
        END

        WHEN 'HD' THEN CASE
            WHEN q.quality IN ('Remux-1080p', 'Bluray-1080p', 'WEBDL-1080p', 'HDTV-1080p') THEN 1
            ELSE 0
        END

        WHEN 'SD/HD' THEN CASE
            WHEN q.quality IN (
                'Remux-1080p', 'Bluray-1080p', 'WEBDL-1080p', 'HDTV-1080p',
                'Bluray-720p', 'WEBDL-720p', 'HDTV-720p'
            ) THEN 1
            ELSE 0
        END

        WHEN '4k' THEN CASE
            WHEN q.quality IN ('Remux-2160p', 'Bluray-2160p', 'WEBDL-2160p') THEN 1
            ELSE 0
        END

        ELSE 0
    END AS allowed,
    q.rank,
    q.max_movie_size_mb,
    q.max_episode_size_mb
FROM quality_profiles qp
CROSS JOIN (
    SELECT 'Remux-2160p' AS quality, 12 AS rank, NULL  AS max_movie_size_mb, NULL AS max_episode_size_mb
    UNION ALL SELECT 'Bluray-2160p',  11, 25000,  8000
    UNION ALL SELECT 'WEBDL-2160p',   10, 25000,  6000
    UNION ALL SELECT 'Remux-1080p',    9, 25000,  8000
    UNION ALL SELECT 'Bluray-1080p',   8, 6000, 3000
    UNION ALL SELECT 'WEBDL-1080p',    7, 6000, 2000
    UNION ALL SELECT 'HDTV-1080p',     6, 6000, 1800
    UNION ALL SELECT 'Bluray-720p',    5, 2000, 1200
    UNION ALL SELECT 'WEBDL-720p',     4, 2000,  900
    UNION ALL SELECT 'HDTV-720p',      3, 2000,  800
    UNION ALL SELECT 'WEBDL-480p',     2, 500,  400
    UNION ALL SELECT 'SDTV',           1, 500,  300
    UNION ALL SELECT 'Unknown',        0, NULL,  NULL
) q
WHERE qp.name IN ('Any', 'SD', 'HD', 'SD/HD', '4k');

-- Default any unprofiled items to HD
UPDATE media_items
SET quality_profile_id = (SELECT id FROM quality_profiles WHERE name = 'HD')
WHERE quality_profile_id IS NULL;