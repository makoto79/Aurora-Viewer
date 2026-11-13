//! The character sheet capture in the app (the fork's LLFloaterAvatarSheet
//! and LLAvatarCaptureSession, originally LGPL 2.1): the window's actions,
//! the preview, the run's commands (`charsheet::run`) turned into the
//! camera, the A-pose layer, the neutral light, the frozen world, the
//! isolation and one photo job per shot (`Purpose::Sheet`), the images
//! handed to the writer (`charsheet::output`).
//!
//! Aurora rules (the plan's decisions): the A-pose on someone else only
//! where the Poser may pose them (their consent, or the local-nearby
//! opt-in), their own pose otherwise; the camera locked while the preview
//! or a run holds it (End preview, Esc or closing the window give it back);
//! the Photo window and the capture never both at once. RLVa is not
//! consulted, as in the fork.

use super::*;
use crate::charsheet::measure::{self, Metrics};
use crate::charsheet::output::{self, Event, Writer};
use crate::charsheet::plan::{self, Group, Shot, Side, View as ShotView};
use crate::charsheet::run::{Command, Input, Run, Stop};
use crate::charsheet::{naming, pose};
use crate::ik_edit::poser::SubjectPose;
use crate::ui::charsheet::{Action, Candidate, View};
use crate::world::eep_photo::PhotoChoice;
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

/// The neutral light's sun: in front of the avatar, this high, so the
/// back views are not in the avatar's own shadow (NEUTRAL_SUN_ELEVATION_DEG).
const NEUTRAL_SUN_ELEVATION_DEG: f32 = 60.0;
/// The photo window's "noon" time of day (`snapshot::light::PhotoLight`).
const NOON: u8 = 2;
/// The capture's water level under its camera (`charsheet_scene`): well
/// past anything a shot frames.
const CAPTURE_WATER_BELOW_M: f32 = 100.0;

#[derive(Default)]
pub(super) struct Charsheet {
    pub ui: crate::ui::charsheet::CharsheetUi,
    /// The A-pose for the subject, made once from its pose when it starts.
    pose: Option<PoseLayer>,
    preview: Option<Preview>,
    run: Option<Running>,
    /// The flycam the capture replaced, given back when it ends.
    camera_before: Option<Option<crate::joystick::flycam::Flycam>>,
    /// The world frozen by the capture (not by the Photo window).
    froze: bool,
    status: Option<(String, bool)>,
    last_folder: Option<PathBuf>,
    folder_pick: Option<crossbeam_channel::Receiver<Option<PathBuf>>>,
    /// The info line of the chosen avatar, and when it was measured.
    info: Option<(Uuid, Instant, String, bool)>,
}

struct PoseLayer {
    subject: Uuid,
    layer: Option<Arc<SubjectPose>>,
    since: Instant,
}

struct Preview {
    index: usize,
    plan: Vec<Shot>,
    forward: glam::Vec3,
    since: Instant,
}

struct Running {
    run: Run,
    subject: Uuid,
    plan: Vec<Shot>,
    forward: glam::Vec3,
    writer: Option<Writer>,
    /// An image the writer's queue had no room for yet.
    pending: Option<(usize, Shot, u32, u32, Vec<u8>)>,
    region: Option<u64>,
    /// The writer was told no more images come.
    finished: bool,
    stop: Option<Stop>,
    lit: bool,
}

impl App {
    /// The capture holds the camera: the preview, or a run from its
    /// preparation to its end.
    pub(super) fn charsheet_owns_camera(&self) -> bool {
        let c = &self.charsheet;
        // (a run started from the preview keeps the camera through its
        // countdown: it is still on the preview's shot)
        c.preview.is_some()
            || c.run
                .as_ref()
                .is_some_and(|r| r.run.prepared() || (r.run.running() && c.camera_before.is_some()))
    }

    /// The preview's backdrop colour in place of the checkerboard (linear).
    pub(super) fn charsheet_backdrop(&self) -> Option<[f32; 3]> {
        let s = &self.settings.charsheet;
        (self.charsheet_owns_camera() && !s.transparent).then(|| s.background.map(egui::ecolor::linear_from_gamma))
    }

    /// Esc: the preview ends, a run stops. True when it was the capture's.
    pub(super) fn charsheet_escape(&mut self) -> bool {
        if self.charsheet.run.as_ref().is_some_and(|r| r.run.running()) {
            self.charsheet_stop(Stop::Stopped);
            return true;
        }
        if self.charsheet.preview.is_some() {
            self.charsheet_end_preview();
            return true;
        }
        false
    }

    /// The window opens with this avatar chosen (its context menu, the
    /// demo). Not while a run goes.
    pub(super) fn open_charsheet(&mut self, subject: Uuid) {
        self.charsheet.ui.open = true;
        if self.charsheet.run.as_ref().is_some_and(|r| r.run.running()) {
            return;
        }
        if subject != self.charsheet.ui.subject {
            self.charsheet_end_preview();
            self.charsheet.ui.subject = subject;
        }
    }

