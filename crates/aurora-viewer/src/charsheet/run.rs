//! The steps of a run, pure and driven by `tick` (the app turns its
//! commands into camera moves and photo jobs, `app/charsheet.rs`). Port of
//! LLAvatarCaptureSession::start, onIdle and captureNextShot (the fork's
//! llavatarcapture.cpp, originally LGPL 2.1): pose, measure, countdown,
//! freeze and settle, then one shot after the other; a partial series still
//! gets its contact sheet and manifest.
//!
//! Aurora differences: each shot waits a few frames after its camera move
//! (the renderer's temporal passes, shadows and the levels of detail follow
//! the new view) and, on the first shot of a group, for the textures in
//! view, instead of the fork's one discarded warm-up render.

use std::time::{Duration, Instant};

/// The pose shows in the skeleton after this (POSE_SETTLE_SECONDS).
pub const POSE_SETTLE: Duration = Duration::from_millis(300);
pub const POSE_SETTLE_FRAMES: u32 = 2;
/// Probes, shadows and the exposure follow the isolation and the freeze
/// (CAPTURE_SETTLE_SECONDS).
pub const CAPTURE_SETTLE: Duration = Duration::from_millis(1500);
/// The reflection probes follow a new sky (NEUTRAL_LIGHT_SETTLE_SECONDS).
pub const LIGHT_SETTLE: Duration = Duration::from_secs(2);
/// The longest wait for the neutral sky to load.
pub const LIGHT_TIMEOUT: Duration = Duration::from_secs(5);
/// After each camera move: frames and time before the shot is asked.
pub const SHOT_FRAMES: u32 = 3;
pub const SHOT_MIN_WAIT: Duration = Duration::from_millis(120);
/// The first shot of a group waits this long at most for its textures.
pub const TEXTURE_WAIT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    Completed,
    /// Stop, Esc or the window closed.
    Stopped,
    /// The avatar left, was derendered or muted.
    AvatarLost,
    /// A teleport or a region change: the camera is not put back.
    Teleport,
    LoggedOut,
    MeasureFailed,
    /// The writer could not write (folder, disk).
    WriteFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// The A-pose layer on.
    Pose,
    /// Measure and build the plan (`Run::measured` answers).
    Measure,
    /// The neutral light on.
    Light,
    /// Freeze, isolation, render state, the writer started.
    Prepare,
    /// The camera on shot i.
    Camera(usize),
    /// A photo job for shot i (`Run::shot_done` answers).
    Ask(usize),
    /// Everything back, the writer finishes.
    Finish(Stop),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Posing { since: Instant, frames: u32 },
    Measuring,
    Countdown { until: Instant },
    Settle { until: Instant, light_deadline: Instant },
    Wait { shot: usize, since: Instant, frames: u32 },
    Asked(usize),
    Done(Stop),
}

/// What the app sees this frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct Input {
    /// Textures in view still below their wanted resolution.
    pub textures_pending: usize,
    /// The neutral sky is in place (or none was asked).
    pub light_ready: bool,
    /// No other photo job runs.
    pub photo_idle: bool,
}

#[derive(Clone, Debug)]
pub struct Run {
    pub phase: Phase,
    pub light: bool,
    pub delay: Duration,
    /// Shots in the plan, once measured.
    pub total: usize,
    /// Shots answered (written or failed).
    pub done: usize,
    pub failed: usize,
    /// The group of each shot, to wait for textures on a group's first.
    firsts: Vec<bool>,
}

impl Run {
    /// A run asked at `now`. The first commands come from the first tick.
    pub fn new(now: Instant, pose: bool, light: bool, delay: Duration) -> (Run, Vec<Command>) {
        let mut run = Run {
            phase: Phase::Measuring,
            light,
            delay,
            total: 0,
            done: 0,
            failed: 0,
            firsts: Vec::new(),
        };
        let cmds = if pose {
            run.phase = Phase::Posing { since: now, frames: 0 };
            vec![Command::Pose]
        } else {
            vec![Command::Measure]
        };
        (run, cmds)
    }

    pub fn running(&self) -> bool {
        !matches!(self.phase, Phase::Done(_))
    }

