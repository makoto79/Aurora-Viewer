use super::measure::{self, Metrics};
use super::naming;
use super::output;
use super::plan::{self, Group, Side, View};
use super::run::{self, Command, Input, Phase, Run, Stop};
use super::*;
use glam::Vec3;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// A standing avatar of reference height facing +X, arms out (rest pose):
/// the joints the measurement reads, in metres.
fn rest_joints() -> HashMap<&'static str, Vec3> {
    HashMap::from([
        ("mPelvis", Vec3::new(0.0, 0.0, 1.0)),
        ("mChest", Vec3::new(0.0, 0.0, 1.3)),
        ("mHead", Vec3::new(0.0, 0.0, 1.62)),
        ("mSkull", Vec3::new(0.0, 0.0, 1.72)),
        ("mElbowLeft", Vec3::new(0.0, 0.45, 1.45)),
        ("mElbowRight", Vec3::new(0.0, -0.45, 1.45)),
        ("mWristLeft", Vec3::new(0.0, 0.7, 1.45)),
        ("mWristRight", Vec3::new(0.0, -0.7, 1.45)),
        ("mHandMiddle1Left", Vec3::new(0.0, 0.8, 1.45)),
        ("mHandMiddle1Right", Vec3::new(0.0, -0.8, 1.45)),
        ("mHandMiddle3Left", Vec3::new(0.0, 0.88, 1.45)),
        ("mHandMiddle3Right", Vec3::new(0.0, -0.88, 1.45)),
        ("mHandIndex1Left", Vec3::new(0.03, 0.8, 1.45)),
        ("mHandIndex1Right", Vec3::new(0.03, -0.8, 1.45)),
        ("mHandPinky1Left", Vec3::new(-0.03, 0.79, 1.45)),
        ("mHandPinky1Right", Vec3::new(-0.03, -0.79, 1.45)),
        ("mHipLeft", Vec3::new(0.0, 0.1, 0.95)),
        ("mHipRight", Vec3::new(0.0, -0.1, 0.95)),
        ("mAnkleLeft", Vec3::new(0.0, 0.1, 0.1)),
        ("mAnkleRight", Vec3::new(0.0, -0.1, 0.1)),
        ("mFootLeft", Vec3::new(0.08, 0.1, 0.04)),
        ("mFootRight", Vec3::new(0.08, -0.1, 0.04)),
        ("mToeLeft", Vec3::new(0.16, 0.1, 0.03)),
        ("mToeRight", Vec3::new(0.16, -0.1, 0.03)),
    ])
}

fn metrics_of(joints: &HashMap<&'static str, Vec3>, height: f32, facing: Vec3) -> Option<Metrics> {
    measure::measure(|n| joints.get(n).copied(), height, facing)
}

fn rest_metrics() -> Metrics {
    metrics_of(&rest_joints(), 1.8, Vec3::X).expect("measured")
}

#[test]
fn charsheet_measure_rest_avatar() {
    let m = rest_metrics();
    assert!((m.scale - 1.0).abs() < 1e-6);
    // crown from the √2 skull estimate, sole below the lowest foot joint
    assert!((m.head_top_z - (1.62 + std::f32::consts::SQRT_2 * 0.10)).abs() < 1e-5);
    // the lowest of the ankles and mFoot (0.04), less ANKLE_TO_SOLE
    assert!((m.sole_z - (0.04 - measure::ANKLE_TO_SOLE)).abs() < 1e-5);
    assert!((m.height - (m.head_top_z - m.sole_z)).abs() < 1e-6);
    // hand centre half way to the middle fingertip, normal up out of the back
    assert!(m.hand_center[0].distance(Vec3::new(0.0, 0.79, 1.45)) < 1e-5);
    assert!(m.hand_normal[0].dot(Vec3::Z) > 0.99, "{:?}", m.hand_normal[0]);
    assert!(m.hand_normal[1].dot(Vec3::Z) > 0.99, "{:?}", m.hand_normal[1]);
    assert!(m.hand_axis[0].dot(Vec3::Y) > 0.99);
    assert!(m.hand_axis[1].dot(Vec3::NEG_Y) > 0.99);
    assert!((m.skull_center.z - (1.62 + m.head_top_z) * 0.5).abs() < 1e-6);
    assert_eq!(m.forward, Vec3::X);
}

