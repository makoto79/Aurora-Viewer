//! A series written on its own worker thread, so the frame loop never waits
//! on a PNG: each cut-out image laid over the backdrop colour (or kept with
//! its alpha), written beside its final name and renamed (`snapshot::save`),
//! a tile kept for the contact sheet; at the end the contact sheet and the
//! manifest. Port of the fork's LLFloaterAvatarSheet::onShotCaptured,
//! addContactTile, writeContactSheet and writeManifest (originally LGPL 2.1).
//!
//! The fork drew each transparent shot twice over two backdrops and took the
//! alpha from their difference; Aurora's cut-out frame gives the coverage
//! directly (`aurora_render::cutout`), so one image per shot is enough.

use super::naming;
use super::plan::{Group, Shot};
use crate::snapshot::{FileFormat, save};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU32;
use uuid::Uuid;

/// Contact sheet tiles: this tall, this gap around them.
pub const CONTACT_TILE_HEIGHT: u32 = 320;
pub const CONTACT_GAP: u32 = 8;

/// What the manifest says of the avatar and the setup.
#[derive(Clone, Debug)]
pub struct Header {
    pub avatar_id: Uuid,
    pub user_name: String,
    pub display_name: String,
    pub height_m: f32,
    pub a_pose: bool,
    pub neutral_light: bool,
    pub transparent: bool,
    /// sRGB 0..1.
    pub background: [f32; 3],
    pub zoom_percent: f32,
    pub height_offset_m: f32,
    pub supersample: bool,
}

#[derive(Clone, Debug)]
pub struct Series {
    /// The base folder; the series gets its own folder in it.
    pub base: PathBuf,
    pub prefix: String,
    pub stamp: String,
    pub contact_sheet: bool,
    pub header: Header,
}

