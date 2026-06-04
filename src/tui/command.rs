use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use rusqlite::Connection;
use unicode_width::UnicodeWidthStr;

#[cfg(all(target_os = "macos", feature = "macos-media-session"))]
use crate::integration::IntegrationEvent;
use crate::{db, library};

use super::{App, CommandOutputKind};

#[cfg(not(all(target_os = "macos", feature = "macos-media-session")))]
pub(super) const COMMAND_NAMES: &[&str] = &BASE_COMMAND_NAMES;

#[cfg(all(target_os = "macos", feature = "macos-media-session"))]
pub(super) const COMMAND_NAMES: &[&str] = &[
    "add",
    "remove",
    "update",
    "library",
    "playlist",
    "playlist-clear",
    "playlist-delete",
    "keymap",
    "keymap-reset",
    "restore-filter",
    "restore-track",
    "filter",
    "clear",
    "clear-output",
    "notifications",
];

#[cfg(not(all(target_os = "macos", feature = "macos-media-session")))]
const BASE_COMMAND_NAMES: [&str; 14] = [
    "add",
    "remove",
    "update",
    "library",
    "playlist",
    "playlist-clear",
    "playlist-delete",
    "keymap",
    "keymap-reset",
    "restore-filter",
    "restore-track",
    "filter",
    "clear",
    "clear-output",
];

struct CompletionResult {
    replacement: Option<String>,
    notice: Option<String>,
}

#[derive(Clone)]
struct CompletionCandidate {
    value: String,
    is_dir: bool,
}

fn complete_command_input(conn: &Connection, input: &str) -> Result<CompletionResult> {
    let Some((command, before_arg, arg)) = split_command_arg(input) else {
        return Ok(complete_command_name(input));
    };

    match command.to_ascii_lowercase().as_str() {
        "add" | "update" | "u" => Ok(complete_path_arg(
            before_arg,
            arg,
            filesystem_candidates(arg),
        )),
        "remove" | "rm" => Ok(complete_path_arg(
            before_arg,
            arg,
            library_root_candidates(conn, arg)?,
        )),
        _ => Ok(CompletionResult {
            replacement: None,
            notice: Some(format!("{command} does not take path completion")),
        }),
    }
}

fn split_command_arg(input: &str) -> Option<(&str, &str, &str)> {
    let trimmed = input.trim_start();
    let leading_width = input.len() - trimmed.len();
    let command_width = trimmed.find(char::is_whitespace)?;
    let command = &trimmed[..command_width];
    let after_command = leading_width + command_width;
    let arg_start = input[after_command..]
        .find(|character: char| !character.is_whitespace())
        .map(|offset| after_command + offset)
        .unwrap_or(input.len());

    Some((command, &input[..arg_start], &input[arg_start..]))
}

fn complete_command_name(input: &str) -> CompletionResult {
    let prefix = input.trim_start();
    let leading = &input[..input.len() - prefix.len()];
    let matches: Vec<String> = COMMAND_NAMES
        .iter()
        .filter(|command| command.starts_with(prefix))
        .map(|command| (*command).to_string())
        .collect();

    complete_text(leading, prefix, matches, true)
}

fn complete_path_arg(
    before_arg: &str,
    arg: &str,
    candidates: Vec<CompletionCandidate>,
) -> CompletionResult {
    if candidates.is_empty() {
        return CompletionResult {
            replacement: None,
            notice: Some(String::from("no completion matches")),
        };
    }

    if candidates.len() == 1 {
        let candidate = &candidates[0];
        let suffix = if candidate.is_dir { "/" } else { " " };
        return CompletionResult {
            replacement: Some(format!("{before_arg}{}{suffix}", candidate.value)),
            notice: None,
        };
    }

    let values: Vec<String> = candidates
        .iter()
        .map(|candidate| {
            if candidate.is_dir {
                format!("{}/", candidate.value)
            } else {
                candidate.value.clone()
            }
        })
        .collect();
    let common = common_prefix(&values);
    let replacement =
        (display_width(&common) > display_width(arg)).then(|| format!("{before_arg}{common}"));

    CompletionResult {
        replacement,
        notice: Some(matches_notice(&values)),
    }
}

