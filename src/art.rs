use std::fs;
use std::path::{Path, PathBuf};

use crate::media::TrackMetadata;
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

pub fn cache_cover_for_track(track: &TrackMetadata, art_dir: &Path) -> Result<Option<PathBuf>> {
    if let Some(embedded) = &track.embedded_art {
        fs::create_dir_all(art_dir)
            .with_context(|| format!("creating art cache {}", art_dir.display()))?;
        let path = art_dir.join(format!("{}.{}", track.fingerprint(), embedded.extension));
        fs::write(&path, &embedded.bytes)
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
        let path = art_dir.join(format!("{}.{}", track.fingerprint(), extension));
        fs::copy(&folder_art, &path).with_context(|| {
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
