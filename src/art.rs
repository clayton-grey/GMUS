use std::fs;
use std::path::{Path, PathBuf};

use crate::media::{self, TrackMetadata};
use anyhow::{Context, Result};

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
    art_dir: &Path,
    media_item_id: i64,
) -> Result<Option<PathBuf>> {
    if let Some(embedded) = media::read_embedded_art(&track.path)? {
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

    let stem = audio_path.file_stem()?.to_str()?;
    for extension in ["jpg", "jpeg", "png", "webp"] {
        let candidate = dir.join(format!("{stem}.{extension}"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
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
    use super::{copy_cover_if_changed, write_cover_bytes_if_changed};

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
}
