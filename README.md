# GMUS

GMUS is a cmus-inspired terminal music player with a unified library browser,
metadata view, playlist manager, command bar, and local playback history. It
keeps the fast keyboard feel of cmus while adding richer filtering, persistent
UI preferences, editable hotkeys, macOS media integration, and a SQLite-backed
library/history store.

## Features

- Artist, album, compilation, and playlist browsing in a two-pane terminal UI
- Text search plus fielded filters for title, artist, album, genre, year,
  play count, library root, path, and more
- Playlists with duplicate entries, playlist-local ordering, and add/remove
  shortcuts from library or playlist views
- Editable keymap pane with persisted custom hotkeys and reserved recovery keys
- Persistent restart state for the last played track, confirmed filter, keymap,
  and pane-size adjustments
- Rodio/Symphonia playback for common formats including MP3, FLAC, AAC/M4A,
  ALAC, AIFF, CAF, Ogg Vorbis, and WAV
- Cover art extraction and caching for metadata, CLI inspection, and macOS
  integrations
- macOS Now Playing metadata, media-key handling, and optional track-change
  artwork notifications
- CLI commands for scanning, playback, cover-art lookup, play-history recording,
  and database statistics

## Install and Build

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

### Supported Platforms

GMUS supports macOS and Linux. CI runs the full test suite on both platforms,
and the application currently assumes Unix-like filesystem paths and data
directories. Windows is not supported yet.

On Linux, the Rodio/CPAL stack may require system audio development packages
such as ALSA headers. The CI workflow installs `libasound2-dev` and
`pkg-config` for this. For the most portable SQLite build on Linux or Windows,
use `--features bundled-sqlite`; this makes SQLite itself portable but does not
add Windows application support.

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

## Terminal Interface

Running `gmus` launches the TUI. Running `gmus tui PATH` scans a file or
directory first, then opens the interface.

The TUI is organized around a cmus-style browser:

- left pane: artists, compilation groups, and playlists with expandable album
  or playlist rows
- right pane: album headers, playlist headers, disc dividers, durations, play
  counts, and selectable tracks for the current library scope
- bottom management pane: selected-track metadata, keymap editing, playlist
  management, filter help, or inverted command help/output
- command/filter/rate row: shown below the bottom management pane while typing
- status rows: current track, playback position, playback modes, and transient
  feedback
- narrow terminals stack the library pane above the track pane, with the bottom
  management pane preserved

### Keyboard Control

- `Tab`: switch between artist tree, track pane, and bottom management pane when open
- `Up` / `Down`: move selection
- `PageUp` / `PageDown`: move selection by a page
- `Enter`: play the first listed track for the selected tree item, or play the selected track
- `Space`: expand/collapse in the tree
- `e`: expand/collapse the selected artist
- `Left` / `Right` or `h` / `l`: seek -5/+5 seconds
- `,` / `.`: seek -1/+1 minute
- `x`: play
- `c`: pause/resume
- `p`: open or focus the playlist pane
- `k`: open or focus the keymap pane
- In the keymap pane, `Enter` edits non-reserved mappings, `Esc` cancels
  editing, and `Backspace` resets it to default
- `Enter`, `Esc`, `:`, and `Ctrl-C` combinations are reserved for their default
  behaviors and cannot be edited or mapped to another action
- `{` / `}`: nudge the selected pane boundary left/up or right/down
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
- `r`: type a playback rate, then `Enter` or `Tab` to apply, or `Esc` to cancel
- `/`: type a library filter, then `Enter` or `Tab` to apply, or `Esc` to clear
- `:`: type a command, then `Enter` to run it; `Tab` completes commands and paths
- `q` or `Ctrl-C`: quit

Pane sizing starts from the default relative layout and stores small offsets in
the local database. In the wide layout, `{` moves the selected browser boundary
left and `}` moves it right. In stacked or bottom-pane layouts, `{` moves the
selected boundary up and `}` moves it down. With the tracks pane focused, the
adjustment is relative to the tracks pane's own edge.

