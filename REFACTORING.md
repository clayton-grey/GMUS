# Refactoring Backlog

This file captures cleanup identified during the feature-complete review. The
first pass prioritized correctness, lifecycle ownership, and data integrity.
The items below are intentionally deferred because they require broader changes
than the immediate reliability payoff justified.

## Completed In The First Pass

- Made playback shutdown single-owner and failure-safe.
- Added best-effort terminal restoration through an RAII session guard.
- Prevented ambiguous track fingerprints and destructive duplicate merges.
- Made scans reconcile repeated root adds, isolate artwork failures, and avoid
  repeated global duplicate merging.
- Fixed filtered album summaries and playlist focus/selection edge cases.
- Centralized TUI input-mode transitions and shared display-width formatting.

## Completed In The Second Pass

- Replaced the three parallel playlist maps with a typed playlist cache that
  keeps entry IDs, media IDs, and optional current-library indices together.
- Preserved unavailable playlist entries in counts while excluding them from
  rendering and playback.
- Grouped the media integration backend and synchronization bookkeeping into
  one `IntegrationState`.
- Kept `db` as the public facade while extracting persisted UI settings,
  browser state, pane layout, and key bindings into `db::settings`.
- Added migration coverage for legacy key-binding tables.

## Completed In The Third Pass

- Made `media_items.id` the durable identity and changed metadata fingerprints
  into non-unique duplicate-candidate keys.
- Made track upserts path-first, preserving identity and history across
  same-path metadata changes while keeping matching present files distinct.
- Added deterministic repair for legacy media items with multiple present
  locations; location-linked events follow their location while ambiguous
  playlists, aggregate-only playback history, and browser selection remain on
  the original identity.
- Enforced one present location per media item in SQLite and separated legacy
  location splitting from historical-path reuse so a newly appearing file
  cannot inherit the renamed track's events.
- Moved artwork cache keys from metadata fingerprints to durable media IDs.
- Added ordered, immediate, transactional schema migrations with a frozen
  version-one compatibility bootstrap, foreign-key validation, and schema
  version rejection for older binaries.
- Added immutable version-one fixtures covering unversioned and sparse schemas,
  legacy playlist/keymap shapes, concurrent file-backed upgrades,
  deterministic identity splitting, idempotence, integrity, invalid versions,
  and rollback after a late failure.
- Tightened rename reconciliation to exactly one missing candidate with matching
  file size and modification time, kept artwork ownership identity-local, and
  refreshed saved browser hierarchy when merging.

## Completed In The Fourth Pass

- Kept `db` as the public facade while extracting library-root persistence,
  playlist persistence and ordering, and playback-history recording/statistics
  into focused modules.
- Kept playlist legacy-shape repair private to `db::playlists` while exposing it
  narrowly to ordered migrations.
- Grouped command output lines, structured library roots, selection, kind, and
  focus into one `CommandOutputState` with centralized show, clear, and movement
  behavior.

## Next Structural Pass

- Continue grouping the large TUI `App` state into browser, playlist, input,
  playback, and layout state objects while preserving the coordinator API.
- Extract the cohesive catalog and identity implementation from the `db`
  facade after its cross-table tests are ready to move with it.
- Split the large TUI test module by concern after the state boundaries settle.

## Later Reliability And Performance Work

- Replace conservative rename matching based on metadata, file size, and
  modification time with stable filesystem identity evidence.
- Define an explicit best-effort policy for nested filesystem read failures
  during scans, not only metadata and artwork failures.
- Give background library jobs explicit owned shutdown semantics instead of
  detaching worker threads.
- Define explicit complete, partial, and unavailable scan outcomes, including
  deleted single-file roots and unavailable volumes.
- Skip metadata and artwork parsing for unchanged files during rescans.
- Revisit playback listened-time accounting after long event-loop stalls.
- Preserve non-UTF-8 filesystem paths without lossy string conversion.
- Either add Windows path handling and CI or explicitly document Unix-like
  platform support.
