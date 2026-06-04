use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

use anyhow::{bail, Context, Result};

use crate::integration::TrackSnapshot;

use super::notifier_helper;

const HELPER_APP_NAME: &str = "GMUS.app";
const LEGACY_HELPER_APP_NAME: &str = "GMUS Notifier.app";

#[derive(Debug)]
pub(super) struct MacNotifier {
    helper: NotifierHelperApp,
    visible: bool,
}

impl Default for MacNotifier {
    fn default() -> Self {
        Self::new(true)
    }
}

impl MacNotifier {
    pub(super) fn new(visible: bool) -> Self {
        Self {
            helper: NotifierHelperApp,
            visible,
        }
    }

    pub(super) fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    pub(super) fn notify_track_changed(&mut self, track: &TrackSnapshot) -> Result<()> {
        if !self.visible {
            return Ok(());
        }

        let payload = NotificationPayload::from_track(track);
        self.helper.show_notification(&payload)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NotificationPayload {
    title: String,
    subtitle: Option<String>,
    body: String,
    artwork_path: Option<PathBuf>,
}

impl NotificationPayload {
    fn from_track(track: &TrackSnapshot) -> Self {
        Self {
            title: clean_text(track.title.as_deref())
                .unwrap_or_else(|| String::from("Unknown Track")),
            subtitle: clean_text(track.artist.as_deref()),
            body: clean_text(track.album.as_deref()).unwrap_or_default(),
            artwork_path: track.artwork_path.clone(),
        }
    }
}

#[derive(Debug, Default)]
struct NotifierHelperApp;

impl NotifierHelperApp {
    fn show_notification(&mut self, payload: &NotificationPayload) -> Result<()> {
        let result = self.launch_notification(payload);
        if let Err(error) = &result {
            log_helper_launch_error(error);
        }
        result
    }

    fn launch_notification(&mut self, payload: &NotificationPayload) -> Result<()> {
        let app_dir = ensure_helper_bundle().context("preparing GMUS notification helper")?;
        let output = Command::new("/usr/bin/open")
            .arg("-n")
            .arg("-g")
            .arg("-j")
            .arg(&app_dir)
            .arg("--args")
            .args(notifier_helper::notification_args(
                &payload.title,
                payload.subtitle.as_deref(),
                &payload.body,
                payload.artwork_path.as_deref(),
            ))
            .output()
            .context("launching GMUS notification helper")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("GMUS notification helper launch failed: {}", stderr.trim());
        }

        Ok(())
    }
}

fn ensure_helper_bundle() -> Result<PathBuf> {
    let support_dir = notifier_helper::gmus_support_dir()?;
    remove_legacy_helper_bundle(&support_dir);

    let app_dir = support_dir.join(HELPER_APP_NAME);
    let contents_dir = app_dir.join("Contents");
    let macos_dir = contents_dir.join("MacOS");
    fs::create_dir_all(&macos_dir).with_context(|| format!("creating {}", macos_dir.display()))?;

    let info_changed = write_if_changed(&contents_dir.join("Info.plist"), helper_info_plist())?;
    let executable_changed = copy_current_executable(
        &macos_dir.join("gmus-notifier"),
        &contents_dir.join("GMUSExecutableSignature"),
    )?;
    let signature_missing = !contents_dir.join("_CodeSignature").exists();
    if info_changed || executable_changed || signature_missing {
        sign_helper_bundle(&app_dir)?;
    }

    Ok(app_dir)
}

fn remove_legacy_helper_bundle(support_dir: &Path) {
    let legacy_app_dir = support_dir.join(LEGACY_HELPER_APP_NAME);
    if legacy_app_dir.is_dir() {
        let _ = fs::remove_dir_all(legacy_app_dir);
    }
}

fn write_if_changed(path: &Path, content: &str) -> Result<bool> {
    if fs::read_to_string(path).is_ok_and(|existing| existing == content) {
        return Ok(false);
    }

    fs::write(path, content).with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

fn copy_current_executable(destination: &Path, signature_path: &Path) -> Result<bool> {
    let source = std::env::current_exe().context("locating the running GMUS executable")?;
    let source_signature = executable_signature(&source)?;
    if executable_is_current(destination, signature_path, &source_signature)? {
        return Ok(false);
    }

    fs::copy(&source, destination)
        .with_context(|| format!("copying {} to {}", source.display(), destination.display()))?;
    fs::set_permissions(destination, fs::Permissions::from_mode(0o755))
        .with_context(|| format!("marking {} executable", destination.display()))?;
    fs::write(signature_path, source_signature)
        .with_context(|| format!("writing {}", signature_path.display()))?;
    Ok(true)
}

fn executable_is_current(
    destination: &Path,
    signature_path: &Path,
    source_signature: &str,
) -> Result<bool> {
    if !destination.exists() {
        return Ok(false);
    }

    Ok(fs::read_to_string(signature_path).is_ok_and(|signature| signature == source_signature))
}

fn executable_signature(path: &Path) -> Result<String> {
    let metadata = fs::metadata(path).with_context(|| format!("reading {}", path.display()))?;
    let modified = metadata
        .modified()
        .unwrap_or(UNIX_EPOCH)
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Ok(format!(
        "{}\n{}\n{}.{}\n",
        path.display(),
        metadata.len(),
        modified.as_secs(),
        modified.subsec_nanos()
    ))
}

fn sign_helper_bundle(app_dir: &Path) -> Result<()> {
    let output = Command::new("/usr/bin/codesign")
        .arg("--force")
        .arg("--deep")
        .arg("--sign")
        .arg("-")
        .arg(app_dir)
        .output()
        .context("running codesign for GMUS notification helper")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "codesign failed for GMUS notification helper: {}",
            stderr.trim()
        );
    }

