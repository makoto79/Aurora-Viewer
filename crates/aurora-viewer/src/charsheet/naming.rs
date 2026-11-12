//! File and folder names of a series. Port of
//! LLFloaterAvatarSheet::prepareSeriesDir, getFileTokens, shotFileName and
//! LLAvatarCaptureSession::sanitizeForPath (the fork, originally LGPL 2.1).
//! The tokens end up in prompts for image models: fixed English whatever
//! the viewer's language. Directions name where the camera stands, left and
//! right being the avatar's.

use super::plan::{Group, Shot, Side, View};
use std::path::{Path, PathBuf};

/// The compass directions of `Shot::direction`, as file tokens.
const DIRECTIONS: [&str; 8] = [
    "front",
    "front-left",
    "left",
    "back-left",
    "back",
    "back-right",
    "right",
    "front-right",
];

/// The i18n keys of the same directions, for the window.
pub const DIRECTION_KEYS: [&str; 8] = [
    "charsheet-dir-front",
    "charsheet-dir-front-left",
    "charsheet-dir-left",
    "charsheet-dir-back-left",
    "charsheet-dir-back",
    "charsheet-dir-back-right",
    "charsheet-dir-right",
    "charsheet-dir-front-right",
];

/// Characters no file system takes, as the fork's sanitizeForPath.
fn sanitize_for_path(text: &str) -> String {
    text.chars()
        .map(|c| if "\\/:*?\"<>|".contains(c) || c.is_control() { '_' } else { c })
        .collect::<String>()
        .trim()
        .to_owned()
}

/// The files' prefix: the account name ("jane.doe", unique and plain
/// ASCII; display names are neither), spaces as underscores, "avatar" when
/// unknown. A leading or trailing dot is dropped (Windows refuses a folder
/// ending in one).
pub fn prefix(username: &str) -> String {
    let p = sanitize_for_path(username).replace(' ', "_");
    let p = p.trim_matches('.').to_owned();
    if p.is_empty() { "avatar".to_owned() } else { p }
}

/// Group and view tokens of a shot (getFileTokens).
pub fn tokens(shot: &Shot) -> (String, String) {
    let side = match shot.side {
        Some(Side::Right) => "right",
        _ => "left",
    };
    match shot.group {
        Group::Hands => (
            format!("hand-{side}"),
            if shot.view == View::HandPalm { "palm" } else { "back" }.to_owned(),
        ),
        Group::Feet if shot.side.is_some() => (format!("foot-{side}"), "outer".to_owned()),
        Group::Feet => (
            "feet".to_owned(),
            if shot.view == View::Bottom { "bottom" } else { "front" }.to_owned(),
        ),
        Group::Body | Group::Head => (
            if shot.group == Group::Body { "body" } else { "head" }.to_owned(),
            DIRECTIONS[shot.direction()].to_owned(),
        ),
    }
}

/// `<prefix>_<NN>_<group>_<view>.png`: the number leads, so every file
/// browser lists the series in the plan's order.
pub fn file_name(prefix: &str, index: usize, shot: &Shot) -> String {
    let (group, view) = tokens(shot);
    format!("{prefix}_{:02}_{group}_{view}.png", index + 1)
}

pub fn contact_sheet_name(prefix: &str) -> String {
    format!("{prefix}_sheet.png")
}

pub const MANIFEST_NAME: &str = "sheet_manifest.json";

/// The series' own folder in `base`: `<prefix>_<YYYY-MM-DD_HHMM>`, then
/// `_2`, `_3`… when two runs fall in the same minute.
pub fn series_folder(base: &Path, prefix: &str, stamp: &str, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let name = format!("{prefix}_{stamp}");
    let first = base.join(&name);
    if !exists(&first) {
        return first;
    }
    (2u32..)
        .map(|n| base.join(format!("{name}_{n}")))
        .find(|p| !exists(p))
        .unwrap_or(first)
}

/// The local time as the fork's strftime("%Y-%m-%d_%H%M").
pub fn local_stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d_%H%M").to_string()
}

/// The i18n key of a shot's group, for the window's label.
pub fn group_key(group: Group) -> &'static str {
    match group {
        Group::Body => "charsheet-group-body",
        Group::Head => "charsheet-group-head",
        Group::Hands => "charsheet-group-hands",
        Group::Feet => "charsheet-group-feet",
    }
}
