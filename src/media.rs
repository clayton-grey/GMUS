use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::PictureType;
use lofty::prelude::Accessor;
use lofty::tag::{ItemKey, Tag};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct EmbeddedArt {
    pub bytes: Vec<u8>,
    pub extension: &'static str,
}

#[derive(Debug, Clone)]
pub struct TrackMetadata {
    pub path: PathBuf,
    pub file_size: i64,
    pub modified_at: Option<i64>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub album_year: Option<i64>,
    pub composer: Option<String>,
    pub genre: Option<String>,
    pub track_number: Option<i64>,
    pub disc_number: Option<i64>,
    pub duration_ms: Option<i64>,
    pub embedded_art: Option<EmbeddedArt>,
}

impl TrackMetadata {
    pub fn fingerprint(&self) -> String {
        let mut basis = String::new();
        if self.title.is_some() || self.artist.is_some() || self.album.is_some() {
            basis.push_str("tags:v1:");
            push_norm(
                &mut basis,
                self.album_artist.as_deref().or(self.artist.as_deref()),
            );
            push_norm(&mut basis, self.album.as_deref());
            push_norm(&mut basis, self.title.as_deref());
            basis.push_str(&self.duration_ms.unwrap_or_default().to_string());
        } else {
            basis.push_str("file:v1:");
            basis.push_str(&self.file_size.to_string());
            basis.push(':');
            basis.push_str(&self.modified_at.unwrap_or_default().to_string());
            basis.push(':');
            basis.push_str(&self.path.to_string_lossy());
        }

        let mut hasher = Sha256::new();
        hasher.update(basis.as_bytes());
        hex::encode(hasher.finalize())
    }
}

pub fn read_track(path: &Path) -> Result<TrackMetadata> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("reading filesystem metadata for {}", path.display()))?;
    let modified_at = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as i64);

    let tagged = lofty::read_from_path(path)
        .with_context(|| format!("reading audio metadata from {}", path.display()))?;
    let properties = tagged.properties();
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());

    let embedded_art = tag.and_then(|tag| {
        tag.get_picture_type(PictureType::CoverFront)
            .or_else(|| tag.pictures().first())
            .and_then(|picture| {
                extension_for_mime(picture.mime_type()).map(|extension| (picture, extension))
            })
            .map(|(picture, extension)| EmbeddedArt {
                bytes: picture.data().to_vec(),
                extension,
            })
    });

    Ok(TrackMetadata {
        path: path.to_path_buf(),
        file_size: metadata.len() as i64,
        modified_at,
        title: tag.and_then(|tag| tag.title().map(|value| value.to_string())),
        artist: tag.and_then(|tag| tag.artist().map(|value| value.to_string())),
        album: tag.and_then(|tag| tag.album().map(|value| value.to_string())),
        album_artist: tag
            .and_then(|tag| tag.get_string(ItemKey::AlbumArtist))
            .map(ToOwned::to_owned),
        album_year: tag_album_year(tag),
        composer: tag_text(tag, ItemKey::Composer),
        genre: tag_text(tag, ItemKey::Genre),
        track_number: tag.and_then(|tag| tag.track().map(i64::from)),
        disc_number: tag.and_then(|tag| tag.disk().map(i64::from)),
        duration_ms: Some(properties.duration().as_millis() as i64).filter(|value| *value > 0),
        embedded_art,
    })
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

fn extension_for_mime(mime: Option<&lofty::picture::MimeType>) -> Option<&'static str> {
    let mime = mime?;
    let debug = format!("{mime:?}").to_ascii_lowercase();
    if debug.contains("jpeg") || debug.contains("jpg") {
        Some("jpg")
    } else if debug.contains("png") {
        Some("png")
    } else if debug.contains("webp") {
        Some("webp")
    } else {
        None
    }
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

fn tag_text(tag: Option<&Tag>, key: ItemKey) -> Option<String> {
    tag.and_then(|tag| tag.get_string(key))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
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

fn push_norm(out: &mut String, value: Option<&str>) {
    out.push(':');
    if let Some(value) = value {
        out.push_str(&value.trim().to_ascii_lowercase());
    }
}

#[cfg(test)]
mod tests {
    use super::parse_year;

    #[test]
    fn parses_year_from_tag_dates() {
        assert_eq!(parse_year("2018-05-11"), Some(2018));
        assert_eq!(parse_year("released 1997"), Some(1997));
        assert_eq!(parse_year("97"), None);
    }
}
