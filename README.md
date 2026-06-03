# GMUS

GMUS is an early Rust prototype for a small terminal music player inspired by
cmus, with macOS compatibility and a low compute footprint as first-class goals.

The initial implementation focuses on the foundation:

- SQLite-backed play history that is independent from any library membership.
- Tag scanning with `lofty`.
- Cover-art discovery from embedded tags and folder images.
- A small Ratatui shell that gives the application a real terminal surface.
- Thin traits for playback and OS integrations.

## Install And Build

GMUS is built with stable Rust:

```sh
cargo build
cargo install --path .
```

The default build enables Rodio playback and macOS media-session integration
when compiling on macOS. On non-macOS targets, the macOS feature resolves to a
no-op integration and the macOS-only crates are not compiled. Other useful build
modes:

```sh
cargo build --no-default-features
cargo build --features bundled-sqlite
cargo build --all-features
```

On Linux, the Rodio/CPAL stack may require system audio development packages
such as ALSA headers. The CI workflow installs `libasound2-dev` and
`pkg-config` for this. For the most portable SQLite build on Linux or Windows,
use `--features bundled-sqlite`.

## Development Checks

```sh
cargo fmt --check
cargo test --all-targets
cargo test --no-default-features --all-targets
cargo test --all-targets --features bundled-sqlite
cargo test --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
```

## CLI Commands

```sh
gmus
gmus scan ~/Music
gmus tui ~/Music
gmus art ~/Music/Album/track.flac
gmus play ~/Music/Album/track.flac
gmus stats
gmus record-play ~/Music/Album/track.flac --duration-ms 180000
gmus record-play ~/Music/Album/track.flac --duration-ms 45000 --completed=false
```

All commands accept `--db PATH` before the subcommand, for example
`gmus --db /tmp/gmus.sqlite3 stats`.

By default, GMUS stores data in:

- macOS: `~/Library/Application Support/GMUS/gmus.sqlite3`
- other Unix-like systems: `$XDG_DATA_HOME/gmus/gmus.sqlite3` or
  `~/.local/share/gmus/gmus.sqlite3`

Cover art is cached next to the database under `art/`.

On macOS, GMUS may also create a hidden notification helper bundle at
`~/Library/Application Support/GMUS/GMUS.app`. The helper gives macOS a real app
identity for track-change artwork overlays. It is launched only when GMUS posts
a track change, uses `LSUIElement` so it has no Dock or menu bar presence, and
exits after the overlay dismisses.

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

- `Tab`: switch between artist tree, track pane, and playlist pane when open
- `Up` / `Down` or `j` / `k`: move selection
- `Enter`: play the first listed track for the selected tree item, or play the selected track
- `Space`: expand/collapse in the tree
- `e`: expand/collapse the selected artist
- `Left` / `Right` or `h` / `l`: seek -5/+5 seconds
- `,` / `.`: seek -1/+1 minute
- `x`: play
- `c`: pause/resume
- `p`: open or focus the playlist pane
- `+` / `=`: add the selected track, artist, album, or playlist entry to the active playlist
- `-`: remove the selected track or playlist entry from the active playlist
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
- `:playlist NAME`: create/select a playlist and open the playlist pane
- `:playlist-clear NAME`: remove all tracks from a playlist
- `:playlist-delete NAME`: delete a playlist
- `:filter TEXT`: apply a filter from command mode
- `:clear`: clear the active filter
- `:clear-output`, `:close`, or `:hide`: close command output and return the info pane to metadata
- `:notifications [on|off|toggle|status]`: show or hide macOS track-change overlays

Most playlist commands also have short aliases in the TUI command bar: `:pl`,
`:pl-clear`, `:pl-delete`, `:playlist-rm`, and `:pl-rm`.

`Esc` closes command output before falling through to filter clearing. Normal
navigation/actions also return the info pane to selected-track metadata.

The TUI publishes owned integration events for track changes and playback state,
and consumes integration commands such as play, pause, next, previous, and seek.
On macOS, the default integration backend maps those events to Now Playing
metadata, listens for system media-control events through `souvlaki`, and shows
track-change artwork overlays through a hidden helper app. The helper keeps a
small AppKit-only surface and exits after each overlay. The macOS backend also
pumps a small AppKit event loop from the TUI loop; this is required for reliable
media key callbacks in terminal apps without opening a visible window.

OS-specific integrations live behind this event boundary so the core playback,
library, and TUI code can stay independent from platform APIs.

Cover art is extracted and cached during scans with embedded artwork preferred
over folder artwork. The macOS integration reuses that cached art for Now
Playing metadata and track-change overlays. In-terminal art display is
deferred to a future companion/widget or protocol-backed solution so the core
TUI stays light and stable.