#[test]
fn charsheet_measure_fallbacks_and_failures() {
    let mut j = rest_joints();
    for n in ["mSkull", "mHandMiddle3Left", "mHandMiddle1Left", "mToeLeft"] {
        j.remove(n);
    }
    let m = metrics_of(&j, 0.0, Vec3::new(1.0, 1.0, 0.5)).expect("measured without the optional joints");
    // unknown height: scale 1, fixed crown offset, hand past the wrist
    assert_eq!(m.scale, 1.0);
    assert!((m.head_top_z - (1.62 + measure::HEAD_TO_CROWN)).abs() < 1e-6);
    assert!(m.hand_center[0].distance(Vec3::new(0.0, 0.7 + measure::HAND_CENTER_OFFSET, 1.45)) < 1e-5);
    assert!(m.hand_normal[0].dot(Vec3::Z) > 0.99);
    // facing flattened and normalised
    assert!((m.forward - Vec3::new(1.0, 1.0, 0.0).normalize()).length() < 1e-6);
    // a missing required joint, or a skeleton folded to nothing
    let mut missing = rest_joints();
    missing.remove("mAnkleRight");
    assert!(metrics_of(&missing, 1.8, Vec3::X).is_none());
    let flat: HashMap<_, _> = rest_joints().into_iter().map(|(k, v)| (k, Vec3::new(v.x, v.y, 0.1))).collect();
    assert!(metrics_of(&flat, 1.8, Vec3::X).is_none());
    // half size: half scale
    let half = metrics_of(&rest_joints(), 0.9, Vec3::X).expect("measured");
    assert!((half.scale - 0.5).abs() < 1e-6);
}

#[test]
fn charsheet_plan_order_and_groups() {
    let shots = plan::build(&rest_metrics(), &Settings::default());
    assert_eq!(shots.len(), plan::IMAGE_COUNT);
    let groups: Vec<Group> = shots.iter().map(|s| s.group).collect();
    assert!(groups[..6].iter().all(|g| *g == Group::Body));
    assert!(groups[6..12].iter().all(|g| *g == Group::Head));
    assert!(groups[12..16].iter().all(|g| *g == Group::Hands));
    assert!(groups[16..].iter().all(|g| *g == Group::Feet));
    let names: Vec<String> = shots.iter().enumerate().map(|(i, s)| naming::file_name("jane.doe", i, s)).collect();
    assert_eq!(names[0], "jane.doe_01_body_front.png");
    assert_eq!(names[1], "jane.doe_02_body_front-left.png");
    assert_eq!(names[2], "jane.doe_03_body_left.png");
    assert_eq!(names[3], "jane.doe_04_body_back.png");
    assert_eq!(names[4], "jane.doe_05_body_right.png");
    assert_eq!(names[5], "jane.doe_06_body_front-right.png");
    assert_eq!(names[6], "jane.doe_07_head_front.png");
    assert_eq!(names[12], "jane.doe_13_hand-left_back.png");
    assert_eq!(names[13], "jane.doe_14_hand-left_palm.png");
    assert_eq!(names[14], "jane.doe_15_hand-right_back.png");
    assert_eq!(names[15], "jane.doe_16_hand-right_palm.png");
    assert_eq!(names[16], "jane.doe_17_feet_front.png");
    assert_eq!(names[17], "jane.doe_18_foot-left_outer.png");
    assert_eq!(names[18], "jane.doe_19_foot-right_outer.png");
    assert_eq!(names[19], "jane.doe_20_feet_bottom.png");
}

