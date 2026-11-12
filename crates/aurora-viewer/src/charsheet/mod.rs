//! Character sheet capture: a fixed series of 20 images of one avatar (body
//! and head turnarounds, hands, feet) on a plain or transparent background,
//! for artists and image tools. Pure logic here (settings, measurement, shot
//! plan, A-pose, names, output, the run's steps); the app drives the camera
//! and the photo jobs (`app/charsheet.rs`), the window is `ui/charsheet.rs`.
//!
//! Port of LLFloaterAvatarSheet (indra/newview/llfloateravatarsheet.cpp) and
//! LLAvatarCaptureSession (indra/newview/llavatarcapture.cpp) of the human's
//! own Legacy Viewer fork, originally LGPL 2.1. Custom feature of this build.

pub mod measure;
pub mod naming;
pub mod output;
pub mod plan;
pub mod pose;
pub mod run;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The image size presets, in the fork's order (FSAvatarSheetResolution).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resolution {
    #[default]
    R1024,
    R1536,
    R2048,
    R768,
}

impl Resolution {
    pub const ALL: [Resolution; 4] = [Resolution::R1024, Resolution::R1536, Resolution::R2048, Resolution::R768];

    pub fn side(self) -> u32 {
        match self {
            Resolution::R1024 => 1024,
            Resolution::R1536 => 1536,
            Resolution::R2048 => 2048,
            Resolution::R768 => 768,
        }
    }
}

/// The window's choices, saved with the other settings (FSAvatarSheet*).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Put the avatar into the A-pose, locally (FSAvatarSheetApplyPose).
    pub a_pose: bool,
    /// Even daylight instead of the sky (FSAvatarSheetNeutralLight).
    pub neutral_light: bool,
    /// Backdrop colour, sRGB 0..1 (FSAvatarSheetBackground).
    pub background: [f32; 3],
    /// PNGs with alpha instead of the backdrop (FSAvatarSheetTransparent).
    pub transparent: bool,
    /// Above 100 the camera comes closer (FSAvatarSheetZoom).
    pub zoom_percent: f32,
    /// Vertical shift of the body framing, metres (FSAvatarSheetHeightOffset).
    pub height_offset_m: f32,
    /// Base folder; None: the photo folder's "Character sheets".
    pub folder: Option<PathBuf>,
    pub resolution: Resolution,
    /// Body shots in portrait format (FSAvatarSheetPortraitBody).
    pub portrait_body: bool,
    /// One extra grid image of all shots (FSAvatarSheetContactSheet).
    pub contact_sheet: bool,
    /// Countdown before the first image, seconds (FSAvatarSheetDelay).
    pub delay_s: u32,
    /// "High quality": each image drawn at twice its size and reduced.
    /// Not in the fork (Aurora addition, off by default).
    pub supersample: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            a_pose: true,
            neutral_light: true,
            background: [0.5, 0.5, 0.5],
            transparent: false,
            zoom_percent: 100.0,
            height_offset_m: 0.0,
            folder: None,
            resolution: Resolution::R1024,
            portrait_body: false,
            contact_sheet: true,
            delay_s: 3,
            supersample: false,
        }
    }
}

pub const ZOOM_RANGE: std::ops::RangeInclusive<f32> = 50.0..=200.0;
pub const HEIGHT_OFFSET_RANGE: std::ops::RangeInclusive<f32> = -1.0..=1.0;
pub const DELAY_MAX_S: u32 = 60;

impl Settings {
    /// The settings file is not trusted: every value back within its range.
    pub fn sanitized(mut self) -> Self {
        let finite = |v: f32, d: f32| if v.is_finite() { v } else { d };
        self.zoom_percent = finite(self.zoom_percent, 100.0).clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end());
        self.height_offset_m = finite(self.height_offset_m, 0.0).clamp(*HEIGHT_OFFSET_RANGE.start(), *HEIGHT_OFFSET_RANGE.end());
        self.background = self.background.map(|c| finite(c, 0.5).clamp(0.0, 1.0));
        self.delay_s = self.delay_s.min(DELAY_MAX_S);
        if self.folder.as_ref().is_some_and(|f| f.as_os_str().is_empty()) {
            self.folder = None;
        }
        self
    }

    /// The size of a square shot.
    pub fn square(&self) -> (u32, u32) {
        let s = self.resolution.side();
        (s, s)
    }

    /// The size of a portrait body shot: 3:4 wide, 21:16 high of the
    /// preset (768 x 1344 at 1024), as the fork's getPortraitBodySize.
    pub fn portrait(&self) -> (u32, u32) {
        let s = self.resolution.side();
        (s * 3 / 4, s * 21 / 16)
    }
}

/// The default base folder: the photo window's save folder, then
/// "Character sheets" (decision A6 of the plan).
pub fn default_folder(photo_folder: &std::path::Path) -> PathBuf {
    photo_folder.join("Character sheets")
}
