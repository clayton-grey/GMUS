use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::media::{self, EmbeddedArt, TrackMetadata};
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

const FOLDER_ART_NAMES: &[&str] = &[
    "cover.jpg",
    "cover.jpeg",
    "cover.png",
    "folder.jpg",
    "folder.jpeg",
    "folder.png",
    "front.jpg",
    "front.jpeg",
    "front.png",
];

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ArtworkMode {
    Cached,
    OnDemand,
}

impl ArtworkMode {
    pub fn from_env() -> Self {
        Self::from_value(env::var("GMUS_ARTWORK_MODE").ok().as_deref())
    }

    fn from_value(value: Option<&str>) -> Self {
        match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            Some("cached" | "cache" | "eager" | "scan") => Self::Cached,
            Some("ondemand" | "on-demand" | "lazy" | "file") => Self::OnDemand,
            _ => Self::OnDemand,
        }
    }

    pub fn scan_version(self) -> i64 {
        match self {
            Self::Cached => media::SCAN_VERSION,
            Self::OnDemand => media::SCAN_VERSION + 100,
        }
    }

    pub fn caches_during_scan(self) -> bool {
        self == Self::Cached
    }
}

#[cfg(all(target_os = "macos", feature = "macos-media-session"))]
pub fn trace_enabled() -> bool {
    matches!(
        env::var("GMUS_ARTWORK_TRACE").ok().as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

pub fn cache_cover_for_track(
    track: &TrackMetadata,
    embedded_art: Option<&EmbeddedArt>,
    art_dir: &Path,
    media_item_id: i64,
) -> Result<Option<PathBuf>> {
    cache_cover_for_audio_path(&track.path, embedded_art, art_dir, media_item_id)
}

pub fn cache_cover_for_audio_path(
    audio_path: &Path,
    embedded_art: Option<&EmbeddedArt>,
    art_dir: &Path,
    media_item_id: i64,
) -> Result<Option<PathBuf>> {
    if let Some(embedded) = embedded_art {
        return cache_embedded_cover(embedded, art_dir, media_item_id).map(Some);
    }

    if let Some(folder_art) = find_folder_art(audio_path) {
        fs::create_dir_all(art_dir)
            .with_context(|| format!("creating art cache {}", art_dir.display()))?;
        let extension = folder_art
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("img");
        let path = art_dir.join(format!("{media_item_id}.{extension}"));
        copy_cover_if_changed(&folder_art, &path).with_context(|| {
            format!(
                "copying folder cover art from {} to {}",
                folder_art.display(),
                path.display()
            )
        })?;
        return Ok(Some(path));
    }

    Ok(None)
}

pub fn materialize_cover_for_audio_path(
    audio_path: &Path,
    art_dir: &Path,
    media_item_id: i64,
) -> Result<Option<PathBuf>> {
    if let Some(embedded) = media::read_embedded_art(audio_path)? {
        return cache_embedded_cover(&embedded, art_dir, media_item_id).map(Some);
    }

    Ok(find_folder_art(audio_path))
}

fn cache_embedded_cover(
    embedded: &EmbeddedArt,
    art_dir: &Path,
    media_item_id: i64,
) -> Result<PathBuf> {
    fs::create_dir_all(art_dir)
        .with_context(|| format!("creating art cache {}", art_dir.display()))?;
    let path = art_dir.join(format!("{media_item_id}.{}", embedded.extension));
    write_cover_bytes_if_changed(&path, &embedded.bytes)
        .with_context(|| format!("writing embedded cover art to {}", path.display()))?;
    Ok(path)
}

pub fn find_folder_art(audio_path: &Path) -> Option<PathBuf> {
    let dir = audio_path.parent()?;

    for name in FOLDER_ART_NAMES {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    let stem = audio_path.file_stem()?;
    for extension in ["jpg", "jpeg", "png", "webp"] {
        let mut file_name = stem.to_os_string();
        file_name.push(".");
        file_name.push(extension);
        let candidate = dir.join(file_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}

pub fn folder_art_signature(audio_path: &Path) -> Result<Option<String>> {
    let Some(path) = find_folder_art(audio_path) else {
        return Ok(None);
    };
    let metadata = fs::metadata(&path)
        .with_context(|| format!("reading folder cover art metadata {}", path.display()))?;
    let stamp = media::file_stamp(&metadata);
    let mut hasher = Sha256::new();
    hasher.update(native_path_bytes(&path));
    hasher.update(stamp.file_size.to_be_bytes());
    hash_optional_i64(&mut hasher, stamp.modified_at_ns);
    hash_optional_i64(&mut hasher, stamp.fs_device);
    hash_optional_i64(&mut hasher, stamp.fs_inode);
    Ok(Some(hex::encode(hasher.finalize())))
}

fn hash_optional_i64(hasher: &mut Sha256, value: Option<i64>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            hasher.update(value.to_be_bytes());
        }
        None => hasher.update([0]),
    }
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

fn copy_cover_if_changed(source: &Path, destination: &Path) -> Result<()> {
    let bytes = fs::read(source)
        .with_context(|| format!("reading folder cover art {}", source.display()))?;
    write_cover_bytes_if_changed(destination, &bytes)
}

fn write_cover_bytes_if_changed(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Ok(existing) = fs::read(path) {
        if existing == bytes {
            return Ok(());
        }
    }

    fs::write(path, bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        copy_cover_if_changed, folder_art_signature, write_cover_bytes_if_changed, ArtworkMode,
    };

    #[test]
    fn artwork_mode_accepts_on_demand_aliases() {
        assert_eq!(ArtworkMode::from_value(None), ArtworkMode::OnDemand);
        assert_eq!(ArtworkMode::from_value(Some("cached")), ArtworkMode::Cached);
        assert_eq!(ArtworkMode::from_value(Some("cache")), ArtworkMode::Cached);
        assert_eq!(ArtworkMode::from_value(Some("eager")), ArtworkMode::Cached);
        assert_eq!(ArtworkMode::from_value(Some("scan")), ArtworkMode::Cached);
        assert_eq!(
            ArtworkMode::from_value(Some("ondemand")),
            ArtworkMode::OnDemand
        );
        assert_eq!(
            ArtworkMode::from_value(Some("on-demand")),
            ArtworkMode::OnDemand
        );
        assert_eq!(ArtworkMode::from_value(Some("lazy")), ArtworkMode::OnDemand);
        assert_eq!(
            ArtworkMode::from_value(Some(" file ")),
            ArtworkMode::OnDemand
        );
    }

    #[test]
    fn cover_cache_writes_only_when_content_differs() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("cover.jpg");
        let cached = dir.path().join("cached.jpg");

        std::fs::write(&source, b"first").unwrap();
        copy_cover_if_changed(&source, &cached).unwrap();
        assert_eq!(std::fs::read(&cached).unwrap(), b"first");

        write_cover_bytes_if_changed(&cached, b"first").unwrap();
        assert_eq!(std::fs::read(&cached).unwrap(), b"first");

        std::fs::write(&source, b"second").unwrap();
        copy_cover_if_changed(&source, &cached).unwrap();
        assert_eq!(std::fs::read(&cached).unwrap(), b"second");
    }

    #[test]
    fn on_demand_materialization_reuses_folder_art_path() {
        let dir = tempfile::tempdir().unwrap();
        let audio = dir.path().join("song.wav");
        let cover = dir.path().join("cover.jpg");
        let art_dir = dir.path().join("art-cache");
        write_test_wav(&audio);
        std::fs::write(&cover, b"folder art").unwrap();

        let path = super::materialize_cover_for_audio_path(&audio, &art_dir, 42).unwrap();

        assert_eq!(path.as_deref(), Some(cover.as_path()));
        assert!(!art_dir.exists());
    }

    #[test]
    fn folder_art_signature_changes_with_selected_artwork() {
        let dir = tempfile::tempdir().unwrap();
        let audio = dir.path().join("song.flac");
        let cover = dir.path().join("cover.jpg");
        std::fs::write(&audio, b"audio").unwrap();

        assert_eq!(folder_art_signature(&audio).unwrap(), None);

        std::fs::write(&cover, b"first").unwrap();
        let first = folder_art_signature(&audio).unwrap();

        std::fs::write(&cover, b"longer second").unwrap();
        let second = folder_art_signature(&audio).unwrap();

        assert!(first.is_some());
        assert!(second.is_some());
        assert_ne!(first, second);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn finds_same_stem_art_for_non_utf8_audio_name() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let dir = tempfile::tempdir().unwrap();
        let audio = dir
            .path()
            .join(OsString::from_vec(b"song-\xff.flac".to_vec()));
        let cover = dir
            .path()
            .join(OsString::from_vec(b"song-\xff.jpg".to_vec()));
        std::fs::write(&audio, b"audio").unwrap();
        std::fs::write(&cover, b"cover").unwrap();

        assert_eq!(super::find_folder_art(&audio), Some(cover));
    }

    fn write_test_wav(path: &std::path::Path) {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&38_u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&8_000_u32.to_le_bytes());
        bytes.extend_from_slice(&16_000_u32.to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        bytes.extend_from_slice(&0_i16.to_le_bytes());
        std::fs::write(path, bytes).unwrap();
    }
}
