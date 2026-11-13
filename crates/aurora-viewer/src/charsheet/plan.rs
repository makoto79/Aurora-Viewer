//! The 20 shots of a series, built once from one measurement. Port of
//! LLFloaterAvatarSheet::buildShotList, addTurnaround, addHandShots,
//! addFeetShots and LLAvatarCaptureSession::distanceForFrameHeight,
//! cameraOffsetFor, makeOrbitShot, makeDirectedShot (the fork's
//! llfloateravatarsheet.cpp and llavatarcapture.cpp, originally LGPL 2.1).
//! Positions are in the frame they were measured in (region render space);
//! a positive azimuth is the avatar's left.

use super::Settings;
use super::measure::Metrics;
use glam::{Quat, Vec3};

pub const IMAGE_COUNT: usize = 20;

/// Close to orthographic: a character sheet reads best that way.
pub const FOV_BODY_DEG: f32 = 20.0;
pub const FOV_DETAIL_DEG: f32 = 15.0;
/// Hands are framed small: a narrow angle would put the camera far off.
pub const FOV_HAND_DEG: f32 = 35.0;
pub const FRAME_BODY: f32 = 1.15;
pub const FRAME_HEAD: f32 = 0.45;
pub const FRAME_HAND: f32 = 0.30;
pub const FRAME_FOOT: f32 = 0.40;
pub const FRAMING_MARGIN_PERCENT: f32 = 10.0;
/// The body frame a little above the middle: the feet need less room.
pub const BODY_CENTER_LIFT: f32 = 0.03;
pub const HEAD_ELEVATION: f32 = 5.0;
pub const FEET_PAIR_PADDING: f32 = 0.35;
pub const FEET_FRONT_ELEVATION: f32 = 20.0;
pub const FEET_SIDE_ELEVATION: f32 = 10.0;
/// A standing subject's width over its height, for the horizontal fit.
pub const SUBJECT_WIDTH_RATIO: f32 = 0.75;
/// The portrait frame's: a wider ratio would push the camera back and undo
/// the point of a portrait frame.
pub const PORTRAIT_SUBJECT_WIDTH_RATIO: f32 = 0.5;
pub const MIN_DISTANCE: f32 = 0.25;
pub const TURNAROUND_AZIMUTHS: [f32; 6] = [0.0, 45.0, 90.0, 180.0, -90.0, -45.0];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    Body,
    Head,
    Hands,
    Feet,
}

impl Group {
    pub const ALL: [Group; 4] = [Group::Body, Group::Head, Group::Hands, Group::Feet];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    /// Around the avatar, at the shot's azimuth.
    Orbit,
    HandBack,
    HandPalm,
    /// The soles from straight below.
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shot {
    pub group: Group,
    pub side: Option<Side>,
    pub view: View,
    pub azimuth_deg: f32,
    pub elevation_deg: f32,
    pub target: Vec3,
    pub eye: Vec3,
    /// The image's up.
    pub up: Vec3,
    pub distance: f32,
    /// Vertical field of view.
    pub fov: f32,
    pub size: (u32, u32),
}

/// How far the camera stands for `frame_height` to fill the image's height
/// with the margin, and the subject's width its width.
pub fn distance_for_frame_height(frame_height: f32, fov: f32, margin_percent: f32, aspect: f32, width_ratio: f32) -> f32 {
    let margin = 1.0 + margin_percent.max(0.0) * 0.01;
    let tan_half = (fov * 0.5).tan().max(0.0001);
    let distance = frame_height * 0.5 * margin / tan_half;
    // a portrait image has a narrower horizontal angle than its vertical one
    let half_width = frame_height * width_ratio * 0.5 * margin;
    let distance_h = half_width / (tan_half * aspect.max(0.1));
    distance.max(distance_h).max(MIN_DISTANCE)
}

/// The camera around `target`: `forward` turned by the azimuth about Z,
/// raised by the elevation.
pub fn orbit_eye(target: Vec3, forward: Vec3, azimuth_deg: f32, elevation_deg: f32, distance: f32) -> Vec3 {
    let direction = Quat::from_rotation_z(azimuth_deg.to_radians()) * forward;
    let el = elevation_deg.to_radians();
    target + direction * (distance * el.cos()) + Vec3::Z * (distance * el.sin())
}

/// A seated avatar carries the seat's tilt in its model rotation, and the
/// A-pose keeps the root as it is: framed with the world's up, the avatar
/// came out tilted in every image. The capture works in the avatar's
/// upright frame instead: the joints are measured with the tilt (the swing
/// from the world's up to the model's up, no yaw) taken off about `pivot`,
/// and the plan's cameras get it back, so the avatar stands straight in the
/// images. A standing avatar has no tilt: identity. The fork frames with
/// the world's up and shows the bug (Aurora fix, custom feature).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Upright {
    pub pivot: Vec3,
    pub tilt: Quat,
}

