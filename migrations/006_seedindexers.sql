INSERT INTO indexers (name, url, driver, api_key, enabled)
SELECT 'ApiBay', 'https://apibay.org', 'apibay', '', 1
WHERE NOT EXISTS (SELECT 1 FROM indexers WHERE LOWER(name) = 'apibay' OR url = 'https://apibay.org');

INSERT INTO indexers (name, url, driver, api_key, enabled)
SELECT 'YTS', 'https://yts.lt', 'yts', '', 1
WHERE NOT EXISTS (SELECT 1 FROM indexers WHERE LOWER(name) = 'yts' OR url = 'https://yts.lt');

INSERT INTO indexers (name, url, driver, api_key, enabled)
SELECT 'EZTV', 'https://eztvx.to', 'eztv', '', 1
WHERE NOT EXISTS (SELECT 1 FROM indexers WHERE LOWER(name) = 'eztv' OR url = 'https://eztvx.to');