    pub(super) fn toggle_charsheet(&mut self) {
        if self.charsheet.ui.open {
            self.charsheet_close();
        } else {
            let own = self.world.agent_id;
            self.open_charsheet(if self.charsheet.ui.subject.is_nil() {
                own
            } else {
                self.charsheet.ui.subject
            });
        }
    }

    fn charsheet_close(&mut self) {
        self.charsheet.ui.open = false;
        self.charsheet_stop(Stop::Stopped);
        self.charsheet_end_preview();
    }

    fn charsheet_subject(&self) -> Uuid {
        let s = self.charsheet.ui.subject;
        if s.is_nil() { self.world.agent_id } else { s }
    }

    /// The A-pose may be put on this avatar: ourselves, or whom the Poser
    /// may pose (`poser_consent::can_pose`: consent or the local-nearby
    /// opt-in). Stricter than the fork, which posed anyone.
    fn charsheet_may_pose(&self, subject: Uuid) -> bool {
        subject == self.world.agent_id || self.world.poser_consent.can_pose(&self.world, subject)
    }

    /// Whether the avatar is still there to be captured.
    fn charsheet_present(&self, subject: Uuid) -> bool {
        !subject.is_nil()
            && self.world.objects.index_of_uuid(&subject).is_some()
            && (subject == self.world.agent_id || !self.world.mutes.is_muted_id(&subject))
    }

    // ------------------------------------------------------------ per frame

    /// After the Poser's layers: the A-pose layer of the subject, unless the
    /// Poser poses it (its pose is used as is, as in the fork).
    pub(super) fn charsheet_pose_layer(&mut self) {
        let Some(p) = &mut self.charsheet.pose else { return };
        if self.scene.poser.contains_key(&p.subject) {
            return;
        }
        if p.layer.is_none() {
            let rig = &self.world.avatar_lib.rig;
            p.layer = crate::ik_edit::Frame::subject(&self.world, p.subject)
                .and_then(|frame| SubjectPose::capture(rig, &frame))
                .and_then(|captured| pose::a_pose(rig, &captured))
                .map(Arc::new);
            if p.layer.is_none() {
                return;
            }
            p.since = Instant::now();
        }
        if let Some(layer) = &p.layer {
            self.scene.poser.insert(p.subject, layer.clone());
        }
    }

    /// Before the camera's update: the shot's view, every frame, so that no
    /// input moves it (the camera is locked while the capture holds it).
    pub(super) fn charsheet_camera(&mut self) {
        // the avatar does not walk out of its frame: from the preview, or
        // the run's start (its plan is measured before the countdown), on
        if self.charsheet_active() {
            self.input = Default::default();
        }
        let shot = if let Some(p) = &self.charsheet.preview {
            p.plan.get(p.index).copied()
        } else if let Some(r) = self.charsheet.run.as_ref().filter(|r| r.run.prepared()) {
            r.plan.get(r.run.shot().unwrap_or(0)).copied()
        } else {
            None
        };
        let Some(shot) = shot else { return };
        if self.charsheet.camera_before.is_none() {
            self.charsheet.camera_before = Some(self.camera.flycam);
            if self.camera.mouselook() {
                self.camera.change_to_default(&mut self.world.agent, &self.settings.camera);
            }
        }
        self.camera.flycam = Some(shot.camera());
    }

    fn charsheet_release_camera(&mut self, restore: bool) {
        if let Some(before) = self.charsheet.camera_before.take() {
            self.camera.clear_flycam();
            if restore && let Some(flycam) = before {
                self.camera.flycam = Some(flycam);
            }
        }
    }

    /// The preview or a run holds the camera (and the scene).
    pub(super) fn charsheet_active(&self) -> bool {
        self.charsheet.preview.is_some() || self.charsheet.run.as_ref().is_some_and(|r| r.run.running())
    }

    /// The scene of the preview and the run, after the frame's parameters,
    /// as the avatar studio's stage strips its own (`studio_render`): the
    /// weather is the user's view, never part of a character sheet. No
    /// drops or flakes, no lightning, no aurora glow in the sky, no haze,
    /// no mirror; the snow on the avatars is kept but not drawn
    /// (`scene.avatar_snow_hidden`). The weather's settings and memories
    /// are left as they are: everything is back with the next frame after.
    /// Not in the fork, which had no weather (Aurora addition).
    ///
    /// No water either: a camera below the region's water (a steep seat
    /// puts the camera behind the avatar deep into the ground) drew the
    /// whole image through the underwater fog, though no water is drawn.
    /// The water level goes far below the camera and the water's lists are
    /// emptied, as the avatar studio's stage does.
    pub(super) fn charsheet_scene(&mut self, fp: &mut aurora_render::FrameParams) {
        fp.rain = 0.0;
        fp.rain_look.lightning = [0.0; 4];
        fp.rain_look.aurora = [0.0; 4];
        fp.fog_density = 0.0;
        fp.mirror = None;
        fp.water_height = fp.camera_pos.z - CAPTURE_WATER_BELOW_M;
        let lists = &mut self.scene.lists;
        lists.water.clear();
        lists.water_events.clear();
        lists.water_contacts.clear();
        lists.water_ground = Default::default();
    }