    /// The plan was built (`firsts`: whether each shot starts its group),
    /// or the measurement failed (None).
    pub fn measured(&mut self, now: Instant, firsts: Option<Vec<bool>>) -> Vec<Command> {
        if self.phase != Phase::Measuring {
            return Vec::new();
        }
        let Some(firsts) = firsts.filter(|f| !f.is_empty()) else {
            return self.stop(Stop::MeasureFailed);
        };
        self.total = firsts.len();
        self.firsts = firsts;
        // with the neutral light: at least LIGHT_SETTLE before the first shot
        let wait = if self.light { self.delay.max(LIGHT_SETTLE) } else { self.delay };
        self.phase = Phase::Countdown { until: now + wait };
        if self.light { vec![Command::Light] } else { Vec::new() }
    }

    /// Shot i was answered: written to the writer (`ok`) or not drawn.
    pub fn shot_done(&mut self, now: Instant, shot: usize, ok: bool) -> Vec<Command> {
        if self.phase != Phase::Asked(shot) {
            return Vec::new();
        }
        self.done += 1;
        if !ok {
            self.failed += 1;
        }
        if shot + 1 < self.total {
            self.phase = Phase::Wait {
                shot: shot + 1,
                since: now,
                frames: 0,
            };
            vec![Command::Camera(shot + 1)]
        } else {
            self.stop(Stop::Completed)
        }
    }

    /// Ends the run now, whatever it was doing.
    pub fn stop(&mut self, reason: Stop) -> Vec<Command> {
        if !self.running() {
            return Vec::new();
        }
        self.phase = Phase::Done(reason);
        vec![Command::Finish(reason)]
    }

    pub fn tick(&mut self, now: Instant, input: Input) -> Vec<Command> {
        match self.phase {
            Phase::Posing { since, frames } => {
                let frames = frames + 1;
                if frames >= POSE_SETTLE_FRAMES && now.duration_since(since) >= POSE_SETTLE {
                    self.phase = Phase::Measuring;
                    vec![Command::Measure]
                } else {
                    self.phase = Phase::Posing { since, frames };
                    Vec::new()
                }
            }
            Phase::Countdown { until } if now >= until => {
                self.phase = Phase::Settle {
                    until: now + CAPTURE_SETTLE,
                    light_deadline: now + LIGHT_TIMEOUT,
                };
                vec![Command::Prepare]
            }
            Phase::Settle { until, light_deadline } if now >= until && (input.light_ready || now >= light_deadline) => {
                self.phase = Phase::Wait {
                    shot: 0,
                    since: now,
                    frames: 0,
                };
                vec![Command::Camera(0)]
            }
            Phase::Wait { shot, since, frames } => {
                let frames = frames + 1;
                let waited = now.duration_since(since);
                let first = self.firsts.get(shot).copied().unwrap_or(false);
                let textures = !first || input.textures_pending == 0 || waited >= TEXTURE_WAIT;
                if frames >= SHOT_FRAMES && waited >= SHOT_MIN_WAIT && textures && input.photo_idle {
                    self.phase = Phase::Asked(shot);
                    vec![Command::Ask(shot)]
                } else {
                    self.phase = Phase::Wait { shot, since, frames };
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }

    /// Seconds left of the countdown, rounded up.
    pub fn countdown(&self, now: Instant) -> Option<u32> {
        match self.phase {
            Phase::Countdown { until } => Some(until.saturating_duration_since(now).as_secs_f32().ceil() as u32),
            _ => None,
        }
    }

    /// The shot being taken.
    pub fn shot(&self) -> Option<usize> {
        match self.phase {
            Phase::Wait { shot, .. } | Phase::Asked(shot) => Some(shot),
            _ => None,
        }
    }

    /// 0..=1.
    pub fn progress(&self) -> f32 {
        match self.phase {
            Phase::Done(Stop::Completed) => 1.0,
            _ if self.total == 0 => 0.0,
            _ => self.done as f32 / self.total as f32,
        }
    }

    /// The run holds the camera, the light, the freeze (from Prepare on).
    pub fn prepared(&self) -> bool {
        matches!(self.phase, Phase::Settle { .. } | Phase::Wait { .. } | Phase::Asked(_))
    }
}