fn complete_text(
    leading: &str,
    prefix: &str,
    matches: Vec<String>,
    append_space_on_unique: bool,
) -> CompletionResult {
    if matches.is_empty() {
        return CompletionResult {
            replacement: None,
            notice: Some(String::from("no completion matches")),
        };
    }

    if matches.len() == 1 {
        let suffix = if append_space_on_unique { " " } else { "" };
        return CompletionResult {
            replacement: Some(format!("{leading}{}{suffix}", matches[0])),
            notice: None,
        };
    }

    let common = common_prefix(&matches);
    CompletionResult {
        replacement: (display_width(&common) > display_width(prefix))
            .then(|| format!("{leading}{common}")),
        notice: Some(matches_notice(&matches)),
    }
}

fn filesystem_candidates(arg: &str) -> Vec<CompletionCandidate> {
    if arg == "~" {
        return vec![CompletionCandidate {
            value: String::from("~"),
            is_dir: true,
        }];
    }

    let lookup_arg = unquote_command_arg(arg);
    let lookup_path = expand_command_path(lookup_arg);
    let trailing_separator = lookup_arg.ends_with('/');
    let directory = if trailing_separator {
        lookup_path.clone()
    } else {
        lookup_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    };
    let name_prefix = if trailing_separator {
        String::new()
    } else {
        lookup_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string()
    };
    let display_prefix = display_path_prefix(lookup_arg, trailing_separator);

    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };

    let mut candidates = Vec::new();
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if !file_name.starts_with(&name_prefix) {
            continue;
        }
        if !name_prefix.starts_with('.') && file_name.starts_with('.') {
            continue;
        }
        let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
        candidates.push(CompletionCandidate {
            value: format!("{display_prefix}{file_name}"),
            is_dir,
        });
    }
    sort_candidates(candidates)
}

fn library_root_candidates(conn: &Connection, arg: &str) -> Result<Vec<CompletionCandidate>> {
    let lookup_arg = unquote_command_arg(arg);
    let expanded = expand_command_path(lookup_arg);
    let prefix = expanded.to_string_lossy();
    let mut candidates: Vec<CompletionCandidate> = db::active_library_roots(conn)?
        .into_iter()
        .filter(|root| root.path.starts_with(prefix.as_ref()))
        .map(|root| CompletionCandidate {
            value: root.path,
            is_dir: false,
        })
        .collect();

    if candidates.is_empty() {
        candidates = filesystem_candidates(arg);
    }
    Ok(sort_candidates(candidates))
}

fn sort_candidates(mut candidates: Vec<CompletionCandidate>) -> Vec<CompletionCandidate> {
    candidates.sort_by(|left, right| {
        left.value
            .to_ascii_lowercase()
            .cmp(&right.value.to_ascii_lowercase())
    });
    candidates
}

fn display_path_prefix(raw_path: &str, trailing_separator: bool) -> String {
    if trailing_separator {
        return raw_path.to_string();
    }

    raw_path
        .rfind('/')
        .map(|position| raw_path[..=position].to_string())
        .unwrap_or_default()
}

fn expand_command_path(raw_path: &str) -> PathBuf {
    if raw_path == "~" {
        return env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    }

    if let Some(rest) = raw_path.strip_prefix("~/") {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }

    PathBuf::from(raw_path)
}

fn common_prefix(values: &[String]) -> String {
    let Some(first) = values.first() else {
        return String::new();
    };
    let mut prefix = first.clone();
    for value in values.iter().skip(1) {
        while !value.starts_with(&prefix) {
            if prefix.pop().is_none() {
                return String::new();
            }
        }
    }
    prefix
}

fn matches_notice(matches: &[String]) -> String {
    let shown = matches
        .iter()
        .take(5)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("  ");
    if matches.len() > 5 {
        format!("matches: {shown}  ...")
    } else {
        format!("matches: {shown}")
    }
}

fn toggle_command_value(raw_value: &str, current: bool) -> Option<bool> {
    match raw_value.trim().to_ascii_lowercase().as_str() {
        "" | "toggle" => Some(!current),
        "status" => Some(current),
        "on" | "yes" | "true" | "show" | "enable" | "enabled" => Some(true),
        "off" | "no" | "false" | "hide" | "disable" | "disabled" => Some(false),
        _ => None,
    }
}

fn on_off(enabled: bool) -> &'static str {
    if enabled {
        "on"
    } else {
        "off"
    }
}

