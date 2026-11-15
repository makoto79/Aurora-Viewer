//! The capture's studio light: three lights around the avatar and a grey
//! ambient, nothing from the region's sky, so a series has true colours
//! and looks the same in every region and at every hour. The layout is the
//! avatar studio's three-point light (`App::studio_render`, key, fill and
//! rim), in white: its bluish fill would tint the sheet. Not in the fork,
//! which had only its noon sun (Aurora addition).

use super::measure::Metrics;
use super::plan::Upright;
use glam::{Quat, Vec3};

/// The avatar studio's stage is lit for an avatar this tall.
pub const REFERENCE_HEIGHT: f32 = 1.8;
/// Key front right, fill front left, rim behind: metres from the floor
/// under the avatar for a REFERENCE_HEIGHT avatar, +X its facing, +Y its
/// left; linear colours times intensity.
pub const LIGHTS: [(Vec3, Vec3); 3] = [
    (Vec3::new(2.8, -2.5, 3.5), Vec3::splat(2.8)),
    (Vec3::new(1.5, 3.0, 2.8), Vec3::splat(1.3)),
    (Vec3::new(-2.0, 0.5, 3.5), Vec3::splat(1.8)),
];
/// The lights' reach at reference height (the avatar studio's).
pub const RADIUS: f32 = 12.0;
pub const FALLOFF: f32 = 0.5;
/// The grey ambient instead of the sky's: above, at the horizon, below.
pub const AMBIENT_ZENITH: Vec3 = Vec3::splat(0.18);
pub const AMBIENT_HORIZON: Vec3 = Vec3::splat(0.15);
pub const AMBIENT_GROUND: Vec3 = Vec3::splat(0.08);

/// Where the lights stand: the floor under the avatar, its facing (and a
/// seat's tilt), its height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rig {
    pub floor: Vec3,
    pub rotation: Quat,
    pub height: f32,
}

impl Rig {
    /// From the metrics measured in the upright frame (their `forward`
    /// still the upright one), back in the world with the frame's tilt.
    pub fn new(m: &Metrics, upright: &Upright) -> Self {
        let floor = Vec3::new(m.pelvis.x, m.pelvis.y, m.sole_z);
        let yaw = Quat::from_rotation_z(m.forward.y.atan2(m.forward.x));
        Self {
            floor: upright.pivot + upright.tilt * (floor - upright.pivot),
            rotation: (upright.tilt * yaw).normalize(),
            height: m.height,
        }
    }

    fn scale(&self) -> f32 {
        if self.height.is_finite() && self.height > 0.0 {
            (self.height / REFERENCE_HEIGHT).clamp(0.25, 4.0)
        } else {
            1.0
        }
    }

    /// The three lights in the world: position, colour, radius.
    pub fn lights(&self) -> [(Vec3, Vec3, f32); 3] {
        let s = self.scale();
        LIGHTS.map(|(at, color)| (self.floor + self.rotation * (at * s), color, RADIUS * s))
    }
}