    /// The neutral light while the preview or a run holds it (None: the
    /// capture leaves the light to the Photo window and the user).
    pub(super) fn charsheet_light(&self) -> Option<PhotoChoice> {
        if !self.settings.charsheet.neutral_light || self.world.eep.editing() {
            return None;
        }
        let forward = if let Some(p) = &self.charsheet.preview {
            p.forward
        } else {
            self.charsheet.run.as_ref().filter(|r| r.lit && r.run.running())?.forward
        };
        let yaw = forward.y.atan2(forward.x);
        let sun = glam::Quat::from_rotation_z(yaw) * glam::Quat::from_rotation_y(-NEUTRAL_SUN_ELEVATION_DEG.to_radians());
        Some(PhotoChoice {
            time: Some(NOON),
            sun: Some(sun),
            // the region's clouds would dim and flatten the light of one
            // sim against another
            no_clouds: true,
            ..Default::default()
        })
    }

    /// AURORA_DEMO_CHARSHEET=<mode> (docs/TESTING.md): the window, the
    /// preview's steps, a full run (`run`, `run-transparent`,
    /// `run-portrait`), someone else's avatar (`other`, `other-local`) and
    /// a run ended early (`stop`, `lost`). The images go to
    /// AURORA_DEMO_CHARSHEET_DIR, else the temporary folder; the settings are
    /// changed for the session only, never saved.
    fn charsheet_demo(&mut self, mode: &str) {
        let frame = self.frame_count;
        let peer = crate::demo::poser::PEER;
        let other = mode.starts_with("other");
        if frame == 280
            && other
            && let Some(handle) = self.world.main_region
        {
            self.world.apply(aurora_net::NetEvent::ObjectUpdates {
                handle,
                objects: vec![crate::demo::poser::peer()],
            });
            // other-local: the "pose without asking" switch on, never saved
            if mode == "other-local" {
                self.settings.pose_requests.local_nearby = true;
            }
        }
        if frame == 300 {
            let s = &mut self.settings.charsheet;
            s.delay_s = 0;
            s.transparent = mode == "run-transparent";
            s.portrait_body = mode == "run-portrait";
            if mode == "run-portrait" {
                s.resolution = crate::charsheet::Resolution::R1536;
            }
            let dir = std::env::var_os("AURORA_DEMO_CHARSHEET_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::temp_dir().join("aurora-charsheet-demo"));
            s.folder = Some(dir);
            let subject = if other { peer } else { self.world.agent_id };
            self.open_charsheet(subject);
            log::info!("demo charsheet: {mode}, window open on {subject}");
        }
        let now = Instant::now();
        match mode {
            "preview" | "other" | "other-local" => {
                // the shots 1, 7 (head), 13 (back of the left hand), 20 (soles)
                let steps = [(360, 0usize), (480, 6), (600, 12), (720, 19)];
                if let Some((_, target)) = steps.iter().find(|(f, _)| *f == frame) {
                    let current = self.charsheet.preview.as_ref().map(|p| p.index);
                    let delta = current.map_or(1, |c| *target as i32 - c as i32);
                    if current.is_none() || delta != 0 {
                        self.charsheet_preview_step(delta, now);
                        if let Some(p) = &mut self.charsheet.preview {
                            p.index = *target;
                        }
                    }
                    let label = self
                        .charsheet
                        .preview
                        .as_ref()
                        .and_then(|p| p.plan.get(p.index))
                        .map(shot_label)
                        .unwrap_or_default();
                    log::info!("demo charsheet: preview shot {} ({label}) at frame {frame}", target + 1);
                }
                if frame == 840 {
                    self.charsheet_end_preview();
                    log::info!("demo charsheet: preview ended at frame {frame}");
                }
            }
            "run" | "run-transparent" | "run-portrait" | "stop" | "lost" if frame == 330 => self.charsheet_start(),
            _ => {}
        }
        let shot = self.charsheet.run.as_ref().and_then(|r| r.run.shot());
        if mode == "stop" && shot == Some(7) {
            log::info!("demo charsheet: Stop pressed at shot 8");
            self.charsheet_stop(Stop::Stopped);
        }
        if mode == "lost" && shot == Some(4) {
            // (the avatar's leaving, as the stop check would find it)
            log::info!("demo charsheet: the avatar is gone at shot 5");
            self.charsheet_stop(Stop::AvatarLost);
        }
    }

    /// Once per frame, before the photo's frame is planned.
    pub(super) fn charsheet_tick(&mut self, renderer: &Renderer) {
        if self.demo
            && let Ok(mode) = std::env::var("AURORA_DEMO_CHARSHEET")
        {
            self.charsheet_demo(&mode);
        }
        self.charsheet_folder_answer();
        if self.charsheet.ui.subject.is_nil() && !self.world.agent_id.is_nil() {
            self.charsheet.ui.subject = self.world.agent_id;
        }
        let now = Instant::now();
        // the Photo window and the capture never both
        if self.panels.snapshot {
            if self.charsheet.run.as_ref().is_some_and(|r| r.run.running()) {
                self.panels.snapshot = false;
                self.charsheet.status = Some((aurora_i18n::tr!("charsheet-photo-window-busy").to_owned(), true));
            } else if self.charsheet.preview.is_some() {
                self.charsheet_end_preview();
            }
        }
        // the preview: lost subject, teleport, logout
        if self.charsheet.preview.is_some() {
            let subject = self.charsheet_subject();
            if !self.in_world() || self.world.teleporting || !self.charsheet_present(subject) {
                self.charsheet_end_preview();
            } else {
                self.charsheet_preview_follow(now);
            }
        }
        self.charsheet_run_tick(now, renderer);
    }

    fn charsheet_run_tick(&mut self, now: Instant, renderer: &Renderer) {
        let Some(r) = &self.charsheet.run else { return };
        if r.run.running() {
            let subject = r.subject;
            let stop = if !self.in_world() {
                Some(Stop::LoggedOut)
            } else if self.world.teleporting || self.world.main_region != r.region {
                Some(Stop::Teleport)
            } else if !self.charsheet_present(subject) {
                Some(Stop::AvatarLost)
            } else {
                None
            };
            if let Some(stop) = stop {
                self.charsheet_stop(stop);
            }
        }
        // the writer's news
        let events = self
            .charsheet
            .run
            .as_ref()
            .and_then(|r| r.writer.as_ref())
            .map(|w| w.poll())
            .unwrap_or_default();
        for event in events {
            match event {
                Event::Saved { .. } => {}
                Event::Failed(e) => {
                    self.charsheet.status = Some((aurora_i18n::tr!("charsheet-write-failed", error = e.as_str()), true));
                    self.charsheet_stop(Stop::WriteFailed);
                }
                Event::Done(summary) => self.charsheet_done(summary),
            }
        }
        let Some(r) = &mut self.charsheet.run else { return };
        // an image waiting for room in the writer's queue
        if let Some((i, shot, w, h, px)) = r.pending.take()
            && let Some(writer) = &mut r.writer
            && let Err(px) = writer.image(i, shot, w, h, px)
        {
            r.pending = Some((i, shot, w, h, px));
        }
        if !r.run.running() {
            if !r.finished && r.pending.is_none() {
                r.finished = true;
                match &mut r.writer {
                    Some(writer) => writer.finish(),
                    None => {
                        let stop = r.stop;
                        self.charsheet.run = None;
                        self.charsheet_report(stop, None);
                    }
                }
            }
            return;
        }
        // the shot asked was not drawn (the job given up): skipped
        if let Some(asked) = r
            .run
            .shot()
            .filter(|_| matches!(r.run.phase, crate::charsheet::run::Phase::Asked(_)))
            && !self.snapshots.drawing()
        {
            log::warn!("charsheet: shot {} was not drawn, skipped", asked + 1);
            let cmds = r.run.shot_done(now, asked, false);
            self.charsheet_commands(cmds, now, None);
        }
        let photo_idle = !self.photo_busy();
        let Some(r) = &mut self.charsheet.run else { return };
        let input = Input {
            textures_pending: self.scene.textures.pending_in_view(),
            light_ready: !r.lit || self.world.eep.editing() || !self.world.eep.photo_loading(),
            photo_idle,
        };
        let cmds = r.run.tick(now, input);
        self.charsheet_commands(cmds, now, Some(renderer));
        // the status line follows the run
        if let Some(r) = &self.charsheet.run
            && r.run.running()
        {
            let line = if let Some(s) = r.run.countdown(now) {
                aurora_i18n::tr!("charsheet-countdown", seconds = s)
            } else if let Some(i) = r.run.shot() {
                let label = r.plan.get(i).map(shot_label).unwrap_or_default();
                aurora_i18n::tr!("charsheet-capturing", index = i + 1, total = r.run.total, shot = label.as_str())
            } else {
                aurora_i18n::tr!("charsheet-preparing").to_owned()
            };
            self.charsheet.status = Some((line, false));
        }
    }

    fn charsheet_commands(&mut self, cmds: Vec<Command>, now: Instant, renderer: Option<&Renderer>) {
        for cmd in cmds {
            match cmd {
                Command::Pose => {
                    let subject = self.charsheet.run.as_ref().map_or(Uuid::nil(), |r| r.subject);
                    self.charsheet_want_pose(subject, now);
                }
                Command::Measure => {
                    let Some(r) = &self.charsheet.run else { continue };
                    let subject = r.subject;
                    let measured = self.charsheet_measure(subject);
                    let firsts = measured.as_ref().map(|(_, plan)| firsts(plan));
                    if let (Some(r), Some((m, plan))) = (&mut self.charsheet.run, measured) {
                        r.plan = plan;
                        r.forward = m.forward;
                    }
                    if firsts.is_none() {
                        self.charsheet.status = Some((aurora_i18n::tr!("charsheet-measure-failed").to_owned(), true));
                    }
                    let more = self.charsheet.run.as_mut().map(|r| r.run.measured(now, firsts)).unwrap_or_default();
                    self.charsheet_commands(more, now, renderer);
                }
                Command::Light => {
                    if let Some(r) = &mut self.charsheet.run {
                        r.lit = true;
                    }
                }
                Command::Prepare => self.charsheet_prepare(now),
                Command::Camera(_) => {}
                Command::Ask(i) => {
                    if let Some(renderer) = renderer {
                        self.photo_job(super::snapshot::Purpose::Sheet(i), renderer);
                    }
                }
                Command::Finish(stop) => self.charsheet_finish(stop),
            }
        }
    }

    /// The run's world: frozen, the subject alone, the writer started.
    fn charsheet_prepare(&mut self, now: Instant) {
        let Some(r) = &self.charsheet.run else { return };
        let subject = r.subject;
        // the Photo window's freeze is its own: only ours is undone at the end
        if !self.photo_world_frozen() {
            self.world.freeze_motion(Some(now));
            self.scene.pause_all_skeletons = true;
            self.charsheet.froze = true;
        }
        self.world.photo_isolation.set_capture(Some(subject));
        let s = self.settings.charsheet.clone();
        let (user_name, display_name) = self
            .world
            .social
            .avatar_names
            .get(&subject)
            .map(|n| (n.username.clone(), n.display_name.clone()))
            .unwrap_or_default();
        let height = self.charsheet_height(subject);
        // (a subject the Poser poses never got the layer: its pose is used)
        let posed = self.charsheet.pose.as_ref().is_some_and(|p| p.layer.is_some());
        let base = self.charsheet_base_folder();
        let series = output::Series {
            base,
            prefix: naming::prefix(&user_name),
            stamp: naming::local_stamp(),
            contact_sheet: s.contact_sheet,
            header: output::Header {
                avatar_id: subject,
                user_name,
                display_name,
                height_m: height,
                a_pose: posed,
                neutral_light: self.charsheet_light().is_some(),
                transparent: s.transparent,
                background: s.background,
                zoom_percent: s.zoom_percent,
                height_offset_m: s.height_offset_m,
                supersample: s.supersample,
            },
        };
        if let Some(r) = &mut self.charsheet.run {
            r.writer = Some(Writer::start(series));
        }
    }

    /// The images back from the photo job of shot `i`.
    pub(super) fn charsheet_pixels(&mut self, i: usize, size: (u32, u32), rgba: Vec<u8>) {
        let now = Instant::now();
        let Some(r) = &mut self.charsheet.run else { return };
        if r.run.phase != crate::charsheet::run::Phase::Asked(i) {
            return;
        }
        let Some(shot) = r.plan.get(i).copied() else { return };
        log::info!("charsheet: shot {}/{} drawn, {} x {}", i + 1, r.run.total, size.0, size.1);
        if let Some(writer) = &mut r.writer
            && let Err(px) = writer.image(i, shot, size.0, size.1, rgba)
        {
            r.pending = Some((i, shot, size.0, size.1, px));
        }
        let cmds = r.run.shot_done(now, i, true);
        self.charsheet_commands(cmds, now, None);
    }

    /// The size of shot `i`'s photo job.
    pub(super) fn charsheet_shot_size(&self, i: usize) -> (u32, u32) {
        self.charsheet
            .run
            .as_ref()
            .and_then(|r| r.plan.get(i))
            .map_or_else(|| self.settings.charsheet.square(), |s| s.size)
    }

    fn charsheet_stop(&mut self, stop: Stop) {
        let cmds = self.charsheet.run.as_mut().map(|r| r.run.stop(stop)).unwrap_or_default();
        self.charsheet_commands(cmds, Instant::now(), None);
    }

    /// The run ended: everything it changed back, in reverse order; the
    /// writer finishes on its own (`charsheet_run_tick`).
    fn charsheet_finish(&mut self, stop: Stop) {
        if let Some(r) = &mut self.charsheet.run {
            r.stop = Some(stop);
        }
        // a teleport moved the world: the camera is not put back there
        self.charsheet_release_camera(stop != Stop::Teleport);
        self.world.photo_isolation.set_capture(None);
        if std::mem::take(&mut self.charsheet.froze) {
            self.world.freeze_motion(None);
            self.scene.pause_all_skeletons = false;
        }
        self.charsheet.pose = None;
        log::info!("charsheet: run ended ({stop:?})");
        if stop == Stop::Completed {
            self.play_ui_sound(crate::ui_sound::UiSound::Snapshot);
        }
    }

    fn charsheet_done(&mut self, summary: output::Summary) {
        let stop = self.charsheet.run.take().and_then(|r| r.stop);
        if self.demo {
            log::info!(
                "demo charsheet: series done ({stop:?}): {} image(s), contact sheet {:?}, folder {:?}",
                summary.saved,
                summary.contact_sheet,
                summary.folder
            );
        }
        self.charsheet_report(stop, Some(summary));
    }

    fn charsheet_report(&mut self, stop: Option<Stop>, summary: Option<output::Summary>) {
        let summary = summary.unwrap_or_default();
        let folder = summary.folder.as_ref().map(|f| f.display().to_string()).unwrap_or_default();
        self.charsheet.last_folder = summary.folder.clone();
        let count = summary.saved;
        let (line, problem) = match stop {
            Some(Stop::Completed) => (aurora_i18n::tr!("charsheet-done", count = count, folder = folder.as_str()), false),
            Some(Stop::AvatarLost) => (
                aurora_i18n::tr!("charsheet-avatar-lost", count = count, folder = folder.as_str()),
                true,
            ),
            Some(Stop::Teleport) => (
                aurora_i18n::tr!("charsheet-teleport", count = count, folder = folder.as_str()),
                true,
            ),
            Some(Stop::MeasureFailed) => (aurora_i18n::tr!("charsheet-measure-failed").to_owned(), true),
            Some(Stop::WriteFailed) => {
                // (the error already shown, with the images that made it)
                if let Some((line, _)) = &self.charsheet.status {
                    (line.clone(), true)
                } else {
                    (aurora_i18n::tr!("charsheet-stopped", count = count, folder = folder.as_str()), true)
                }
            }
            _ if count == 0 => (aurora_i18n::tr!("charsheet-aborted").to_owned(), false),
            _ => (
                aurora_i18n::tr!("charsheet-stopped", count = count, folder = folder.as_str()),
                false,
            ),
        };
        self.charsheet.status = Some((line, problem));
    }

    // ------------------------------------------------------------ preview

    fn charsheet_want_pose(&mut self, subject: Uuid, now: Instant) {
        if !self.settings.charsheet.a_pose || subject.is_nil() {
            self.charsheet.pose = None;
            return;
        }
        if !self.charsheet_may_pose(subject) {
            self.charsheet.pose = None;
            self.charsheet.status = Some((aurora_i18n::tr!("charsheet-pose-not-allowed").to_owned(), false));
            return;
        }
        if self.charsheet.pose.as_ref().is_some_and(|p| p.subject == subject) {
            return;
        }
        self.charsheet.pose = Some(PoseLayer {
            subject,
            layer: None,
            since: now,
        });
    }

    /// One shot back or forward; the first press starts the preview on the
    /// first shot.
    fn charsheet_preview_step(&mut self, delta: i32, now: Instant) {
        let subject = self.charsheet_subject();
        if !self.in_world() || !self.charsheet_present(subject) {
            return;
        }
        if self.panels.snapshot {
            self.panels.snapshot = false;
        }
        let index = match &self.charsheet.preview {
            Some(p) => (p.index as i32 + delta).rem_euclid(plan::IMAGE_COUNT as i32) as usize,
            None => 0,
        };
        let started = self.charsheet.preview.is_none();
        self.charsheet_want_pose(subject, now);
        self.world.photo_isolation.set_capture(Some(subject));
        let (plan, forward) = self
            .charsheet_measure(subject)
            .map(|(m, plan)| (plan, m.forward))
            .unwrap_or_else(|| (Vec::new(), glam::Vec3::X));
        self.charsheet.preview = Some(Preview {
            index,
            plan,
            forward,
            since: if started {
                now
            } else {
                self.charsheet.preview.as_ref().map_or(now, |p| p.since)
            },
        });
        if started {
            log::info!("charsheet: preview of {subject}");
        }
    }

    /// The preview measures again while its pose settles and after a
    /// setting changed: the plan follows.
    fn charsheet_preview_follow(&mut self, now: Instant) {
        let subject = self.charsheet_subject();
        let settled = self
            .charsheet
            .pose
            .as_ref()
            .is_none_or(|p| p.layer.is_some() && now.duration_since(p.since) >= crate::charsheet::run::POSE_SETTLE);
        let Some(p) = &self.charsheet.preview else { return };
        // until the pose has settled (and once more after), the plan is
        // measured again each frame; then it stays still
        let fresh = now.duration_since(p.since) < Duration::from_millis(800);
        if (fresh || p.plan.is_empty())
            && settled
            && let Some((m, plan)) = self.charsheet_measure(subject)
            && let Some(p) = &mut self.charsheet.preview
        {
            p.plan = plan;
            p.forward = m.forward;
        }
        let Some(p) = &self.charsheet.preview else { return };
        let label = p.plan.get(p.index).map(shot_label).unwrap_or_default();
        let posed_by_poser = self.scene.poser.contains_key(&subject) && self.charsheet.pose.as_ref().is_some_and(|l| l.layer.is_none());
        let line = if posed_by_poser {
            aurora_i18n::tr!("charsheet-poser-active").to_owned()
        } else if self.settings.charsheet.neutral_light && self.world.eep.editing() {
            aurora_i18n::tr!("charsheet-light-unavailable").to_owned()
        } else {
            aurora_i18n::tr!("charsheet-preview-status", shot = label.as_str())
        };
        if self.charsheet.run.is_none() {
            self.charsheet.status = Some((line, false));
        }
    }

    fn charsheet_end_preview(&mut self) {
        if self.charsheet.preview.take().is_none() {
            return;
        }
        let running = self.charsheet.run.as_ref().is_some_and(|r| r.run.running());
        if !running {
            self.charsheet_release_camera(true);
            self.world.photo_isolation.set_capture(None);
            self.charsheet.pose = None;
            self.charsheet.status = None;
        }
        log::info!("charsheet: preview ended");
    }

    // ------------------------------------------------------------ the run

    fn charsheet_start(&mut self) {
        let subject = self.charsheet_subject();
        if !self.in_world() || !self.charsheet_present(subject) || self.charsheet.run.is_some() {
            return;
        }
        self.panels.snapshot = false;
        let now = Instant::now();
        let s = &self.settings.charsheet;
        let (light, delay) = (s.neutral_light, Duration::from_secs(u64::from(s.delay_s)));
        // the preview's pose is the run's: no new settling
        let allowed = s.a_pose && self.charsheet_may_pose(subject);
        let posed = allowed
            && self
                .charsheet
                .pose
                .as_ref()
                .is_some_and(|p| p.subject == subject && p.layer.is_some());
        let wants_pose = allowed && !posed;
        if !allowed {
            self.charsheet.pose = None;
        }
        // (the preview's camera, isolation and pose pass to the run as they are)
        self.charsheet.preview = None;
        let (run, cmds) = Run::new(now, wants_pose, light, delay);
        self.charsheet.run = Some(Running {
            run,
            subject,
            plan: Vec::new(),
            forward: glam::Vec3::X,
            writer: None,
            pending: None,
            region: self.world.main_region,
            finished: false,
            stop: None,
            lit: false,
        });
        self.charsheet.status = None;
        self.charsheet.last_folder = None;
        log::info!("charsheet: run of {subject} started");
        self.charsheet_commands(cmds, now, None);
    }

    // ------------------------------------------------------------ helpers

    /// The rest pose's body height (mBodySize.mV[VZ]): stable while posed.
    fn charsheet_height(&self, subject: Uuid) -> f32 {
        let rig = &self.world.avatar_lib.rig;
        let values = self.world.visual_params(&subject).unwrap_or(&[]);
        let shape = self.world.avatar_lib.shape_params.shape(values, rig);
        crate::scene::shape::body_size(rig, &shape.local_pos, &shape.scale).1
    }

    fn charsheet_measure(&self, subject: Uuid) -> Option<(Metrics, Vec<Shot>)> {
        let frame = crate::ik_edit::Frame::subject(&self.world, subject)?;
        let rig = &self.world.avatar_lib.rig;
        // measured and planned straight up, the seat's tilt given back to
        // the cameras (plan::Upright)
        let pivot = frame
            .joints
            .get(rig.pelvis)
            .map_or(frame.model.w_axis.truncate(), |j| j.w_axis.truncate());
        let upright = plan::Upright::new(pivot, frame.model.z_axis.truncate());
        let mut m = measure::measure(
            |n| {
                rig.names
                    .get(n)
                    .and_then(|&i| frame.joints.get(i))
                    .map(|j| upright.point(j.w_axis.truncate()))
            },
            self.charsheet_height(subject),
            upright.direction(frame.model.x_axis.truncate()),
        )?;
        let plan = plan::build(&m, &self.settings.charsheet)
            .into_iter()
            .map(|s| upright.shot(s))
            .collect();
        // the plan is in the world now: so is the facing, for the sun
        m.forward = upright.world_forward(m.forward);
        Some((m, plan))
    }

    fn charsheet_base_folder(&self) -> PathBuf {
        self.settings
            .charsheet
            .folder
            .clone()
            .unwrap_or_else(|| crate::charsheet::default_folder(&self.settings.snapshot.folder()))
    }

    fn charsheet_folder_answer(&mut self) {
        let Some(rx) = &self.charsheet.folder_pick else { return };
        match rx.try_recv() {
            Ok(chosen) => {
                self.charsheet.folder_pick = None;
                if let Some(folder) = chosen {
                    self.settings.charsheet.folder = Some(folder);
                    self.settings.save();
                }
            }
            Err(crossbeam_channel::TryRecvError::Empty) => {}
            Err(crossbeam_channel::TryRecvError::Disconnected) => self.charsheet.folder_pick = None,
        }
    }

    fn charsheet_candidates(&self) -> Vec<Candidate> {
        let own = self.world.agent_id;
        let subject = self.charsheet_subject();
        let mut list = Vec::new();
        if !own.is_nil() {
            list.push(Candidate {
                id: own,
                name: self.world.own_name(),
                you: true,
            });
        }
        let mut others: Vec<(f32, Candidate)> = self
            .world
            .objects
            .avatars()
            .map(|(_, o)| o.full_id)
            .filter(|id| *id != own && !self.world.mutes.is_muted_id(id))
            .filter(|id| *id == subject || crate::world::poser_consent::nearby(&self.world, *id))
            .map(|id| {
                let d = crate::world::poser_consent::distance(&self.world, id).unwrap_or(f32::MAX);
                (
                    d,
                    Candidate {
                        id,
                        name: crate::world::poser_consent::name(&self.world, id),
                        you: false,
                    },
                )
            })
            .collect();
        others.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));
        list.extend(others.into_iter().map(|(_, c)| c));
        list
    }

    /// The info line of the chosen avatar, measured twice a second.
    fn charsheet_info(&mut self, now: Instant) -> (Option<String>, bool) {
        let subject = self.charsheet_subject();
        if let Some((id, at, line, ok)) = &self.charsheet.info
            && *id == subject
            && now.duration_since(*at) < Duration::from_millis(500)
        {
            return (Some(line.clone()), *ok);
        }
        let (line, ok) = match self.charsheet_measure(subject) {
            Some((m, _)) => (aurora_i18n::tr!("charsheet-info", height = format!("{:.2}", m.height)), true),
            None if self.charsheet_present(subject) => (aurora_i18n::tr!("charsheet-info-incomplete").to_owned(), false),
            None => (aurora_i18n::tr!("charsheet-no-avatar").to_owned(), false),
        };
        self.charsheet.info = Some((subject, now, line.clone(), ok));
        (Some(line), ok)
    }

    /// The window, and what its buttons ask.
    pub(super) fn charsheet_window(&mut self, ctx: &egui::Context, p: &Palette) {
        if !self.charsheet.ui.open {
            return;
        }
        let now = Instant::now();
        let candidates = self.charsheet_candidates();
        let (info, measurable) = self.charsheet_info(now);
        let base = self.charsheet_base_folder();
        let running = self.charsheet.run.as_ref().is_some_and(|r| r.run.running());
        let progress = self.charsheet.run.as_ref().filter(|r| r.run.running()).map(|r| {
            (
                r.run.progress(),
                aurora_i18n::tr!("charsheet-progress", done = r.run.done, total = plan::IMAGE_COUNT),
            )
        });
        let preview = self.charsheet.preview.as_ref().map(|p| {
            let label = p.plan.get(p.index).map(shot_label).unwrap_or_default();
            aurora_i18n::tr!(
                "charsheet-preview-step",
                index = p.index + 1,
                total = plan::IMAGE_COUNT,
                shot = label.as_str()
            )
        });
        let view = View {
            candidates: &candidates,
            info,
            folder: base.display().to_string(),
            folder_is_default: self.settings.charsheet.folder.is_none(),
            preview,
            running,
            progress,
            status: self.charsheet.status.clone(),
            can_start: measurable && self.in_world() && self.charsheet.run.is_none(),
            last_folder: self.charsheet.last_folder.is_some(),
        };
        let mut settings = self.settings.charsheet.clone();
        let actions = self.charsheet.ui.show(ctx, p, &mut settings, &view);
        if settings != self.settings.charsheet {
            self.settings.charsheet = settings.sanitized();
        }
        for action in actions {
            match action {
                Action::Select(_) => {
                    self.charsheet_end_preview();
                    self.charsheet.info = None;
                    self.charsheet.status = None;
                }
                Action::Preview(delta) => self.charsheet_preview_step(delta, now),
                Action::EndPreview => self.charsheet_end_preview(),
                Action::Start => self.charsheet_start(),
                Action::Stop => self.charsheet_stop(Stop::Stopped),
                Action::Browse => {
                    if self.charsheet.folder_pick.is_none() {
                        let (tx, rx) = crossbeam_channel::bounded(1);
                        let start = base.clone();
                        let title = aurora_i18n::tr!("charsheet-folder-dialog");
                        let _ = std::thread::Builder::new().name("aurora-folder-dialog".into()).spawn(move || {
                            let _ = tx.send(rfd::FileDialog::new().set_title(title).set_directory(start).pick_folder());
                        });
                        self.charsheet.folder_pick = Some(rx);
                    }
                }
                Action::DefaultFolder => {
                    self.settings.charsheet.folder = None;
                    self.settings.save();
                }
                Action::OpenFolder => {
                    if let Some(folder) = &self.charsheet.last_folder
                        && let Err(e) = crate::open_path::open(folder)
                    {
                        log::warn!("charsheet: cannot open {}: {e}", folder.display());
                    }
                }
                Action::Changed => {
                    self.settings.save();
                    // the A-pose switched: the preview follows
                    if self.charsheet.preview.is_some() {
                        let subject = self.charsheet_subject();
                        self.charsheet_want_pose(subject, now);
                        if let Some(p) = &mut self.charsheet.preview {
                            p.since = now;
                        }
                    }
                }
                Action::Closed => self.charsheet_close(),
            }
        }
    }
}

