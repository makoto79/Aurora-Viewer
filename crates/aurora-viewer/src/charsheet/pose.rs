//! The capture's A-pose, local only: a pose layer the app hands the scene
//! like the Poser's (`scene.poser`). Port of LLAvatarCapturePoseMotion (the
//! fork's llavatarcapture.cpp, originally LGPL 2.1): every joint at its rest
//! rotation (the Poser's T-pose, `rig.local_rot`) except the shoulders
//! lowered and the hips spread, the ankles turned back so the soles stay
//! flat. Rotations only, apart from the pelvis held at its rest position:
//! the other positions stay as captured, so mesh bodies keep their
//! proportions.
//!
//! Difference from the fork (decision A9): tails, wings and hind legs keep
//! the rotations of the instant the pose was made instead of animating on;
//! the world is frozen for the run anyway.

use crate::ik_edit::poser::SubjectPose;
use crate::scene::anim::Rig;
use glam::Quat;

/// The rest pose has the arms straight out: lowering both shoulders about
/// the forward axis is all it takes. Palms stay down, fingers straight.
pub const SHOULDER_DROP_DEG: f32 = 40.0;
/// The feet apart, each seen on its own.
pub const HIP_SPREAD_DEG: f32 = 7.0;

/// Joints whose rest pose is no neutral pose (a tail at rest sticks
/// straight out backwards), and the root that carries position and facing.
fn kept(name: &str) -> bool {
    name == "mRoot" || ["mTail", "mWing", "mHindLimb"].iter().any(|p| name.starts_with(p))
}

/// The A-pose built on `captured` (the subject's pose, as the Poser
/// captures it). None when the rig and the pose do not match.
pub fn a_pose(rig: &Rig, captured: &SubjectPose) -> Option<SubjectPose> {
    if !captured.valid(rig) {
        return None;
    }
    let mut pose = captured.clone();
    pose.locks.clear();
    let drop = SHOULDER_DROP_DEG.to_radians();
    let spread = HIP_SPREAD_DEG.to_radians();
    for b in 0..rig.len() {
        let name = rig.canonical_names[b].as_str();
        if kept(name) {
            continue;
        }
        // +X forward, +Y left, +Z up: a turn about X by a negative angle
        // takes the left arm (+Y) down, a positive one the left leg (−Z)
        // out to the left
        let turn = match name {
            "mShoulderLeft" => Quat::from_rotation_x(-drop),
            "mShoulderRight" => Quat::from_rotation_x(drop),
            "mHipLeft" => Quat::from_rotation_x(spread),
            "mHipRight" => Quat::from_rotation_x(-spread),
            "mAnkleLeft" => Quat::from_rotation_x(-spread),
            "mAnkleRight" => Quat::from_rotation_x(spread),
            _ => Quat::IDENTITY,
        };
        pose.rotations[b] = (turn * rig.local_rot[b]).normalize();
        pose.origin[b] = rig.local_rot[b];
    }
    // no stand sway, no sitting height: the pelvis where the skeleton has it
    pose.positions[rig.pelvis] = rig.local_pos[rig.pelvis];
    pose.valid(rig).then_some(pose)
}
