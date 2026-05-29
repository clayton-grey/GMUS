# GMUS

GMUS is an early Rust prototype for a small terminal music player inspired by
cmus, with macOS compatibility and a low compute footprint as first-class goals.

The initial implementation focuses on the foundation:

- SQLite-backed play history that is independent from any library membership.
- Tag scanning with `lofty`.
- Cover-art discovery from embedded tags and folder images.
- A small Ratatui shell that gives the application a real terminal surface.
- Thin traits for playback and OS media-session integrations.

## Current Commands

```sh
gmus
gmus scan ~/Music
gmus tui ~/Music
gmus art ~/Music/Album/track.flac
gmus stats
gmus record-play ~/Music/Album/track.flac --duration-ms 180000
```

By default, GMUS stores data in:

- macOS: `~/Library/Application Support/GMUS/gmus.sqlite3`
- other Unix-like systems: `$XDG_DATA_HOME/gmus/gmus.sqlite3` or
  `~/.local/share/gmus/gmus.sqlite3`

Cover art is cached next to the database under `art/`.

## Design Notes

The app intentionally does not require tracks to belong to a fixed library. A
track can be scanned, played, removed from a view, or moved on disk without
throwing away its play history. The MVP uses metadata plus duration as the
primary lightweight identity and records each play as an append-only event.

Playback is behind a trait. The current MVP uses Rodio's pure-Rust
Symphonia/CPAL path with common macOS library formats enabled, including MP3,
FLAC, AAC/M4A, ALAC, AIFF, CAF, Ogg Vorbis, and WAV.

The TUI is moving toward the cmus library view:

- left pane: artists, with expandable album rows
- right pane: album headers with years/durations and selectable tracks for the selected artist or album
- bottom info pane: metadata for the selected track, or inverted command help/output for `:` commands
- command/filter row: shown below the bottom info pane when active
- bottom strip: current track, position, playback state, and transient messages
- narrow terminals stack the artist pane above the track pane, with info still at the bottom

Keyboard control:

- `Tab`: switch between artist tree and track pane
- `Up` / `Down` or `j` / `k`: move selection
- `Enter`: play the first listed track for the selected tree item, or play the selected track
- `Space`: expand/collapse in the tree
- `e`: expand/collapse the selected artist
- `Left` / `Right` or `h` / `l`: seek -5/+5 seconds
- `,` / `.`: seek -1/+1 minute
- `x`: play
- `c` or `p`: pause/resume
- `v`: stop
- `b` / `z`: next or previous
- `C`: toggle continuous auto-advance
- `L`: cycle play target between library, artist, and album
- `R`: toggle repeat
- `S`: toggle shuffle
- `Ctrl-R`: refresh the library from the local database
- `i`: show/hide the info pane
- `I`: move the browser selection to the current track
- `/`: type a library filter, then `Enter` or `Tab` to apply, or `Esc` to clear
- `:`: type a command, then `Enter` to run it; `Tab` completes commands and paths
- `q` or `Ctrl-C`: quit

Playback advances through the active filtered track set and selected play target
when continuous mode is on, so next, previous, shuffle, repeat, and natural
auto-advance stay inside the current filter.

Library commands:

- `:add PATH`: scan a file or directory and add it as an active library root
- `:remove PATH`: remove a root from the active library without deleting metadata or play history
- `:update`: rescan active library roots
- `:update PATH`: scan or rescan one path and keep it active
- `:library`: show active and inactive library roots in the info pane
- `:filter TEXT`: apply a filter from command mode
- `:clear`: clear the active filter
- `:clear-output`, `:close`, or `:hide`: close command output and return the info pane to metadata

`Esc` closes command output before falling through to filter clearing. Normal
navigation/actions also return the info pane to selected-track metadata.

On macOS, the default build also publishes Now Playing metadata and listens for
system media-control events through `souvlaki`. The macOS backend also pumps a
small AppKit event loop from the TUI loop; this is required for reliable media
key callbacks in terminal apps without opening a visible window.

Cover art is still extracted and cached for macOS Now Playing metadata. In-terminal
art display is intentionally deferred to a future companion/widget or
protocol-backed solution so the core TUI stays light and stable.