/// Whether each shot starts its group (the first of a group waits for its
/// textures).
fn firsts(plan: &[Shot]) -> Vec<bool> {
    plan.iter()
        .enumerate()
        .map(|(i, s)| i == 0 || plan[i - 1].group != s.group)
        .collect()
}

/// A shot's label in the window ("Head · front left").
pub(super) fn shot_label(shot: &Shot) -> String {
    let group = aurora_i18n::text(naming::group_key(shot.group));
    let side = || match shot.side {
        Some(Side::Left) => aurora_i18n::tr!("charsheet-side-left"),
        Some(Side::Right) => aurora_i18n::tr!("charsheet-side-right"),
        None => "",
    };
    let view = match (shot.group, shot.view) {
        (Group::Hands, ShotView::HandPalm) => format!("{} {}", side(), aurora_i18n::tr!("charsheet-view-palm")),
        (Group::Hands, _) => format!("{} {}", side(), aurora_i18n::tr!("charsheet-view-back")),
        (Group::Feet, ShotView::Bottom) => aurora_i18n::tr!("charsheet-view-bottom").to_owned(),
        (Group::Feet, _) if shot.side.is_some() => format!("{} {}", side(), aurora_i18n::tr!("charsheet-view-outer")),
        _ => aurora_i18n::text(naming::DIRECTION_KEYS[shot.direction()]).to_owned(),
    };
    format!("{group} · {view}")
}