    Ok(())
}

fn helper_info_plist() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDisplayName</key>
    <string>GMUS</string>
    <key>CFBundleExecutable</key>
    <string>gmus-notifier</string>
    <key>CFBundleIdentifier</key>
    <string>io.github.claytongrey.gmus.notifier</string>
    <key>CFBundleName</key>
    <string>GMUS</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>LSUIElement</key>
    <true/>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
"#
}

fn log_helper_launch_error(error: &anyhow::Error) {
    let Ok(dir) = notifier_helper::gmus_support_dir() else {
        return;
    };
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(
        dir.join("notifier-launch-error.log"),
        format!("{error:#}\n"),
    );
}

fn clean_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::NotificationPayload;
    use crate::integration::TrackSnapshot;

    #[test]
    fn notification_payload_uses_track_metadata() {
        let payload = NotificationPayload::from_track(&TrackSnapshot {
            title: Some(String::from("Song")),
            artist: Some(String::from("Artist")),
            album: Some(String::from("Album")),
            duration_ms: Some(123_000),
            artwork_path: Some("/tmp/cover.jpg".into()),
        });

        assert_eq!(payload.title, "Song");
        assert_eq!(payload.subtitle.as_deref(), Some("Artist"));
        assert_eq!(payload.body, "Album");
        assert_eq!(
            payload.artwork_path.as_deref(),
            Some(std::path::Path::new("/tmp/cover.jpg"))
        );
    }

    #[test]
    fn notification_payload_falls_back_to_unknown_track_title() {
        let payload = NotificationPayload::from_track(&TrackSnapshot {
            title: Some(String::from("  ")),
            artist: Some(String::from("  Artist  ")),
            album: Some(String::from("  Album  ")),
            duration_ms: None,
            artwork_path: None,
        });

        assert_eq!(payload.title, "Unknown Track");
        assert_eq!(payload.subtitle.as_deref(), Some("Artist"));
        assert_eq!(payload.body, "Album");
    }

    #[test]
    fn helper_info_plist_hides_helper_app() {
        let plist = super::helper_info_plist();

        assert!(plist.contains("<key>LSUIElement</key>"));
        assert!(plist.contains("<true/>"));
        assert!(plist.contains("io.github.claytongrey.gmus.notifier"));
    }
}