#[test]
fn charsheet_plan_cameras() {
    let m = rest_metrics();
    let shots = plan::build(&m, &Settings::default());
    // positive azimuth: the camera on the avatar's left (+Y)
    assert!(shots[2].eye.y > shots[2].target.y + 1.0);
    assert!(shots[4].eye.y < shots[4].target.y - 1.0);
    assert!(shots[0].eye.x > shots[0].target.x + 1.0);
    assert!(shots[3].eye.x < shots[3].target.x - 1.0);
    // every camera looks at its target from its distance
    for s in &shots {
        assert!((s.eye.distance(s.target) - s.distance).abs() < 1e-3, "{s:?}");
        assert!(s.up.is_normalized());
        // the camera looks at the target, its up square to the view and on
        // the side of the shot's up (Z for the orbits, raised or not)
        let cam = s.camera();
        let view = (s.target - s.eye).normalize();
        assert!(cam.forward().dot(view) > 0.9999, "{s:?}");
        assert!(cam.up().dot(cam.forward()).abs() < 1e-4);
        assert!(cam.up().dot(s.up) > 0.9, "{s:?}");
    }
    // head shots from slightly above
    assert!(shots[6].eye.z > shots[6].target.z);
    // the soles from straight below, the toes (forward) up in the image
    let soles = shots[19];
    assert_eq!(soles.view, View::Bottom);
    assert!(soles.eye.z < soles.target.z - 0.5);
    assert!(soles.up.dot(m.forward) > 0.99);
    assert!((soles.target.z - m.sole_z).abs() < 1e-6);
    // hand back from above the back of the hand, palm from below, fingers up
    let (back, palm) = (shots[12], shots[13]);
    assert_eq!((back.view, back.side), (View::HandBack, Some(Side::Left)));
    assert!(back.eye.z > back.target.z && palm.eye.z < palm.target.z);
    assert!(back.up.dot(Vec3::Y) > 0.99);
    // feet outer views on the side of the foot
    assert!(shots[17].eye.y > shots[17].target.y && shots[18].eye.y < shots[18].target.y);
}

/// The app's path: joints taken into the upright frame, measured, planned,
/// the shots given back to the world.
fn upright_plan(joints: &HashMap<&'static str, Vec3>, upright: &plan::Upright, facing: Vec3) -> (Metrics, Vec<plan::Shot>) {
    let j = |n: &str| joints.get(n).map(|p| upright.point(*p));
    let m = measure::measure(j, 1.8, upright.direction(facing)).expect("measured");
    let shots = plan::build(&m, &Settings::default()).into_iter().map(|s| upright.shot(s)).collect();
    (m, shots)
}

#[test]
fn charsheet_upright_standing_is_unchanged() {
    let up = plan::Upright::new(Vec3::new(0.0, 0.0, 1.0), Vec3::Z * 2.0);
    assert!(up.tilt.angle_between(glam::Quat::IDENTITY) < 1e-6);
    let (m, shots) = upright_plan(&rest_joints(), &up, Vec3::X);
    let plain = plan::build(&rest_metrics(), &Settings::default());
    let rest = rest_metrics();
    assert!((m.height - rest.height).abs() < 1e-5 && m.pelvis.distance(rest.pelvis) < 1e-5 && m.forward.distance(rest.forward) < 1e-6);
    for (a, b) in shots.iter().zip(&plain) {
        assert!(a.eye.distance(b.eye) < 1e-5 && a.target.distance(b.target) < 1e-5 && a.up.distance(b.up) < 1e-5);
    }
    // a degenerate up keeps the world's
    assert_eq!(plan::Upright::new(Vec3::ZERO, Vec3::ZERO).tilt, glam::Quat::IDENTITY);
}