The keymap pane is itself editable. Select a non-reserved row and press `Enter`
to capture an additional key binding, or press bare `Backspace`/`Delete` while
capturing to reset that action to its defaults. `Enter`, `Esc`, `:`, and
`Ctrl-C` combinations are reserved so activation, cancellation, command entry,
and quitting remain recoverable.

### Filtering

Filters can be typed with `/` or applied through `:filter TEXT`. Bare terms
search title, artist, album, genre, composer, root, date, play count, and path.
Fielded terms use `field:value`, quoted values are supported, and prefixing a
term with `-` excludes it.

Useful fields:

- text fields: `title`, `artist`, `album`, `albumartist`, `genre`, `composer`,
  `root`, `path`, `date`
- numeric fields: `year`, `plays`, `trackno`, `disc`
- boolean field: `compilation`

Numeric filters support exact values, comparisons, and ranges:

```text
genre:ambient year:2010..2020
root:Instrumental -compilation:true plays:>5
artist:"Brian Eno" album:apollo
```

Playback advances through the active filtered track set and selected play target
when continuous mode is on, so next, previous, shuffle, repeat, and natural
auto-advance stay inside the current filter.
On restart, the browser restores the last played track at the artist level and
restores the last confirmed filter when those restore settings are enabled.
Both are on by default and can be toggled from the command bar.

### Command Bar

- `:add PATH`: scan a file or directory and add it as an active library root
- `:remove PATH`: remove a root from the active library without deleting metadata or play history
- `:update`: rescan active library roots
- `:update PATH`: scan or rescan one path and keep it active
- `:library`: show active and inactive library roots in the info pane
- `:playlist NAME`: create/select a playlist and open the playlist pane
- `:playlist-clear NAME`: remove all tracks from a playlist
- `:playlist-delete NAME`: delete a playlist
- `:keymap`: show the keymap pane
- `:keymap-reset`: reset custom key mappings to defaults
- `:column-layout-width [WIDTH|reset|status]`: set the widest terminal width that uses stacked browser panes; columns begin one column above it, and the default is `75`
- `:rate [RATE|PERCENT|reset]`: show or change playback rate, for example `:rate 0.75`, `:rate 75`, or `:rate 75%`
- `:restore-filter [on|off|toggle|status]`: toggle whether the last filter is restored on startup
- `:restore-track [on|off|toggle|status]`: toggle whether the last played
  track is restored on startup
- `:filter TEXT`: apply a filter from command mode
- `:clear`: clear the active filter
- `:clear-output`, `:close`, or `:hide`: close command output and return the info pane to metadata
- `:notifications [on|off|toggle|status]`: show or hide macOS track-change overlays on macOS builds

Common aliases include `:rm`, `:u`, `:roots`, `:pl`, `:pl-clear`,
`:pl-delete`, `:playlist-rm`, `:pl-rm`, `:keys`, `:keys-reset`, `:f`,
`:clear-filter`, and on macOS builds, `:notify`.

Playback rates range from `0.25x` to `4.0x` and remain active across tracks.
Rodio changes pitch along with playback speed; it does not perform pitch-preserving
time stretching. A non-default rate is shown beside the track time in the
playback status row. Unmarked values above `4` are interpreted as percentages.

`Esc` closes command output before falling through to filter clearing. Normal
navigation/actions also return the info pane to selected-track metadata.

## Implementation Notes

GMUS intentionally does not require tracks to belong permanently to a fixed
library. A track can be scanned, played, removed from a view, or moved on disk
without throwing away its play history. Metadata plus duration provide a
lightweight identity, while plays are stored as append-only events.

Playback is behind a small backend trait. The default backend uses Rodio's
pure-Rust Symphonia/CPAL path with common macOS library formats enabled.

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
Playing metadata and track-change overlays. The terminal UI stays text-focused,
with artwork handled by CLI commands and platform integrations.