fn library_job_for_command(
    input: &str,
) -> Option<std::result::Result<library::LibraryJob, String>> {
    let input = input.strip_prefix(':').unwrap_or(input).trim();
    let mut parts = input.splitn(2, char::is_whitespace);
    let command = parts.next()?.to_ascii_lowercase();
    let rest = parts.next().unwrap_or_default().trim();

    match command.as_str() {
        "add" => Some(
            command_path(rest)
                .map(library::LibraryJob::AddRoot)
                .ok_or_else(|| String::from("usage: :add PATH")),
        ),
        "update" | "u" => Some(Ok(command_path(rest)
            .map(library::LibraryJob::UpdateRoot)
            .unwrap_or(library::LibraryJob::UpdateAllRoots))),
        _ => None,
    }
}

fn display_command(input: &str) -> String {
    let command = input.trim();
    if command.starts_with(':') {
        command.to_string()
    } else {
        format!(":{command}")
    }
}

fn command_path(raw_path: &str) -> Option<PathBuf> {
    let raw_path = unquote_command_arg(raw_path.trim());
    if raw_path.is_empty() {
        return None;
    }

    if raw_path == "~" {
        return env::var_os("HOME").map(PathBuf::from);
    }

    if let Some(rest) = raw_path.strip_prefix("~/") {
        if let Some(home) = env::var_os("HOME") {
            return Some(PathBuf::from(home).join(rest));
        }
    }

    Some(PathBuf::from(raw_path))
}

fn unquote_command_arg(value: &str) -> &str {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return &value[1..value.len() - 1];
        }
    }
    if value.len() >= 2 {
        let first = value.as_bytes()[0];
        if first == b'"' || first == b'\'' {
            return &value[1..];
        }
    }
    value
}

fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

impl App {
    pub(super) fn show_command_output(&mut self, lines: Vec<String>) {
        self.command_output = lines;
        self.command_output_kind = CommandOutputKind::Text;
        self.command_roots.clear();
        self.command_selected = 0;
        self.command_focus = false;
    }

    fn show_library_roots(&mut self, roots: Vec<db::LibraryRoot>, selected_path: Option<&str>) {
        let active_count = roots.iter().filter(|root| root.active).count();
        let mut output = vec![format!(
            "library roots ({active_count} active / {} total)",
            roots.len()
        )];
        output.extend(
            roots
                .iter()
                .map(|root| format!("{} {}", if root.active { "[x]" } else { "[ ]" }, root.path)),
        );

        self.command_selected = selected_path
            .and_then(|path| roots.iter().position(|root| root.path == path))
            .unwrap_or(0)
            .min(roots.len().saturating_sub(1));
        self.command_focus = !roots.is_empty();
        self.command_output_kind = CommandOutputKind::LibraryRoots;
        self.command_roots = roots;
        self.command_output = output;
    }

    pub(super) fn clear_command_output(&mut self) -> bool {
        if self.command_output.is_empty() && self.command_roots.is_empty() && !self.command_focus {
            false
        } else {
            self.command_output.clear();
            self.command_output_kind = CommandOutputKind::Text;
            self.command_roots.clear();
            self.command_selected = 0;
            self.command_focus = false;
            true
        }
    }

    #[cfg(test)]
    pub(super) fn execute_command(&mut self, conn: &Connection) {
        self.command_mode = false;
        let command = std::mem::take(&mut self.command);
        let result = self.run_command(conn, command.trim());
        self.finish_command_result(result);
    }

    pub(super) fn submit_command(&mut self, conn: &Connection) {
        self.command_mode = false;
        let command = std::mem::take(&mut self.command);
        if let Some(job) = library_job_for_command(command.trim()) {
            match job {
                Ok(job) => self.start_library_job(command, job),
                Err(message) => self.finish_command_result(Ok(message)),
            }
        } else {
            let result = self.run_command(conn, command.trim());
            self.finish_command_result(result);
        }
    }

    fn start_library_job(&mut self, command: String, job: library::LibraryJob) {
        if let Some(active_job) = &self.library_job {
            self.finish_command_result(Ok(format!(
                "scan already running: {}",
                active_job.command()
            )));
            return;
        }

        let command = display_command(&command);
        self.library_job = Some(super::LibraryJobRunner::spawn(
            command.clone(),
            self.paths.clone(),
            job,
        ));
        self.show_command_output(vec![
            format!("working: {command}"),
            String::from("scanning files recursively..."),
        ]);
        self.message = format!("working: {command}");
        self.show_transient_status(self.message.clone());
    }