#[test]
fn charsheet_upright_tilted_seat() {
    // the avatar on a seat pitched forward 25° and rolled 10°, turned 30°:
    // the whole body tilted rigidly about its pelvis
    let pelvis = Vec3::new(5.0, 3.0, 21.0);
    let yaw = glam::Quat::from_rotation_z(30f32.to_radians());
    let seat = glam::Quat::from_rotation_y(25f32.to_radians()) * glam::Quat::from_rotation_x(10f32.to_radians());
    let model = seat * yaw;
    let rest_pelvis = rest_joints()["mPelvis"];
    let joints: HashMap<_, _> = rest_joints()
        .into_iter()
        .map(|(n, p)| (n, pelvis + model * (p - rest_pelvis)))
        .collect();
    let up = plan::Upright::new(pelvis, model * Vec3::Z);
    let (m, shots) = upright_plan(&joints, &up, model * Vec3::X);
    // measured as if it stood straight: the rest avatar, only turned
    let rest = rest_metrics();
    assert!((m.height - rest.height).abs() < 1e-4, "{} vs {}", m.height, rest.height);
    let turn = up.tilt.inverse() * model;
    assert!((turn * Vec3::Z).distance(Vec3::Z) < 1e-5, "the upright frame keeps only a turn");
    assert!(m.forward.distance(turn * Vec3::X) < 1e-4, "{:?}", m.forward);
    // each camera sees the avatar exactly as the rest avatar's camera sees
    // it: the same eye, target and up in the avatar's own frame
    let plain = plan::build(&rest, &Settings::default());
    let local = |p: Vec3| model.inverse() * (p - pelvis) + rest_pelvis;
    for (s, r) in shots.iter().zip(&plain) {
        assert!(local(s.eye).distance(r.eye) < 1e-3, "{s:?}");
        assert!(local(s.target).distance(r.target) < 1e-3, "{s:?}");
        assert!((model.inverse() * s.up).distance(r.up) < 1e-3, "{s:?}");
    }
    // the body shots' up is the avatar's up, not the world's
    assert!(shots[0].up.dot(model * Vec3::Z) > 0.999);
    assert!(shots[0].up.dot(Vec3::Z) < 0.95);
    // the sun's facing is flat in the world
    let sun = up.world_forward(m.forward);
    assert!(sun.z.abs() < 1e-6 && sun.is_normalized());
}

#[test]
fn charsheet_plan_framing_settings() {
    let m = rest_metrics();
    let base = plan::build(&m, &Settings::default());
    // the vertical fit: frame height × margin fills the field of view
    let fov = plan::FOV_BODY_DEG.to_radians();
    let frame = plan::FRAME_BODY * m.height * 1.1;
    assert!((base[0].distance - frame * 0.5 / (fov * 0.5).tan()).abs() < 1e-3);
    // zoom 200 %: half the framed height, half the distance
    let zoomed = plan::build(
        &m,
        &Settings {
            zoom_percent: 200.0,
            ..Default::default()
        },
    );
    assert!((zoomed[0].distance * 2.0 - base[0].distance).abs() < 1e-3);
    // the height offset moves the body shots only
    let raised = plan::build(
        &m,
        &Settings {
            height_offset_m: 0.25,
            ..Default::default()
        },
    );
    assert!((raised[0].target.z - base[0].target.z - 0.25).abs() < 1e-5);
    assert_eq!(raised[6].target, base[6].target);
    // portrait: its own size; the narrower width ratio keeps the vertical fit
    let portrait = plan::build(
        &m,
        &Settings {
            portrait_body: true,
            ..Default::default()
        },
    );
    assert_eq!(portrait[0].size, (768, 1344));
    assert_eq!(portrait[6].size, (1024, 1024));
    assert!((portrait[0].distance - base[0].distance).abs() < 1e-3);
    // a wide subject in a narrow frame is fitted by its width
    let d = plan::distance_for_frame_height(1.0, 0.5, 0.0, 0.25, 1.0);
    assert!(d > plan::distance_for_frame_height(1.0, 0.5, 0.0, 1.0, 1.0) * 3.9);
    assert_eq!(plan::distance_for_frame_height(0.0, 0.5, 0.0, 1.0, 1.0), plan::MIN_DISTANCE);
}

