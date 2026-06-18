use anyhow::Result;
use rusqlite::Connection;

use crate::db::{self, LibraryTrack};

use super::browser::track_root_label;
use super::{App, FocusPane, InputState};

pub(super) fn track_search_text(track: &LibraryTrack) -> String {
    format!(
        "{} {} {} {} {} {} {} {} {} {} {} {}",
        track.display_title(),
        track.display_artist(),
        track.display_album(),
        track.album_artist.as_deref().unwrap_or_default(),
        track
            .album_year
            .map(|year| year.to_string())
            .unwrap_or_default(),
        track.release_date.as_deref().unwrap_or_default(),
        track.composer.as_deref().unwrap_or_default(),
        track.genre.as_deref().unwrap_or_default(),
        track_root_label(track).unwrap_or_default(),
        if track.compilation { "compilation" } else { "" },
        track.play_count,
        track.path
    )
    .to_ascii_lowercase()
}

#[derive(Debug, Default)]
pub(super) struct FilterQuery {
    terms: Vec<FilterTerm>,
    warnings: Vec<String>,
}

impl FilterQuery {
    pub(super) fn parse(input: &str) -> Self {
        let mut query = Self::default();
        for token in split_filter_tokens(input) {
            match FilterTerm::parse(&token) {
                Ok(Some(term)) => query.terms.push(term),
                Ok(None) => {}
                Err(warning) => {
                    query.warnings.push(warning);
                    query.terms.push(FilterTerm::Invalid);
                }
            }
        }
        query
    }

    pub(super) fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    pub(super) fn matches(&self, track: &LibraryTrack, haystack: &str) -> bool {
        self.terms.iter().all(|term| term.matches(track, haystack))
    }

    pub(super) fn warning(&self) -> Option<&str> {
        self.warnings.first().map(String::as_str)
    }
}

#[derive(Debug)]
enum FilterTerm {
    Bare {
        needle: String,
        negated: bool,
    },
    Field {
        field: FilterField,
        matcher: FilterMatcher,
        negated: bool,
    },
    Invalid,
}

impl FilterTerm {
    fn parse(token: &str) -> std::result::Result<Option<Self>, String> {
        let token = token.trim();
        if token.is_empty() {
            return Ok(None);
        }

        let (negated, body) = token
            .strip_prefix('-')
            .map(|body| (true, body.trim()))
            .unwrap_or((false, token));
        if body.is_empty() {
            return Ok(None);
        }

        let Some((field_name, value)) = body.split_once(':') else {
            return Ok(Some(Self::Bare {
                needle: body.to_ascii_lowercase(),
                negated,
            }));
        };

        let Some(field) = FilterField::parse(field_name) else {
            return Err(format!("unknown filter field: {field_name}"));
        };

        let value = value.trim();
        if value.is_empty() {
            return Ok(None);
        }

        let matcher = field.matcher(value)?;
        Ok(Some(Self::Field {
            field,
            matcher,
            negated,
        }))
    }

