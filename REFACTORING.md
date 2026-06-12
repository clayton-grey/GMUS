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

## Next Structural Pass

- Group the large TUI `App` state into browser, playlist, input, playback, and
  integration state objects while preserving the existing coordinator API.
- Replace the three parallel playlist maps with one typed playlist cache so
  entry IDs, media IDs, and track indices cannot drift.
- Keep `db` as a facade while splitting migrations, settings, playlists,
  catalog, and history into focused implementation modules.
- Split the large TUI test module by concern after the state boundaries settle.

## Later Reliability And Performance Work

- Define an explicit best-effort policy for nested filesystem read failures
  during scans, not only metadata and artwork failures.
- Skip metadata and artwork parsing for unchanged files during rescans.
- Revisit playback listened-time accounting after long event-loop stalls.
- Preserve non-UTF-8 filesystem paths without lossy string conversion.
- Either add Windows path handling and CI or explicitly document Unix-like
  platform support.