    pub(super) fn poll_library_job(&mut self, conn: &Connection) -> Result<bool> {
        let result = match self.library_job.as_ref() {
            Some(job) => match job.try_finish() {
                Ok(Some(result)) => result,
                Ok(None) => return Ok(false),
                Err(error) => Err(error),
            },
            None => return Ok(false),
        };
        self.library_job = None;

        let result = result.and_then(|job_result| {
            if job_result.refreshes_library() {
                self.refresh(conn)?;
            }
            Ok(library::job_status(&job_result))
        });
        self.finish_command_result(result);
        Ok(true)
    }

    fn finish_command_result(&mut self, result: Result<String>) {
        self.message = match result {
            Ok(message) => message,
            Err(error) => format!("command failed: {error:#}"),
        };
        self.show_transient_status(self.message.clone());
    }

    pub(super) fn complete_command(&mut self, conn: &Connection) -> Result<()> {
        let result = complete_command_input(conn, &self.command)?;
        if let Some(replacement) = result.replacement {
            self.command = replacement;
        }
        if let Some(notice) = result.notice {
            self.message = notice;
            self.show_transient_status(self.message.clone());
        }
        Ok(())
    }

    fn run_command(&mut self, conn: &Connection, input: &str) -> Result<String> {
        let input = input.strip_prefix(':').unwrap_or(input).trim();
        if input.is_empty() {
            return Ok(String::from("empty command"));
        }

        let mut parts = input.splitn(2, char::is_whitespace);
        let command = parts.next().unwrap_or_default().to_ascii_lowercase();
        let rest = parts.next().unwrap_or_default().trim();

        match command.as_str() {
            "add" => {
                self.clear_command_output();
                self.command_add(conn, rest)
            }
            "remove" | "rm" => {
                self.clear_command_output();
                self.command_remove(conn, rest)
            }
            "update" | "u" => {
                self.clear_command_output();
                self.command_update(conn, rest)
            }
            "library" | "roots" => self.command_library(conn),
            "playlist" | "pl" => self.command_playlist(conn, rest),
            "playlist-clear" | "pl-clear" => self.command_playlist_clear(conn, rest),
            "playlist-delete" | "pl-delete" | "playlist-rm" | "pl-rm" => {
                self.command_playlist_delete(conn, rest)
            }
            "keymap" | "keys" => {
                self.clear_command_output();
                self.toggle_keymap_panel();
                Ok(String::from("keymap panel"))
            }
            "keymap-reset" | "keys-reset" => {
                self.clear_command_output();
                self.reset_key_bindings(conn)?;
                Ok(String::from("keymap reset to defaults"))
            }
            "restore-filter" => self.command_restore_filter(conn, rest),
            "restore-track" => self.command_restore_track(conn, rest),
            "filter" | "f" => {
                self.clear_command_output();
                self.filter = rest.to_string();
                self.confirm_filter(conn)?;
                Ok(format!("filter: {}", self.filter_display()))
            }
            "clear" | "clear-filter" => {
                self.clear_command_output();
                self.clear_filter(conn)?;
                Ok(String::from("filter cleared"))
            }
            "clear-output" | "close" | "hide" => {
                if self.clear_command_output() {
                    Ok(String::from("output cleared"))
                } else {
                    Ok(String::from("no output to clear"))
                }
            }
            #[cfg(all(target_os = "macos", feature = "macos-media-session"))]
            "notifications" | "notify" => self.command_notifications(rest),
            _ => Ok(format!("unknown command: {command}")),
        }
    }

    #[cfg(all(target_os = "macos", feature = "macos-media-session"))]
    fn command_notifications(&mut self, raw_value: &str) -> Result<String> {
        let value = raw_value.trim().to_ascii_lowercase();
        let visible = match value.as_str() {
            "" | "status" => return Ok(self.notification_status_message()),
            "on" | "yes" | "true" | "show" | "visible" => true,
            "off" | "no" | "false" | "hide" | "hidden" => false,
            "toggle" => !self.track_notifications_visible,
            _ => return Ok(String::from("usage: :notifications [on|off|toggle|status]")),
        };

        self.track_notifications_visible = visible;
        self.integration
            .publish_event(&IntegrationEvent::TrackNotificationsVisible(visible))?;
        Ok(self.notification_status_message())
    }