impl Upright {
    /// `model_up` is the avatar's rotated +Z.
    pub fn new(pivot: Vec3, model_up: Vec3) -> Self {
        let tilt = model_up
            .try_normalize()
            .map_or(Quat::IDENTITY, |up| Quat::from_rotation_arc(Vec3::Z, up));
        Self { pivot, tilt }
    }

    /// A world position in the upright frame.
    pub fn point(&self, p: Vec3) -> Vec3 {
        self.pivot + self.tilt.inverse() * (p - self.pivot)
    }

    /// A world direction in the upright frame.
    pub fn direction(&self, d: Vec3) -> Vec3 {
        self.tilt.inverse() * d
    }

    /// A shot planned in the upright frame, back in the world.
    pub fn shot(&self, mut shot: Shot) -> Shot {
        let back = |p: Vec3| self.pivot + self.tilt * (p - self.pivot);
        shot.target = back(shot.target);
        shot.eye = back(shot.eye);
        shot.up = (self.tilt * shot.up).normalize();
        shot
    }

    /// The upright frame's flat facing in the world, flat again: the
    /// neutral sun's direction.
    pub fn world_forward(&self, forward: Vec3) -> Vec3 {
        let f = self.tilt * forward;
        Vec3::new(f.x, f.y, 0.0).try_normalize().unwrap_or(forward)
    }
}

