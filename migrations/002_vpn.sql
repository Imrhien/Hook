CREATE TABLE IF NOT EXISTS vpn_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    enabled BOOLEAN NOT NULL DEFAULT 0,
    provider_name TEXT NOT NULL DEFAULT 'Custom WireGuard',
    interface_name TEXT NOT NULL DEFAULT 'wg-hook',
    wg_config TEXT NOT NULL DEFAULT ''
);

INSERT OR IGNORE INTO vpn_settings (id, enabled, provider_name, interface_name, wg_config)
VALUES (1, 0, 'Custom WireGuard', 'wg-hook', '');
