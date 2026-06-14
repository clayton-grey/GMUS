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

## Completed In The Fifth Pass

- Extracted catalog queries, path-first upserts, location reconciliation,
  identity splitting and merging, cover ownership, and media-stat repair into
  one cohesive `db::catalog` implementation module.
- Reduced production `db.rs` to the stable public facade, connection setup,
  shared time/formatting utilities, and cross-domain integration tests.
- Kept migration access limited to the single legacy-location split hook and
  kept catalog reconciliation details private.

## Completed In The Sixth Pass

- Began splitting the oversized TUI test suite by moving the filter query,
  rendering, persistence, confirmation, and clearing tests into a focused
  `tui::tests::filter` submodule.
- Preserved shared TUI test fixtures and private coordinator access in the
  parent test module without widening production visibility.

## Completed In The Seventh Pass

- Grouped playback target, continuous, repeat, shuffle, and cached shuffle
  permutation state into one private `PlaybackModeState`.
- Moved ordered and shuffled sequence navigation into the playback mode state
  while keeping playable-sequence assembly and user-facing toggle coordination
  on `App`.
- Kept mode rendering and tests on narrow state queries instead of exposing
  playback policy fields.

## Completed In The Eighth Pass

- Moved command execution, rate input, command output, scan-job command, and
  completion tests into a focused `tui::tests::command` submodule.
- Moved the deterministic shuffle permutation mechanics test beside the private
  playback mode implementation, avoiding test-only state setters and cache
  accessors.
- Continued sharing parent TUI fixtures without widening production
  visibility.

## Completed In The Ninth Pass

- Grouped info visibility, startup overlay visibility, pane offsets, and the
  responsive column breakpoint into one private `LayoutState`.
- Moved visibility transitions and clamped pane resizing into the layout state
  while keeping input coordination, status messages, and persistence on
  `App`.
- Kept rendering, mouse hit-testing, commands, and tests on narrow layout
  queries, with edge-invariant tests beside the implementation.

## Completed In The Tenth Pass

- Replaced the three independent command, filter, and rate mode flags with one
  mutually exclusive `InputMode` owned by a private `InputState`.
- Grouped command, filter, and rate buffers with their mode transitions while
  preserving active filters across mode changes and clearing transient command
  and rate input on entry.
- Kept command execution, filter persistence, rate validation, rendering, and
  input coordination in their existing modules behind narrow input-state
  operations.

## Completed In The Eleventh Pass

- Grouped the library tree cursor, track-row cursor, and tree expansion policy
  into one private `BrowserState`.
- Moved cursor clamping, tree movement, and expansion snapshots behind narrow
  browser-state operations.
- Kept derived browser rows, playlist-panel selection, keymap selection, and
  cross-pane focus outside the browser state because they have separate
  ownership and transition rules; Ratatui list state remains presentation
  state beside the other renderer-facing list states.

## Completed In The Twelfth Pass

- Grouped playlist-panel cursor, expansion, and active-playlist status into one
  private `PlaylistPanelState`.
- Moved row movement and clamping, active-playlist updates, expansion
  transitions, and playlist-content behavior behind narrow state operations.
- Kept playlist cache and derived entries, Ratatui list state, cross-pane
  focus, and shared management-panel visibility outside the playlist state
  because they require separate coordination.

## Completed In The Thirteenth Pass

- Grouped keymap-panel cursor and capture state into one private
  `KeymapPanelState` while keeping persisted binding configuration separate.
- Made persisted duplicate and noncanonical key cleanup and custom-key dispatch
  deterministic, with resets reclaiming any shadowed default keys.
- Kept keymap rendering consistent with effective dispatch when a custom
  binding shadows another action's default key.

## Completed In The Fourteenth Pass

- Centralized playlist, keymap, and track-info visibility transitions in one
  private `ManagementPanelState`, making the two management panels mutually
  exclusive and cancelling key capture whenever keymap visibility ends.
- Moved shared presentation visibility and layout queries out of the filter
  module, and moved track-info panel coordination beside management-panel
  ownership.
- Split the large TUI behavior suite into command, filter, keymap, playlist,
  playback, browser, persistence, and presentation modules, leaving the parent
  test module as shared fixture support.

## TUI Structural Refactoring Complete

- Remaining coordinator fields are model/cache data, Ratatui presentation
  state, cross-pane focus, persisted preferences, playback session resources,
  and status output. Further grouping would broaden ownership or lifecycle
  changes without a clear structural payoff.
- Continue future TUI work from concrete feature, reliability, accessibility,
  or performance needs rather than field-count reduction.

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