#[test]
fn charsheet_naming() {
    assert_eq!(naming::prefix("jane.doe"), "jane.doe");
    assert_eq!(naming::prefix("  a:b/c "), "a_b_c");
    assert_eq!(naming::prefix("two words"), "two_words");
    assert_eq!(naming::prefix(""), "avatar");
    assert_eq!(naming::prefix("..."), "avatar");
    let base = std::path::Path::new("base");
    let taken = [base.join("p_2026-01-02_0304"), base.join("p_2026-01-02_0304_2")];
    let f = naming::series_folder(base, "p", "2026-01-02_0304", |p| taken.iter().any(|t| t == p));
    assert_eq!(f, base.join("p_2026-01-02_0304_3"));
    let free = naming::series_folder(base, "q", "2026-01-02_0304", |_| false);
    assert_eq!(free, base.join("q_2026-01-02_0304"));
    assert_eq!(naming::contact_sheet_name("p"), "p_sheet.png");
    assert_eq!(naming::local_stamp().len(), "2026-01-02_0304".len());
}

#[test]
fn charsheet_settings_sanitized() {
    let s = Settings {
        zoom_percent: f32::NAN,
        height_offset_m: 9.0,
        background: [2.0, -1.0, f32::INFINITY],
        delay_s: 999,
        folder: Some(std::path::PathBuf::new()),
        ..Default::default()
    }
    .sanitized();
    assert_eq!(s.zoom_percent, 100.0);
    assert_eq!(s.height_offset_m, 1.0);
    assert_eq!(s.background, [1.0, 0.0, 0.5]);
    assert_eq!(s.delay_s, DELAY_MAX_S);
    assert_eq!(s.folder, None);
    // a settings file from before this feature: the defaults
    let old: Settings = serde_json::from_str("{}").expect("parsed");
    assert_eq!(old, Settings::default());
    let partial: Settings = serde_json::from_str(r#"{"transparent":true}"#).expect("parsed");
    assert!(partial.transparent && partial.a_pose && partial.contact_sheet);
    assert_eq!(Settings::default().portrait(), (768, 1344));
}

#[test]
fn charsheet_output_composite_and_sheet() {
    let mut px = vec![200, 100, 0, 255, 200, 100, 0, 0, 200, 100, 0, 128];
    output::over_background(&mut px, [0.0, 1.0, 0.0]);
    assert_eq!(&px[0..4], &[200, 100, 0, 255]);
    assert_eq!(&px[4..8], &[0, 255, 0, 255]);
    assert_eq!(px[11], 255);
    assert!((i32::from(px[8]) - 100).abs() <= 1 && (i32::from(px[9]) - 177).abs() <= 1);

    let tile = |g, w: u32| {
        (
            g,
            w,
            output::CONTACT_TILE_HEIGHT,
            vec![9u8; (w * output::CONTACT_TILE_HEIGHT * 4) as usize],
        )
    };
    let tiles = vec![tile(Group::Head, 10), tile(Group::Body, 20), tile(Group::Body, 20)];
    let (w, h, sheet) = output::contact_sheet(&tiles, [1.0, 0.0, 0.0], true);
    let gap = output::CONTACT_GAP;
    assert_eq!(w, gap + 2 * (20 + gap));
    assert_eq!(h, gap + 2 * (output::CONTACT_TILE_HEIGHT + gap));
    assert_eq!(sheet.len(), (w * h * 4) as usize);
    // the corner is the transparent fill, the first tile the body row's
    assert_eq!(&sheet[0..4], &[255, 0, 0, 0]);
    let at = |x: u32, y: u32| &sheet[((y * w + x) * 4) as usize..((y * w + x) * 4 + 4) as usize];
    assert_eq!(at(gap, gap), &[9, 9, 9, 9]);
    // the head row is second and narrower: its right part is the fill
    let head_y = gap * 2 + output::CONTACT_TILE_HEIGHT;
    assert_eq!(at(gap, head_y), &[9, 9, 9, 9]);
    assert_eq!(at(gap + 15, head_y), &[255, 0, 0, 0]);
}

#[test]
fn charsheet_output_writes_a_series() {
    let dir = std::env::temp_dir().join(format!("aurora-charsheet-test-{}", uuid::Uuid::new_v4()));
    let shots = plan::build(&rest_metrics(), &Settings::default());
    let header = output::Header {
        avatar_id: uuid::Uuid::nil(),
        user_name: "jane.doe".into(),
        display_name: "Jane".into(),
        height_m: 1.8,
        a_pose: true,
        neutral_light: true,
        transparent: false,
        background: [0.5, 0.5, 0.5],
        zoom_percent: 100.0,
        height_offset_m: 0.0,
        supersample: false,
    };
    let mut writer = output::Writer::start(output::Series {
        base: dir.clone(),
        prefix: "jane.doe".into(),
        stamp: "2026-01-02_0304".into(),
        contact_sheet: true,
        header,
    });
    for (i, shot) in shots.iter().take(2).enumerate() {
        let mut px = vec![0u8; 8 * 8 * 4];
        loop {
            match writer.image(i, *shot, 8, 8, px) {
                Ok(()) => break,
                Err(back) => {
                    px = back;
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }
    writer.finish();
    let mut events = Vec::new();
    for _ in 0..400 {
        events.extend(writer.poll());
        if events.iter().any(|e| matches!(e, output::Event::Done(_))) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let Some(output::Event::Done(summary)) = events.last().cloned() else {
        panic!("no end: {events:?}");
    };
    let folder = summary.folder.clone().expect("a folder");
    assert_eq!(summary.saved, 2);
    assert_eq!(folder, dir.join("jane.doe_2026-01-02_0304"));
    assert!(folder.join("jane.doe_01_body_front.png").exists());
    assert!(folder.join("jane.doe_sheet.png").exists());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(folder.join(naming::MANIFEST_NAME)).expect("manifest")).expect("json");
    assert_eq!(manifest["image_count"], 2);
    assert_eq!(manifest["shots"][1]["view"], "front-left");
    assert_eq!(manifest["contact_sheet"], "jane.doe_sheet.png");
    assert!(!manifest.to_string().contains("asset"));
    let leftovers: Vec<_> = std::fs::read_dir(&folder)
        .expect("listed")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "part"))
        .collect();
    assert!(leftovers.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn charsheet_writer_without_images_makes_no_folder() {
    let dir = std::env::temp_dir().join(format!("aurora-charsheet-empty-{}", uuid::Uuid::new_v4()));
    let mut writer = output::Writer::start(output::Series {
        base: dir.clone(),
        prefix: "p".into(),
        stamp: "s".into(),
        contact_sheet: true,
        header: output::Header {
            avatar_id: uuid::Uuid::nil(),
            user_name: String::new(),
            display_name: String::new(),
            height_m: 0.0,
            a_pose: false,
            neutral_light: false,
            transparent: true,
            background: [0.0; 3],
            zoom_percent: 100.0,
            height_offset_m: 0.0,
            supersample: false,
        },
    });
    writer.finish();
    let mut done = None;
    for _ in 0..400 {
        if let Some(output::Event::Done(s)) = writer.poll().into_iter().last() {
            done = Some(s);
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(done, Some(output::Summary::default()));
    assert!(!dir.exists());
}

fn idle() -> Input {
    Input {
        textures_pending: 0,
        light_ready: true,
        photo_idle: true,
    }
}

#[test]
fn charsheet_run_full_series() {
    let t0 = Instant::now();
    let at = |ms: u64| t0 + Duration::from_millis(ms);
    let (mut run, cmds) = Run::new(t0, true, true, Duration::from_secs(0));
    assert_eq!(cmds, vec![Command::Pose]);
    // the pose settles before the measurement
    assert!(run.tick(at(100), idle()).is_empty());
    assert!(run.tick(at(200), idle()).is_empty());
    assert_eq!(run.tick(at(320), idle()), vec![Command::Measure]);
    let firsts: Vec<bool> = (0..4).map(|i| i % 2 == 0).collect();
    assert_eq!(run.measured(at(320), Some(firsts)), vec![Command::Light]);
    // neutral light: at least LIGHT_SETTLE despite a zero delay
    assert_eq!(run.countdown(at(320)), Some(2));
    assert!(run.tick(at(1000), idle()).is_empty());
    assert_eq!(run.tick(at(2400), idle()), vec![Command::Prepare]);
    assert!(run.prepared());
    assert!(run.tick(at(3000), idle()).is_empty());
    // settle done, but the sky is not loaded yet
    let unlit = Input {
        light_ready: false,
        ..idle()
    };
    assert!(run.tick(at(4000), unlit).is_empty());
    assert_eq!(run.tick(at(4100), idle()), vec![Command::Camera(0)]);
    let mut now = 4100;
    for shot in 0..4 {
        // a group's first shot waits for its textures (up to TEXTURE_WAIT)
        let busy = Input {
            textures_pending: 5,
            ..idle()
        };
        let mut asked = Vec::new();
        for _ in 0..10 {
            now += 50;
            asked = run.tick(at(now), if shot == 0 && now < 4500 { busy } else { idle() });
            if !asked.is_empty() {
                break;
            }
        }
        assert_eq!(asked, vec![Command::Ask(shot)], "shot {shot}");
        let next = run.shot_done(at(now), shot, shot != 2);
        if shot < 3 {
            assert_eq!(next, vec![Command::Camera(shot + 1)]);
        } else {
            assert_eq!(next, vec![Command::Finish(Stop::Completed)]);
        }
    }
    assert_eq!((run.done, run.failed), (4, 1));
    assert!(!run.running());
    assert_eq!(run.progress(), 1.0);
    // a stop after the end does nothing
    assert!(run.stop(Stop::Stopped).is_empty());
}

#[test]
fn charsheet_run_stops_and_failures() {
    let t0 = Instant::now();
    let (mut run, cmds) = Run::new(t0, false, false, Duration::from_secs(3));
    assert_eq!(cmds, vec![Command::Measure]);
    // without a pose there is nothing to settle; a failed measurement ends it
    assert_eq!(run.measured(t0, None), vec![Command::Finish(Stop::MeasureFailed)]);
    let (mut run, _) = Run::new(t0, false, false, Duration::from_secs(3));
    assert!(run.measured(t0, Some(vec![true, false])).is_empty());
    assert_eq!(run.countdown(t0), Some(3));
    assert_eq!(run.stop(Stop::Teleport), vec![Command::Finish(Stop::Teleport)]);
    assert_eq!(run.phase, Phase::Done(Stop::Teleport));
    // an answer for a shot not asked is ignored
    let (mut run, _) = Run::new(t0, false, false, Duration::ZERO);
    run.measured(t0, Some(vec![true]));
    assert!(run.shot_done(t0, 0, true).is_empty());
    // a photo job of someone else holds the shot back
    run.tick(t0, idle());
    run.tick(t0 + run::CAPTURE_SETTLE, idle());
    let busy = Input {
        photo_idle: false,
        ..idle()
    };
    for i in 0..5 {
        assert!(run.tick(t0 + run::CAPTURE_SETTLE + Duration::from_millis(200 + i), busy).is_empty());
    }
    assert_eq!(
        run.tick(t0 + run::CAPTURE_SETTLE + Duration::from_millis(300), idle()),
        vec![Command::Ask(0)]
    );
}

mod rig {
    use super::*;
    use crate::charsheet::pose;
    use crate::ik_edit::Frame;
    use crate::scene::avatar::AvatarLibrary;
    use crate::world::World;
    use std::sync::Arc;

    fn world() -> World {
        let lib = Arc::new(AvatarLibrary::load());
        let mut world = World::new(lib.clone());
        world.agent_id = crate::demo::DEMO_AGENT;
        world.main_region = Some(1);
        world.apply(aurora_net::NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![crate::demo::action_avatar(false)],
        });
        let rig = &lib.rig;
        world.avatar_poses.insert(
            world.agent_id,
            rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale)
                .iter()
                .zip(&rig.default_world_inv)
                .map(|(p, i)| *p * *i)
                .collect(),
        );
        world
    }

    #[test]
    fn charsheet_a_pose_lowers_the_arms_and_spreads_the_legs() {
        let world = world();
        let rig = &world.avatar_lib.rig;
        let frame = Frame::subject(&world, world.agent_id).expect("frame");
        let captured = crate::ik_edit::poser::SubjectPose::capture(rig, &frame).expect("captured");
        let posed = pose::a_pose(rig, &captured).expect("a pose");
        assert!(posed.valid(rig));
        let joints = rig.world_with(&posed.rotations, Vec3::ZERO, &posed.positions, &posed.scales);
        let at = |n: &str| joints[rig.names[n]].w_axis.truncate();
        for (side, sign) in [("Left", 1.0), ("Right", -1.0)] {
            let shoulder = at(&format!("mShoulder{side}"));
            let wrist = at(&format!("mWrist{side}"));
            // below the shoulder, still on its own side, about 40° down
            assert!(wrist.z < shoulder.z - 0.1, "{side}: {wrist} vs {shoulder}");
            assert!((wrist.y - shoulder.y) * sign > 0.1, "{side}");
            let arm = (wrist - shoulder).normalize();
            let drop = (-arm.z).asin().to_degrees();
            assert!((drop - pose::SHOULDER_DROP_DEG).abs() < 8.0, "{side}: {drop}");
            // the leg spread out, the sole flat again
            let hip = at(&format!("mHip{side}"));
            let ankle = at(&format!("mAnkle{side}"));
            assert!((ankle.y - hip.y) * sign > 0.0, "{side}: {ankle} vs {hip}");
            let foot_q = joints[rig.names[&*format!("mAnkle{side}")]].to_scale_rotation_translation().1;
            let rest_q = rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale)[rig.names[&*format!("mAnkle{side}")]]
                .to_scale_rotation_translation()
                .1;
            assert!((foot_q * Vec3::Z).dot(rest_q * Vec3::Z) > 0.999, "{side}");
        }
        // the pelvis at the skeleton's rest position
        assert_eq!(posed.positions[rig.pelvis], rig.local_pos[rig.pelvis]);
    }

    #[test]
    fn charsheet_a_pose_keeps_tails_wings_and_hind_limbs() {
        let world = world();
        let rig = &world.avatar_lib.rig;
        let frame = Frame::subject(&world, world.agent_id).expect("frame");
        let mut captured = crate::ik_edit::poser::SubjectPose::capture(rig, &frame).expect("captured");
        let tail = rig.names["mTail1"];
        let wing = rig.names["mWing1Left"];
        let bent = glam::Quat::from_rotation_y(0.7);
        captured.rotations[tail] = bent;
        captured.rotations[wing] = bent;
        let head = rig.names["mHead"];
        captured.rotations[head] = bent;
        let posed = pose::a_pose(rig, &captured).expect("a pose");
        assert_eq!(posed.rotations[tail], bent);
        assert_eq!(posed.rotations[wing], bent);
        assert_eq!(posed.rotations[head], rig.local_rot[head]);
    }

    #[test]
    fn charsheet_measure_on_the_demo_skeleton() {
        let world = world();
        let rig = &world.avatar_lib.rig;
        let frame = Frame::subject(&world, world.agent_id).expect("frame");
        let m = measure::measure(
            |n| rig.names.get(n).map(|&i| frame.joints[i].w_axis.truncate()),
            1.8,
            frame.model.x_axis.truncate(),
        )
        .expect("measured");
        assert!((1.4..2.3).contains(&m.height), "{}", m.height);
        assert_eq!(plan::build(&m, &Settings::default()).len(), plan::IMAGE_COUNT);
    }
}
