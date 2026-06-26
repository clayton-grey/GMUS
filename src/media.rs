use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::{MimeType, PictureType};
use lofty::prelude::Accessor;
use lofty::tag::{ItemKey, Tag};
use sha2::{Digest, Sha256};

pub const SCAN_VERSION: i64 = 1;

#[derive(Debug, Clone)]
pub struct EmbeddedArt {
    pub bytes: Vec<u8>,
    pub extension: &'static str,
}

#[derive(Debug, Clone)]
pub struct ParsedTrack {
    pub metadata: TrackMetadata,
    pub embedded_art: Option<EmbeddedArt>,
}

#[derive(Debug, Clone)]
pub struct TrackMetadata {
    pub path: PathBuf,
    pub file_size: i64,
    pub modified_at: Option<i64>,
    pub modified_at_ns: Option<i64>,
    pub fs_device: Option<i64>,
    pub fs_inode: Option<i64>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub album_year: Option<i64>,
    pub release_date: Option<String>,
    pub composer: Option<String>,
    pub genre: Option<String>,
    pub track_number: Option<i64>,
    pub track_total: Option<i64>,
    pub disc_number: Option<i64>,
    pub disc_total: Option<i64>,
    pub duration_ms: Option<i64>,
    pub compilation: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct FileStamp {
    pub file_size: i64,
    pub modified_at: Option<i64>,
    pub modified_at_ns: Option<i64>,
    pub fs_device: Option<i64>,
    pub fs_inode: Option<i64>,
}

impl TrackMetadata {
    pub fn duplicate_key(&self) -> String {
        let mut basis = Vec::new();
        if self.title.is_some() || self.artist.is_some() || self.album.is_some() {
            basis.extend_from_slice(b"tags:v2");
            push_norm_field(
                &mut basis,
                self.album_artist.as_deref().or(self.artist.as_deref()),
            );
            push_norm_field(&mut basis, self.album.as_deref());
            push_norm_field(&mut basis, self.title.as_deref());
            push_optional_i64(&mut basis, self.duration_ms);
        } else {
            basis.extend_from_slice(b"file:v2");
            push_i64(&mut basis, self.file_size);
            push_optional_i64(&mut basis, self.modified_at);
            push_bytes(&mut basis, native_path_bytes(&self.path));
        }

        let mut hasher = Sha256::new();
        hasher.update(&basis);
        hex::encode(hasher.finalize())
    }
}

pub fn read_track(path: &Path) -> Result<TrackMetadata> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("reading filesystem metadata for {}", path.display()))?;
    read_track_with_stamp(path, file_stamp(&metadata))
}

pub fn read_embedded_art(path: &Path) -> Result<Option<EmbeddedArt>> {
    let tagged = lofty::read_from_path(path)
        .with_context(|| format!("reading embedded cover art from {}", path.display()))?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    Ok(embedded_art_from_tag(tag))
}

pub fn read_track_with_stamp(path: &Path, stamp: FileStamp) -> Result<TrackMetadata> {
    let tagged = lofty::read_from_path(path)
        .with_context(|| format!("reading audio metadata from {}", path.display()))?;
    let properties = tagged.properties();
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    Ok(track_metadata_from_tag(
        path,
        stamp,
        properties.duration().as_millis() as i64,
        tag,
    ))
}

pub fn read_track_and_art_with_stamp(path: &Path, stamp: FileStamp) -> Result<ParsedTrack> {
    let tagged = lofty::read_from_path(path)
        .with_context(|| format!("reading audio metadata from {}", path.display()))?;
    let properties = tagged.properties();
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    Ok(ParsedTrack {
        metadata: track_metadata_from_tag(
            path,
            stamp,
            properties.duration().as_millis() as i64,
            tag,
        ),
        embedded_art: embedded_art_from_tag(tag),
    })
}