    #[cfg(all(target_os = "macos", feature = "macos-media-session"))]
    fn notification_status_message(&self) -> String {
        format!(
            "track notifications {}",
            if self.track_notifications_visible {
                "visible"
            } else {
                "hidden"
            }
        )
    }

    fn command_restore_filter(&mut self, conn: &Connection, raw_value: &str) -> Result<String> {
        let Some(enabled) = toggle_command_value(raw_value, self.restore_filter) else {
            return Ok(String::from(
                "usage: :restore-filter [on|off|toggle|status]",
            ));
        };
        if raw_value.trim().eq_ignore_ascii_case("status") {
            return Ok(format!("restore filter {}", on_off(self.restore_filter)));
        }

        self.restore_filter = enabled;
        db::save_restore_filter_enabled(conn, enabled)?;
        if enabled {
            self.save_filter_state(conn)?;
        }
        Ok(format!("restore filter {}", on_off(enabled)))
    }

    fn command_restore_track(&mut self, conn: &Connection, raw_value: &str) -> Result<String> {
        let Some(enabled) = toggle_command_value(raw_value, self.restore_track) else {
            return Ok(String::from("usage: :restore-track [on|off|toggle|status]"));
        };
        if raw_value.trim().eq_ignore_ascii_case("status") {
            return Ok(format!("restore track {}", on_off(self.restore_track)));
        }

        self.restore_track = enabled;
        db::save_restore_track_enabled(conn, enabled)?;
        if enabled {
            self.save_current_track_selection(conn)?;
            self.select_current_track_for_restore();
        }
        Ok(format!("restore track {}", on_off(enabled)))
    }

    fn command_add(&mut self, conn: &Connection, raw_path: &str) -> Result<String> {
        let Some(path) = command_path(raw_path) else {
            return Ok(String::from("usage: :add PATH"));
        };
        let result = library::add_root(conn, &self.paths, &path)?;
        if result.refreshes_library() {
            self.refresh(conn)?;
        }
        Ok(library::job_status(&result))
    }

    fn command_remove(&mut self, conn: &Connection, raw_path: &str) -> Result<String> {
        let Some(path) = command_path(raw_path) else {
            return Ok(String::from("usage: :remove PATH"));
        };
        let root = path.canonicalize().unwrap_or(path);
        if db::deactivate_library_root(conn, &root)? {
            self.refresh(conn)?;
            Ok(format!("removed {} from library", root.display()))
        } else {
            Ok(format!("no library root: {}", root.display()))
        }
    }

    fn command_update(&mut self, conn: &Connection, raw_path: &str) -> Result<String> {
        let result = if let Some(path) = command_path(raw_path) {
            library::update_root(conn, &self.paths, &path)?
        } else {
            library::update_all_roots(conn, &self.paths)?
        };
        if result.refreshes_library() {
            self.refresh(conn)?;
        }
        Ok(library::job_status(&result))
    }

    fn command_library(&mut self, conn: &Connection) -> Result<String> {
        let roots = db::library_roots(conn)?;
        if roots.is_empty() {
            self.show_command_output(vec![
                String::from("library roots"),
                String::from("<legacy all scanned tracks>"),
            ]);
            return Ok(String::from("library roots: <legacy all scanned tracks>"));
        }

        self.show_library_roots(roots, None);

        let active: Vec<&str> = self
            .command_roots
            .iter()
            .filter(|root| root.active)
            .map(|root| root.path.as_str())
            .collect();
        if active.is_empty() {
            Ok(String::from("library roots: <none active>"))
        } else {
            Ok(format!("library roots: {}", active.join("; ")))
        }
    }

    pub(super) fn toggle_selected_library_root(&mut self, conn: &Connection) -> Result<()> {
        if self.command_output_kind != CommandOutputKind::LibraryRoots {
            return Ok(());
        }
        let Some(root) = self.command_roots.get(self.command_selected).cloned() else {
            self.message = String::from("no library root selected");
            return Ok(());
        };

        let next_active = !root.active;
        if db::set_library_root_active(conn, Path::new(&root.path), next_active)? {
            self.refresh(conn)?;
            let roots = db::library_roots(conn)?;
            self.show_library_roots(roots, Some(&root.path));
            self.message = format!(
                "{} {}",
                if next_active { "enabled" } else { "disabled" },
                root.path
            );
            self.show_transient_status(self.message.clone());
        } else {
            self.message = format!("no library root: {}", root.path);
        }
        Ok(())
    }
}