    fn matches(&self, track: &LibraryTrack, haystack: &str) -> bool {
        match self {
            Self::Bare { needle, negated } => apply_negation(haystack.contains(needle), *negated),
            Self::Field {
                field,
                matcher,
                negated,
            } => apply_negation(field.matches(track, matcher), *negated),
            Self::Invalid => false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum FilterField {
    Title,
    Artist,
    Album,
    AlbumArtist,
    Year,
    ReleaseDate,
    Genre,
    Composer,
    Root,
    Path,
    Compilation,
    Plays,
    TrackNumber,
    DiscNumber,
}

impl FilterField {
    fn parse(field: &str) -> Option<Self> {
        match field.trim().to_ascii_lowercase().as_str() {
            "title" | "track" | "name" => Some(Self::Title),
            "artist" => Some(Self::Artist),
            "album" => Some(Self::Album),
            "albumartist" | "album_artist" | "album-artist" | "aa" => Some(Self::AlbumArtist),
            "year" => Some(Self::Year),
            "date" | "released" | "release" => Some(Self::ReleaseDate),
            "genre" => Some(Self::Genre),
            "composer" => Some(Self::Composer),
            "root" | "library" | "library_root" | "library-root" => Some(Self::Root),
            "path" | "file" => Some(Self::Path),
            "comp" | "compilation" => Some(Self::Compilation),
            "plays" | "playcount" | "play_count" | "play-count" => Some(Self::Plays),
            "trackno" | "track_no" | "track_number" | "track-number" | "number" => {
                Some(Self::TrackNumber)
            }
            "disc" | "discno" | "disc_no" | "disc_number" | "disc-number" => Some(Self::DiscNumber),
            _ => None,
        }
    }

    fn matcher(self, value: &str) -> std::result::Result<FilterMatcher, String> {
        match self {
            Self::Year | Self::Plays | Self::TrackNumber | Self::DiscNumber => {
                parse_number_matcher(value)
                    .map(FilterMatcher::Number)
                    .ok_or_else(|| format!("expected a number for {}", self.name()))
            }
            Self::Compilation => parse_bool(value)
                .map(FilterMatcher::Bool)
                .ok_or_else(|| String::from("compilation expects true or false")),
            Self::ReleaseDate => parse_number_matcher(value)
                .map(FilterMatcher::Number)
                .or_else(|| Some(FilterMatcher::Text(value.to_ascii_lowercase())))
                .ok_or_else(|| String::from("expected a date or year")),
            _ => Ok(FilterMatcher::Text(value.to_ascii_lowercase())),
        }
    }

    fn matches(self, track: &LibraryTrack, matcher: &FilterMatcher) -> bool {
        match (self, matcher) {
            (Self::Title, FilterMatcher::Text(needle)) => {
                text_matches(track.display_title(), needle)
            }
            (Self::Artist, FilterMatcher::Text(needle)) => {
                text_matches(track.display_artist(), needle)
            }
            (Self::Album, FilterMatcher::Text(needle)) => {
                text_matches(track.display_album(), needle)
            }
            (Self::AlbumArtist, FilterMatcher::Text(needle)) => {
                optional_text_matches(track.album_artist.as_deref(), needle)
            }
            (Self::Genre, FilterMatcher::Text(needle)) => {
                optional_text_matches(track.genre.as_deref(), needle)
            }
            (Self::Composer, FilterMatcher::Text(needle)) => {
                optional_text_matches(track.composer.as_deref(), needle)
            }
            (Self::Root, FilterMatcher::Text(needle)) => {
                optional_text_matches(track.library_root.as_deref(), needle)
                    || track_root_label(track).is_some_and(|root| text_matches(&root, needle))
            }
            (Self::Path, FilterMatcher::Text(needle)) => text_matches(&track.path, needle),
            (Self::ReleaseDate, FilterMatcher::Text(needle)) => {
                optional_text_matches(track.release_date.as_deref(), needle)
                    || track
                        .album_year
                        .is_some_and(|year| text_matches(&year.to_string(), needle))
            }
            (Self::Year, FilterMatcher::Number(matcher)) => matcher.matches(track_year(track)),
            (Self::ReleaseDate, FilterMatcher::Number(matcher)) => {
                matcher.matches(track_year(track))
            }
            (Self::Plays, FilterMatcher::Number(matcher)) => {
                matcher.matches(Some(track.play_count))
            }
            (Self::TrackNumber, FilterMatcher::Number(matcher)) => {
                matcher.matches(track.track_number)
            }
            (Self::DiscNumber, FilterMatcher::Number(matcher)) => {
                matcher.matches(track.disc_number)
            }
            (Self::Compilation, FilterMatcher::Bool(value)) => track.compilation == *value,
            _ => false,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Artist => "artist",
            Self::Album => "album",
            Self::AlbumArtist => "albumartist",
            Self::Year => "year",
            Self::ReleaseDate => "date",
            Self::Genre => "genre",
            Self::Composer => "composer",
            Self::Root => "root",
            Self::Path => "path",
            Self::Compilation => "compilation",
            Self::Plays => "plays",
            Self::TrackNumber => "trackno",
            Self::DiscNumber => "disc",
        }
    }
}

#[derive(Debug)]
enum FilterMatcher {
    Text(String),
    Bool(bool),
    Number(NumberMatcher),
}

#[derive(Debug)]
enum NumberMatcher {
    Equal(i64),
    Greater(i64),
    GreaterEqual(i64),
    Less(i64),
    LessEqual(i64),
    Range(Option<i64>, Option<i64>),
}

impl NumberMatcher {
    fn matches(&self, value: Option<i64>) -> bool {
        let Some(value) = value else {
            return false;
        };
        match self {
            Self::Equal(target) => value == *target,
            Self::Greater(target) => value > *target,
            Self::GreaterEqual(target) => value >= *target,
            Self::Less(target) => value < *target,
            Self::LessEqual(target) => value <= *target,
            Self::Range(start, end) => {
                start.is_none_or(|start| value >= start) && end.is_none_or(|end| value <= end)
            }
        }
    }
}

fn split_filter_tokens(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;

    for character in input.chars() {
        if escaped {
            token.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if let Some(quote_char) = quote {
            if character == quote_char {
                quote = None;
            } else {
                token.push(character);
            }
            continue;
        }
        match character {
            '"' | '\'' => quote = Some(character),
            character if character.is_whitespace() => {
                if !token.is_empty() {
                    tokens.push(std::mem::take(&mut token));
                }
            }
            character => token.push(character),
        }
    }

    if escaped {
        token.push('\\');
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    tokens
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => Some(true),
        "0" | "false" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

fn parse_number_matcher(value: &str) -> Option<NumberMatcher> {
    let value = value.trim();
    for (prefix, matcher) in [
        (
            ">=",
            NumberMatcher::GreaterEqual as fn(i64) -> NumberMatcher,
        ),
        ("<=", NumberMatcher::LessEqual as fn(i64) -> NumberMatcher),
        (">", NumberMatcher::Greater as fn(i64) -> NumberMatcher),
        ("<", NumberMatcher::Less as fn(i64) -> NumberMatcher),
    ] {
        if let Some(rest) = value.strip_prefix(prefix) {
            return parse_filter_i64(rest).map(matcher);
        }
    }

    if let Some((start, end)) = value.split_once("..") {
        let start = parse_optional_filter_i64(start)?;
        let end = parse_optional_filter_i64(end)?;
        return (start.is_some() || end.is_some()).then_some(NumberMatcher::Range(start, end));
    }

    parse_filter_i64(value).map(NumberMatcher::Equal)
}

fn parse_filter_i64(value: &str) -> Option<i64> {
    value.trim().parse().ok()
}

fn parse_optional_filter_i64(value: &str) -> Option<Option<i64>> {
    let value = value.trim();
    if value.is_empty() {
        Some(None)
    } else {
        parse_filter_i64(value).map(Some)
    }
}

fn track_year(track: &LibraryTrack) -> Option<i64> {
    track.album_year.or_else(|| {
        track
            .release_date
            .as_deref()
            .and_then(|date| date.as_bytes().windows(4).find_map(parse_year_window))
    })
}

fn parse_year_window(window: &[u8]) -> Option<i64> {
    window
        .iter()
        .all(u8::is_ascii_digit)
        .then(|| std::str::from_utf8(window).ok()?.parse().ok())
        .flatten()
}

fn apply_negation(value: bool, negated: bool) -> bool {
    if negated {
        !value
    } else {
        value
    }
}

fn text_matches(value: &str, needle: &str) -> bool {
    value.to_ascii_lowercase().contains(needle)
}

fn optional_text_matches(value: Option<&str>, needle: &str) -> bool {
    value.is_some_and(|value| text_matches(value, needle))
}

impl App {
    pub(super) fn edit_filter(&mut self, edit: impl FnOnce(&mut InputState)) {
        let selected_tree_entry = self.selected_tree_entry().cloned();
        let selected_media_item_id = self.selected_playable_media_item_id();
        edit(&mut self.input);
        self.sync_selection_preserving_browser_anchors(
            selected_tree_entry.as_ref(),
            selected_media_item_id,
        );
    }

    pub(super) fn filter_display(&self) -> &str {
        if self.input.filter().is_empty() {
            "none"
        } else {
            self.input.filter()
        }
    }

    pub(super) fn confirm_filter(&mut self, conn: &Connection) -> Result<()> {
        let warning = FilterQuery::parse(self.input.filter())
            .warning()
            .map(str::to_string);
        let selected_tree_entry = self.selected_tree_entry().cloned();
        let selected_media_item_id = self.selected_playable_media_item_id();
        self.input.finish_filter();
        self.focus = FocusPane::Tree;
        self.reset_shuffle_order();
        self.sync_selection_preserving_browser_anchors(
            selected_tree_entry.as_ref(),
            selected_media_item_id,
        );
        self.message = warning.unwrap_or_else(|| format!("filter: {}", self.filter_display()));
        self.save_filter_state(conn)?;
        Ok(())
    }

    pub(super) fn clear_filter(&mut self, conn: &Connection) -> Result<()> {
        let selected_tree_entry = self.selected_tree_entry().cloned();
        let selected_media_item_id = self.selected_playable_media_item_id();

        self.input.clear_filter();
        self.reset_shuffle_order();
        self.sync_selection_preserving_browser_anchors(
            selected_tree_entry.as_ref(),
            selected_media_item_id,
        );
        self.message = String::from("filter cleared");
        self.save_filter_state(conn)?;
        Ok(())
    }

    pub(super) fn save_filter_state(&self, conn: &Connection) -> Result<()> {
        if self.restore_filter {
            db::save_filter(conn, self.input.filter())?;
        }
        Ok(())
    }

    pub(super) fn filter_bar_visible(&self) -> bool {
        self.input.kind() == super::InputKind::Filter || !self.input.filter().is_empty()
    }
}