enum Msg {
    Image {
        index: usize,
        shot: Shot,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    Finish,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Saved {
        index: usize,
    },
    /// The series cannot go on (folder, disk): the run stops.
    Failed(String),
    Done(Summary),
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Summary {
    /// None: nothing was written, no folder was made.
    pub folder: Option<PathBuf>,
    pub saved: usize,
    pub contact_sheet: Option<String>,
}

/// The worker of one series.
pub struct Writer {
    tx: Option<crossbeam_channel::Sender<Msg>>,
    rx: crossbeam_channel::Receiver<Event>,
}

impl Writer {
    pub fn start(series: Series) -> Writer {
        // at most two full images waiting (~32 MB at 2048²): the run asks its
        // next shot meanwhile, never waits on the disk
        let (tx, worker_rx) = crossbeam_channel::bounded::<Msg>(2);
        let (worker_tx, rx) = crossbeam_channel::unbounded();
        let spawned = std::thread::Builder::new().name("aurora-charsheet".into()).spawn(move || {
            let mut state = State::new(series);
            for msg in worker_rx {
                match msg {
                    Msg::Image {
                        index,
                        shot,
                        width,
                        height,
                        rgba,
                    } => {
                        if state.failed {
                            continue;
                        }
                        match state.image(index, &shot, width, height, rgba) {
                            Ok(()) => {
                                let _ = worker_tx.send(Event::Saved { index });
                            }
                            Err(e) => {
                                log::warn!("charsheet: {e}");
                                state.failed = true;
                                let _ = worker_tx.send(Event::Failed(e));
                            }
                        }
                    }
                    Msg::Finish => break,
                }
            }
            let _ = worker_tx.send(Event::Done(state.finish()));
        });
        if let Err(e) = spawned {
            log::warn!("charsheet: cannot start the writer thread: {e}");
            let (dead_tx, rx) = crossbeam_channel::bounded(1);
            let _ = dead_tx.send(Event::Failed(e.to_string()));
            return Writer { tx: None, rx };
        }
        Writer { tx: Some(tx), rx }
    }

    /// Hand an image over. False when the queue is full: the caller keeps
    /// it and tries again next frame (the frame loop never blocks).
    pub fn image(&mut self, index: usize, shot: Shot, width: u32, height: u32, rgba: Vec<u8>) -> Result<(), Vec<u8>> {
        let Some(tx) = &self.tx else { return Ok(()) };
        match tx.try_send(Msg::Image {
            index,
            shot,
            width,
            height,
            rgba,
        }) {
            Ok(()) => Ok(()),
            Err(crossbeam_channel::TrySendError::Full(Msg::Image { rgba, .. })) => Err(rgba),
            Err(_) => Ok(()),
        }
    }

    /// No more images: the contact sheet and the manifest follow.
    pub fn finish(&mut self) {
        if let Some(tx) = self.tx.take() {
            // the queue may be full: the worker drains it before the end,
            // so a blocking send here waits at most two image writes
            let _ = tx.send(Msg::Finish);
        }
    }

    pub fn poll(&self) -> Vec<Event> {
        self.rx.try_iter().collect()
    }
}

struct State {
    series: Series,
    folder: Option<PathBuf>,
    failed: bool,
    saved: usize,
    tiles: Vec<(Group, u32, u32, Vec<u8>)>,
    shots: Vec<serde_json::Value>,
}

impl State {
    fn new(series: Series) -> Self {
        Self {
            series,
            folder: None,
            failed: false,
            saved: 0,
            tiles: Vec::new(),
            shots: Vec::new(),
        }
    }

    fn folder(&mut self) -> Result<PathBuf, String> {
        if let Some(f) = &self.folder {
            return Ok(f.clone());
        }
        let s = &self.series;
        std::fs::create_dir_all(&s.base).map_err(|e| format!("{}: {e}", s.base.display()))?;
        let folder = naming::series_folder(&s.base, &s.prefix, &s.stamp, Path::exists);
        std::fs::create_dir(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        log::info!("charsheet: series folder {}", folder.display());
        self.folder = Some(folder.clone());
        Ok(folder)
    }

    fn image(&mut self, index: usize, shot: &Shot, width: u32, height: u32, mut rgba: Vec<u8>) -> Result<(), String> {
        if rgba.len() != width as usize * height as usize * 4 {
            return Err(format!("image {}: {} bytes for {width} x {height}", index + 1, rgba.len()));
        }
        let folder = self.folder()?;
        let transparent = self.series.header.transparent;
        if !transparent {
            over_background(&mut rgba, self.series.header.background);
        }
        if self.series.contact_sheet && height > 0 {
            let side = (CONTACT_TILE_HEIGHT as u64 * u64::from(width.max(height)) / u64::from(height)) as u32;
            let (tw, th, tile) = save::downscale(width, height, &rgba, side.max(1));
            self.tiles.push((shot.group, tw, th, tile));
        }
        let name = naming::file_name(&self.series.prefix, index, shot);
        save::save(
            save::Job {
                width,
                height,
                rgba,
                format: FileFormat::Png,
                jpeg_quality: 90,
                path: folder.join(&name),
                alpha: transparent,
            },
            &AtomicU32::new(0),
        )?;
        let (group, view) = naming::tokens(shot);
        self.shots.push(json!({
            "file": name,
            "group": group,
            "view": view,
            "azimuth_deg": shot.azimuth_deg,
            "elevation_deg": shot.elevation_deg,
            "distance_m": shot.distance,
            "fov_deg": shot.fov.to_degrees(),
            "width": width,
            "height": height,
        }));
        self.saved += 1;
        Ok(())
    }

    fn finish(self) -> Summary {
        let Some(folder) = self.folder.clone() else {
            return Summary::default();
        };
        let mut contact = None;
        if self.series.contact_sheet && !self.tiles.is_empty() {
            let h = &self.series.header;
            let (w, hgt, px) = contact_sheet(&self.tiles, h.background, h.transparent);
            let name = naming::contact_sheet_name(&self.series.prefix);
            // (opaque runs: every pixel is already opaque, the PNG drops alpha)
            let written = save::save(
                save::Job {
                    width: w,
                    height: hgt,
                    rgba: px,
                    format: FileFormat::Png,
                    jpeg_quality: 90,
                    path: folder.join(&name),
                    alpha: h.transparent,
                },
                &AtomicU32::new(0),
            );
            match written {
                Ok(_) => contact = Some(name),
                Err(e) => log::warn!("charsheet: contact sheet: {e}"),
            }
        }
        let manifest = manifest(&self.series.header, &self.shots, contact.as_deref());
        let path = folder.join(naming::MANIFEST_NAME);
        let text = serde_json::to_string_pretty(&manifest).unwrap_or_default();
        if let Err(e) = crate::atomic_file::write(&path, format!("{text}\n").as_bytes()) {
            log::warn!("charsheet: {}: {e}", path.display());
        }
        log::info!("charsheet: {} image(s) written to {}", self.saved, folder.display());
        Summary {
            folder: Some(folder),
            saved: self.saved,
            contact_sheet: contact,
        }
    }
}

/// Straight-alpha RGBA laid over an opaque colour, in sRGB as the fork's
/// backdrop was: c·a + bg·(1 − a), alpha 255.
pub fn over_background(rgba: &mut [u8], background: [f32; 3]) {
    let bg = background.map(|c| (c.clamp(0.0, 1.0) * 255.0).round());
    for px in rgba.as_chunks_mut::<4>().0 {
        let a = f32::from(px[3]) / 255.0;
        for c in 0..3 {
            px[c] = (f32::from(px[c]) * a + bg[c] * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
        }
        px[3] = 255;
    }
}

/// One row per group in the plan's order, each as wide as its tiles, the
/// sheet as wide as the widest row; the backdrop or transparent between.
pub fn contact_sheet(tiles: &[(Group, u32, u32, Vec<u8>)], background: [f32; 3], transparent: bool) -> (u32, u32, Vec<u8>) {
    let rows: Vec<Vec<&(Group, u32, u32, Vec<u8>)>> = Group::ALL
        .iter()
        .map(|g| tiles.iter().filter(|t| t.0 == *g).collect::<Vec<_>>())
        .filter(|r| !r.is_empty())
        .collect();
    let width = rows
        .iter()
        .map(|r| CONTACT_GAP + r.iter().map(|t| t.1 + CONTACT_GAP).sum::<u32>())
        .max()
        .unwrap_or(1);
    let height = CONTACT_GAP + rows.len() as u32 * (CONTACT_TILE_HEIGHT + CONTACT_GAP);
    let fill = background.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
    let fill = [fill[0], fill[1], fill[2], if transparent { 0 } else { 255 }];
    let mut out: Vec<u8> = std::iter::repeat_n(fill, (width * height) as usize).flatten().collect();
    let mut y = CONTACT_GAP;
    for row in rows {
        let mut x = CONTACT_GAP;
        for (_, tw, th, px) in row {
            for ty in 0..(*th).min(CONTACT_TILE_HEIGHT) {
                let src = (ty * tw * 4) as usize;
                let dst = (((y + ty) * width + x) * 4) as usize;
                let n = (*tw * 4) as usize;
                if dst + n <= out.len() && src + n <= px.len() {
                    out[dst..dst + n].copy_from_slice(&px[src..src + n]);
                }
            }
            x += tw + CONTACT_GAP;
        }
        y += CONTACT_TILE_HEIGHT + CONTACT_GAP;
    }
    (width, height, out)
}

/// The manifest: the avatar, the setup, every shot written. No asset id of
/// any kind, the avatar's own id only (as the fork's).
pub fn manifest(h: &Header, shots: &[serde_json::Value], contact: Option<&str>) -> serde_json::Value {
    let mut m = json!({
        "created": chrono::Local::now().to_rfc3339(),
        "viewer": aurora_version::version(),
        "avatar": {
            "id": h.avatar_id.to_string(),
            "user_name": h.user_name,
            "display_name": h.display_name,
            "height_m": h.height_m,
        },
        "setup": {
            "a_pose": h.a_pose,
            "neutral_light": h.neutral_light,
            "transparent": h.transparent,
            "isolated": true,
            "background_srgb": h.background,
            "zoom_percent": h.zoom_percent,
            "height_offset_m": h.height_offset_m,
            "supersample": h.supersample,
            "directions": "camera position relative to the avatar; left and right are the avatar's",
        },
        "image_count": shots.len(),
        "shots": shots,
    });
    if let Some(c) = contact {
        m["contact_sheet"] = json!(c);
    }
    m
}
