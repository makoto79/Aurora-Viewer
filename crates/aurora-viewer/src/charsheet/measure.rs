//! The avatar measured once from its skeleton, for the framing of every
//! shot. Port of LLAvatarCaptureSession::measureAvatar and
//! measureHandOrientation (indra/newview/llavatarcapture.cpp of the fork,
//! originally LGPL 2.1): same joints, same constants, same fallbacks.

use glam::Vec3;

/// Body measurements are scaled relative to an avatar of this height.
pub const REFERENCE_HEIGHT: f32 = 1.8;
/// From the last joint to the silhouette, at reference scale.
pub const ANKLE_TO_SOLE: f32 = 0.04;
pub const HEAD_TO_CROWN: f32 = 0.13;
/// Without Bento fingers: how far past the wrist the middle of the hand is.
pub const HAND_CENTER_OFFSET: f32 = 0.09;
/// Feet framed a little below their middle so the sole stays in frame.
pub const FOOT_TARGET_DROP: f32 = 0.02;
/// Shorter than this, the measurement failed (a skeleton still loading).
pub const MIN_HEIGHT: f32 = 0.3;

/// Index 0 is the left side, 1 the right one (the fork's `side`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    /// Rest body height over REFERENCE_HEIGHT (1 when unknown).
    pub scale: f32,
    pub pelvis: Vec3,
    pub hand_center: [Vec3; 2],
    /// Finger direction (the image's up in the hand shots).
    pub hand_axis: [Vec3; 2],
    /// Out of the back of the hand.
    pub hand_normal: [Vec3; 2],
    pub foot_center: [Vec3; 2],
    pub head_top_z: f32,
    pub sole_z: f32,
    pub height: f32,
    /// The middle of the skull: the head turnaround's centre.
    pub skull_center: Vec3,
    /// The avatar's facing, flat, unit length.
    pub forward: Vec3,
}

const SIDES: [&str; 2] = ["Left", "Right"];

/// `joint` gives a joint's world position by its skeleton name; `body_height`
/// is the rest pose's height (mBodySize.mV[VZ]: a stable reference while
/// posed); `facing` the avatar's rotated +X.
pub fn measure(joint: impl Fn(&str) -> Option<Vec3>, body_height: f32, facing: Vec3) -> Option<Metrics> {
    let side = |base: &str, s: usize| joint(&format!("{base}{}", SIDES[s]));
    let head = joint("mHead")?;
    joint("mChest")?;
    let pelvis = joint("mPelvis")?;
    let mut wrist = [Vec3::ZERO; 2];
    let mut elbow = [Vec3::ZERO; 2];
    let mut ankle = [Vec3::ZERO; 2];
    for s in 0..2 {
        wrist[s] = side("mWrist", s)?;
        elbow[s] = side("mElbow", s)?;
        ankle[s] = side("mAnkle", s)?;
        // required like the fork's, though only the others are measured
        side("mHip", s)?;
    }
    let scale = if body_height.is_finite() && body_height > 0.5 {
        body_height / REFERENCE_HEIGHT
    } else {
        1.0
    };

    let mut hand_center = [Vec3::ZERO; 2];
    let mut hand_axis = [Vec3::ZERO; 2];
    let mut hand_normal = [Vec3::ZERO; 2];
    let mut foot_center = [Vec3::ZERO; 2];
    for s in 0..2 {
        let forearm = (wrist[s] - elbow[s])
            .try_normalize()
            .unwrap_or(if s == 0 { Vec3::Y } else { Vec3::NEG_Y });
        // between wrist and fingertip, or the hand sits in the frame's edge
        hand_center[s] = match side("mHandMiddle3", s) {
            Some(tip) => (wrist[s] + tip) * 0.5,
            None => wrist[s] + forearm * (HAND_CENTER_OFFSET * scale),
        };
        let fingers = [side("mHandMiddle1", s), side("mHandIndex1", s), side("mHandPinky1", s)];
        (hand_axis[s], hand_normal[s]) = hand_orientation(s == 0, wrist[s], forearm, fingers);
        // the ankle is at the back of the foot: its middle, a little lower
        let front = side("mToe", s).or_else(|| side("mFoot", s));
        foot_center[s] = front.map_or(ankle[s], |f| (ankle[s] + f) * 0.5);
        foot_center[s].z -= FOOT_TARGET_DROP * scale;
    }

    // the crown: the √2 skull approximation the viewer's body size uses
    let mut head_top_z = head.z + HEAD_TO_CROWN * scale;
    if let Some(skull) = joint("mSkull") {
        let estimate = head.z + std::f32::consts::SQRT_2 * (skull.z - head.z);
        if estimate > head.z {
            head_top_z = estimate;
        }
    }
    let mut lowest = ankle[0].z.min(ankle[1].z);
    for s in 0..2 {
        if let Some(foot) = side("mFoot", s) {
            lowest = lowest.min(foot.z);
        }
    }
    let sole_z = lowest - ANKLE_TO_SOLE * scale;
    let height = head_top_z - sole_z;
    if !height.is_finite() || height < MIN_HEIGHT {
        return None;
    }
    // the head orbit turns around the middle of the skull, not the eyes at
    // its front (the camera would come far too close behind the head)
    let skull_center = Vec3::new(head.x, head.y, (head.z + head_top_z) * 0.5);
    let forward = Vec3::new(facing.x, facing.y, 0.0).try_normalize().unwrap_or(Vec3::X);
    let metrics = Metrics {
        scale,
        pelvis,
        hand_center,
        hand_axis,
        hand_normal,
        foot_center,
        head_top_z,
        sole_z,
        height,
        skull_center,
        forward,
    };
    metrics.finite().then_some(metrics)
}

impl Metrics {
    fn finite(&self) -> bool {
        let v = |v: &Vec3| v.is_finite();
        self.scale.is_finite()
            && v(&self.pelvis)
            && self.hand_center.iter().all(v)
            && self.hand_axis.iter().all(v)
            && self.hand_normal.iter().all(v)
            && self.foot_center.iter().all(v)
            && self.head_top_z.is_finite()
            && self.sole_z.is_finite()
            && v(&self.skull_center)
            && v(&self.forward)
    }
}

/// The finger direction and the normal out of the back of the hand, from
/// the knuckles (middle, index, pinky), or a flat hand along the forearm
/// with its back up, as in the A-pose.
fn hand_orientation(left: bool, wrist: Vec3, forearm: Vec3, fingers: [Option<Vec3>; 3]) -> (Vec3, Vec3) {
    if let [Some(middle), Some(index), Some(pinky)] = fingers
        && let (Some(axis), Some(across)) = ((middle - wrist).try_normalize(), (pinky - index).try_normalize())
    {
        // at rest the left hand points along +Y with the index forward:
        // axis × across points out of its back; the mirrored right hand
        // needs the other sign
        let normal = axis.cross(across) * if left { 1.0 } else { -1.0 };
        if let Some(normal) = normal.try_normalize()
            // the knuckles are not in line with the wrist: the finger
            // direction square to the normal, to serve as the image's up
            && let Some(axis) = (axis - normal * axis.dot(normal)).try_normalize()
        {
            return (axis, normal);
        }
    }
    let normal = (Vec3::Z - forearm * Vec3::Z.dot(forearm)).try_normalize().unwrap_or(Vec3::X);
    (forearm, normal)
}