fn track_metadata_from_tag(
    path: &Path,
    stamp: FileStamp,
    duration_ms: i64,
    tag: Option<&Tag>,
) -> TrackMetadata {
    TrackMetadata {
        path: path.to_path_buf(),
        file_size: stamp.file_size,
        modified_at: stamp.modified_at,
        modified_at_ns: stamp.modified_at_ns,
        fs_device: stamp.fs_device,
        fs_inode: stamp.fs_inode,
        title: tag.and_then(|tag| tag.title().map(|value| value.to_string())),
        artist: tag.and_then(|tag| tag.artist().map(|value| value.to_string())),
        album: tag.and_then(|tag| tag.album().map(|value| value.to_string())),
        album_artist: tag
            .and_then(|tag| tag.get_string(ItemKey::AlbumArtist))
            .map(ToOwned::to_owned),
        album_year: tag_album_year(tag),
        release_date: tag_release_date(tag),
        composer: tag_text(tag, ItemKey::Composer),
        genre: tag_text(tag, ItemKey::Genre),
        track_number: tag.and_then(|tag| tag.track().map(i64::from)),
        track_total: tag.and_then(|tag| tag.track_total().map(i64::from)),
        disc_number: tag.and_then(|tag| tag.disk().map(i64::from)),
        disc_total: tag.and_then(|tag| tag.disk_total().map(i64::from)),
        duration_ms: Some(duration_ms).filter(|value| *value > 0),
        compilation: tag_bool(tag, ItemKey::FlagCompilation),
    }
}

pub fn file_stamp(metadata: &fs::Metadata) -> FileStamp {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok());
    let (fs_device, fs_inode) = filesystem_identity(metadata);
    FileStamp {
        file_size: metadata.len() as i64,
        modified_at: modified.map(|duration| duration.as_secs() as i64),
        modified_at_ns: modified.and_then(|duration| i64::try_from(duration.as_nanos()).ok()),
        fs_device,
        fs_inode,
    }
}

#[cfg(unix)]
fn filesystem_identity(metadata: &fs::Metadata) -> (Option<i64>, Option<i64>) {
    use std::os::unix::fs::MetadataExt;

    (Some(metadata.dev() as i64), Some(metadata.ino() as i64))
}

#[cfg(not(unix))]
fn filesystem_identity(_metadata: &fs::Metadata) -> (Option<i64>, Option<i64>) {
    (None, None)
}

pub fn is_audio_path(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return false;
    };

    matches!(
        extension.to_ascii_lowercase().as_str(),
        "aac"
            | "aiff"
            | "aif"
            | "ape"
            | "flac"
            | "m4a"
            | "mp3"
            | "mp4"
            | "mpc"
            | "ogg"
            | "opus"
            | "speex"
            | "wav"
            | "wv"
    )
}