/// The plan for these metrics and settings, in the fork's order.
pub fn build(m: &Metrics, s: &Settings) -> Vec<Shot> {
    let zoom = (s.zoom_percent.clamp(10.0, 1000.0)) * 0.01;
    let fit =
        |frame: f32, fov: f32, aspect: f32, ratio: f32| distance_for_frame_height(frame / zoom, fov, FRAMING_MARGIN_PERCENT, aspect, ratio);
    let square = s.square();
    let mut shots = Vec::with_capacity(IMAGE_COUNT);
    let orbit = |group, side, target, az, el, distance, fov, size| Shot {
        group,
        side,
        view: View::Orbit,
        azimuth_deg: az,
        elevation_deg: el,
        target,
        eye: orbit_eye(target, m.forward, az, el, distance),
        up: Vec3::Z,
        distance,
        fov,
        size,
    };
    let directed = |group, side, view, target: Vec3, direction: Vec3, up: Vec3, distance, fov| {
        let dir = direction.try_normalize().unwrap_or(Vec3::X);
        Shot {
            group,
            side,
            view,
            azimuth_deg: 0.0,
            elevation_deg: dir.z.clamp(-1.0, 1.0).asin().to_degrees(),
            target,
            eye: target + dir * distance,
            up: safe_up(dir, up, m.forward),
            distance,
            fov,
            size: square,
        }
    };

    // body
    let fov_body = FOV_BODY_DEG.to_radians();
    let fov_detail = FOV_DETAIL_DEG.to_radians();
    let center_z = (m.sole_z + m.head_top_z) * 0.5 + BODY_CENTER_LIFT * m.height + s.height_offset_m;
    let body_center = Vec3::new(m.pelvis.x, m.pelvis.y, center_z);
    let (body_size, body_distance) = if s.portrait_body {
        let size = s.portrait();
        let aspect = size.0 as f32 / size.1 as f32;
        (size, fit(FRAME_BODY * m.height, fov_body, aspect, PORTRAIT_SUBJECT_WIDTH_RATIO))
    } else {
        (square, fit(FRAME_BODY * m.height, fov_body, 1.0, SUBJECT_WIDTH_RATIO))
    };
    for az in TURNAROUND_AZIMUTHS {
        shots.push(orbit(Group::Body, None, body_center, az, 0.0, body_distance, fov_body, body_size));
    }
    // head
    let head_distance = fit(FRAME_HEAD * m.scale, fov_detail, 1.0, SUBJECT_WIDTH_RATIO);
    for az in TURNAROUND_AZIMUTHS {
        shots.push(orbit(
            Group::Head,
            None,
            m.skull_center,
            az,
            HEAD_ELEVATION,
            head_distance,
            fov_detail,
            square,
        ));
    }
    // hands: back and palm, straight along the hand's normal, fingers up;
    // nothing else is drawn, so the camera under the hand sees the palm
    let fov_hand = FOV_HAND_DEG.to_radians();
    let hand_distance = fit(FRAME_HAND * m.scale, fov_hand, 1.0, SUBJECT_WIDTH_RATIO);
    for (i, side) in [Side::Left, Side::Right].into_iter().enumerate() {
        let (c, n, up) = (m.hand_center[i], m.hand_normal[i], m.hand_axis[i]);
        shots.push(directed(
            Group::Hands,
            Some(side),
            View::HandBack,
            c,
            n,
            up,
            hand_distance,
            fov_hand,
        ));
        shots.push(directed(
            Group::Hands,
            Some(side),
            View::HandPalm,
            c,
            -n,
            up,
            hand_distance,
            fov_hand,
        ));
    }
    // feet: both from the front, each from its outer side, the soles from
    // below with the toes up (no terrain is drawn: the camera may sit in it)
    let foot_frame = FRAME_FOOT * m.scale;
    let between = (m.foot_center[0] + m.foot_center[1]) * 0.5;
    let separation = m.foot_center[0].distance(m.foot_center[1]);
    let pair_frame = foot_frame.max(separation + FEET_PAIR_PADDING * m.scale);
    let pair_distance = fit(pair_frame, fov_detail, 1.0, SUBJECT_WIDTH_RATIO);
    let foot_distance = fit(foot_frame, fov_detail, 1.0, SUBJECT_WIDTH_RATIO);
    shots.push(orbit(
        Group::Feet,
        None,
        between,
        0.0,
        FEET_FRONT_ELEVATION,
        pair_distance,
        fov_detail,
        square,
    ));
    shots.push(orbit(
        Group::Feet,
        Some(Side::Left),
        m.foot_center[0],
        90.0,
        FEET_SIDE_ELEVATION,
        foot_distance,
        fov_detail,
        square,
    ));
    shots.push(orbit(
        Group::Feet,
        Some(Side::Right),
        m.foot_center[1],
        -90.0,
        FEET_SIDE_ELEVATION,
        foot_distance,
        fov_detail,
        square,
    ));
    let soles = Vec3::new(between.x, between.y, m.sole_z);
    shots.push(directed(
        Group::Feet,
        None,
        View::Bottom,
        soles,
        Vec3::NEG_Z,
        m.forward,
        pair_distance,
        fov_detail,
    ));
    debug_assert_eq!(shots.len(), IMAGE_COUNT);
    shots
}

/// An up square to the view; when the one asked lies along the view, the
/// avatar's forward, then Z.
fn safe_up(view_dir: Vec3, up: Vec3, forward: Vec3) -> Vec3 {
    for candidate in [up, forward, Vec3::Z, Vec3::X] {
        if let Some(u) = (candidate - view_dir * candidate.dot(view_dir)).try_normalize()
            && candidate.try_normalize().is_some_and(|c| c.dot(view_dir).abs() < 0.999)
        {
            return u;
        }
    }
    Vec3::Z
}

impl Shot {
    /// The fork's nearest compass direction of the camera around the
    /// avatar (directionKey), 0..8 from the front, turning to the left.
    pub fn direction(&self) -> usize {
        let a = (self.azimuth_deg + 360.0).rem_euclid(360.0);
        ((a / 45.0).round() as usize) % 8
    }

    /// The camera for this shot. The view direction is from the eye to the
    /// target; the up is the shot's.
    pub fn camera(&self) -> crate::joystick::flycam::Flycam {
        crate::joystick::flycam::Flycam::from_view(self.eye, self.target, self.up, self.fov)
    }
}
