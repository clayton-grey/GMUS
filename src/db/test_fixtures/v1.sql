CREATE TABLE media_items (
    id              INTEGER PRIMARY KEY,
    fingerprint     TEXT NOT NULL UNIQUE,
    title           TEXT,
    artist          TEXT,
    album           TEXT,
    album_artist    TEXT,
    album_year      INTEGER,
    release_date    TEXT,
    composer        TEXT,
    genre           TEXT,
    cover_path      TEXT,
    track_number    INTEGER,
    track_total     INTEGER,
    disc_number     INTEGER,
    disc_total      INTEGER,
    duration_ms     INTEGER,
    compilation     INTEGER NOT NULL DEFAULT 0,
    first_seen_at   INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE TABLE locations (
    id              INTEGER PRIMARY KEY,
    media_item_id   INTEGER NOT NULL REFERENCES media_items(id) ON DELETE CASCADE,
    path            TEXT NOT NULL UNIQUE,
    file_size       INTEGER,
    modified_at     INTEGER,
    seen_at         INTEGER NOT NULL,
    missing         INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE play_events (
    id              INTEGER PRIMARY KEY,
    media_item_id   INTEGER NOT NULL REFERENCES media_items(id) ON DELETE CASCADE,
    location_id     INTEGER REFERENCES locations(id) ON DELETE SET NULL,
    played_at       INTEGER NOT NULL,
    duration_ms     INTEGER NOT NULL DEFAULT 0,
    completed       INTEGER NOT NULL DEFAULT 0,
    source          TEXT NOT NULL DEFAULT 'local'
);

CREATE TABLE media_stats (
    media_item_id   INTEGER PRIMARY KEY REFERENCES media_items(id) ON DELETE CASCADE,
    play_count      INTEGER NOT NULL DEFAULT 0,
    last_played_at  INTEGER,
    total_play_ms   INTEGER NOT NULL DEFAULT 0,
    skip_count      INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE library_roots (
    id              INTEGER PRIMARY KEY,
    path            TEXT NOT NULL UNIQUE,
    active          INTEGER NOT NULL DEFAULT 1,
    added_at        INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    last_scanned_at INTEGER
);

CREATE TABLE playlists (
    id              INTEGER PRIMARY KEY,
    name            TEXT NOT NULL UNIQUE COLLATE NOCASE,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE TABLE playlist_tracks (
    id              INTEGER PRIMARY KEY,
    playlist_id     INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    media_item_id   INTEGER NOT NULL REFERENCES media_items(id) ON DELETE CASCADE,
    position        INTEGER NOT NULL,
    added_at        INTEGER NOT NULL
);

CREATE TABLE app_browser_selection (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    tree_kind       TEXT NOT NULL,
    artist          TEXT,
    album           TEXT,
    playlist_id     INTEGER,
    media_item_id   INTEGER,
    updated_at      INTEGER NOT NULL
);

CREATE TABLE app_key_bindings (
    action          TEXT NOT NULL,
    key             TEXT NOT NULL,
    updated_at      INTEGER NOT NULL,
    PRIMARY KEY (action, key)
);

CREATE TABLE app_settings (
    key             TEXT PRIMARY KEY,
    value           TEXT NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE TABLE app_filter_state (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    filter          TEXT NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE INDEX idx_locations_media_item
    ON locations(media_item_id);
CREATE INDEX idx_play_events_media_item
    ON play_events(media_item_id, played_at);
CREATE INDEX idx_media_items_artist_album
    ON media_items(album_artist, artist, album);
CREATE INDEX idx_playlist_tracks_playlist_position
    ON playlist_tracks(playlist_id, position);

PRAGMA user_version = 1;
