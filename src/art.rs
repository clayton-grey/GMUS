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

pub fn cache_cover_for_track(
    track: &TrackMetadata,
    embedded_art: Option<&EmbeddedArt>,
    art_dir: &Path,
    media_item_id: i64,
) -> Result<Option<PathBuf>> {
    if let Some(embedded) = embedded_art {
        fs::create_dir_all(art_dir)
            .with_context(|| format!("creating art cache {}", art_dir.display()))?;
        let path = art_dir.join(format!("{media_item_id}.{}", embedded.extension));
        write_cover_bytes_if_changed(&path, &embedded.bytes)
            .with_context(|| format!("writing embedded cover art to {}", path.display()))?;
        return Ok(Some(path));
    }

    if let Some(folder_art) = find_folder_art(&track.path) {
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
    use super::{copy_cover_if_changed, folder_art_signature, write_cover_bytes_if_changed};

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
}