fn extension_for_mime(mime: Option<&MimeType>) -> Option<&'static str> {
    match mime?.as_str().to_ascii_lowercase().as_str() {
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

fn embedded_art_from_tag(tag: Option<&Tag>) -> Option<EmbeddedArt> {
    tag.and_then(|tag| {
        let pictures = tag.pictures();
        pictures
            .iter()
            .filter(|picture| picture.pic_type() == PictureType::CoverFront)
            .chain(
                pictures
                    .iter()
                    .filter(|picture| picture.pic_type() != PictureType::CoverFront),
            )
            .find_map(|picture| {
                extension_for_mime(picture.mime_type()).map(|extension| (picture, extension))
            })
            .map(|(picture, extension)| EmbeddedArt {
                bytes: picture.data().to_vec(),
                extension,
            })
    })
}

fn tag_album_year(tag: Option<&Tag>) -> Option<i64> {
    let tag = tag?;
    tag.date()
        .map(|date| i64::from(date.year))
        .or_else(|| tag_year_from_key(tag, ItemKey::OriginalReleaseDate))
        .or_else(|| tag_year_from_key(tag, ItemKey::ReleaseDate))
        .filter(|year| (1000..=9999).contains(year))
}

fn tag_year_from_key(tag: &Tag, key: ItemKey) -> Option<i64> {
    tag.get_string(key).and_then(parse_year)
}

fn tag_release_date(tag: Option<&Tag>) -> Option<String> {
    let tag = tag?;
    tag_text(Some(tag), ItemKey::OriginalReleaseDate)
        .or_else(|| tag_text(Some(tag), ItemKey::ReleaseDate))
        .or_else(|| tag.date().map(|date| date.to_string()))
        .map(normalize_release_date)
        .filter(|value| !value.is_empty())
}

fn tag_text(tag: Option<&Tag>, key: ItemKey) -> Option<String> {
    tag.and_then(|tag| tag.get_string(key))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn tag_bool(tag: Option<&Tag>, key: ItemKey) -> bool {
    let Some(value) = tag_text(tag, key) else {
        return false;
    };
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "y"
    )
}

fn normalize_release_date(value: String) -> String {
    value.trim().replace('/', "-")
}

fn parse_year(value: &str) -> Option<i64> {
    value.as_bytes().windows(4).find_map(|window| {
        window
            .iter()
            .all(u8::is_ascii_digit)
            .then(|| std::str::from_utf8(window).ok()?.parse().ok())
            .flatten()
    })
}

fn push_norm_field(out: &mut Vec<u8>, value: Option<&str>) {
    match value {
        Some(value) => {
            out.push(1);
            push_bytes(out, value.trim().to_ascii_lowercase().as_bytes());
        }
        None => out.push(0),
    }
}

fn push_optional_i64(out: &mut Vec<u8>, value: Option<i64>) {
    match value {
        Some(value) => {
            out.push(1);
            push_i64(out, value);
        }
        None => out.push(0),
    }
}

fn push_i64(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn push_bytes(out: &mut Vec<u8>, value: &[u8]) {
    out.extend_from_slice(&(value.len() as u64).to_be_bytes());
    out.extend_from_slice(value);
}

#[cfg(unix)]
fn native_path_bytes(path: &Path) -> &[u8] {
    use std::os::unix::ffi::OsStrExt;

    path.as_os_str().as_bytes()
}

#[cfg(not(unix))]
fn native_path_bytes(path: &Path) -> &[u8] {
    path.to_str().unwrap_or_default().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::{embedded_art_from_tag, parse_year, TrackMetadata};
    use lofty::picture::{MimeType, Picture, PictureType};
    use lofty::tag::{Tag, TagType};
    use std::path::PathBuf;

    #[test]
    fn parses_year_from_tag_dates() {
        assert_eq!(parse_year("2018-05-11"), Some(2018));
        assert_eq!(parse_year("released 1997"), Some(1997));
        assert_eq!(parse_year("97"), None);
    }

    #[test]
    fn duplicate_key_distinguishes_tag_field_delimiter_collisions() {
        let first = tagged_track(Some("artist:album"), Some("title"), Some(120_000));
        let second = tagged_track(Some("artist"), Some("album:title"), Some(120_000));

        assert_ne!(first.duplicate_key(), second.duplicate_key());
    }

    #[test]
    fn duplicate_key_distinguishes_title_duration_boundary_collisions() {
        let first = tagged_track(Some("artist"), Some("title1"), Some(23));
        let second = tagged_track(Some("artist"), Some("title"), Some(123));

        assert_ne!(first.duplicate_key(), second.duplicate_key());
    }

    #[test]
    fn embedded_art_falls_back_when_front_cover_mime_is_unsupported() {
        let mut tag = Tag::new(TagType::Id3v2);
        tag.push_picture(
            Picture::unchecked(vec![1])
                .pic_type(PictureType::CoverFront)
                .mime_type(MimeType::Gif)
                .build(),
        );
        tag.push_picture(
            Picture::unchecked(vec![2])
                .pic_type(PictureType::CoverBack)
                .mime_type(MimeType::Png)
                .build(),
        );

        let art = embedded_art_from_tag(Some(&tag)).unwrap();

        assert_eq!(art.extension, "png");
        assert_eq!(art.bytes, vec![2]);
    }

    fn tagged_track(
        artist: Option<&str>,
        title: Option<&str>,
        duration_ms: Option<i64>,
    ) -> TrackMetadata {
        TrackMetadata {
            path: PathBuf::from("/tmp/song.flac"),
            file_size: 10,
            modified_at: Some(1),
            modified_at_ns: Some(1_000_000_000),
            fs_device: None,
            fs_inode: None,
            title: title.map(str::to_owned),
            artist: artist.map(str::to_owned),
            album: Some("album".into()),
            album_artist: None,
            album_year: None,
            release_date: None,
            composer: None,
            genre: None,
            track_number: None,
            track_total: None,
            disc_number: None,
            disc_total: None,
            duration_ms,
            compilation: false,
        }
    }
}
