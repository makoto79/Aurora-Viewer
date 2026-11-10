//! The character sheet capture's window (the fork's floater_avatar_sheet.xml,
//! LLFloaterAvatarSheet): the avatar, the setup, the framing, the output and
//! the run. Aurora style only; the app does the work (`app/charsheet.rs`).

use super::widgets::{self, ActionKind, Floater};
use crate::charsheet::{self, Resolution, Settings};
use crate::theme::Palette;
use egui::Vec2;
use uuid::Uuid;

pub const WINDOW: &str = "charsheet";
/// The function's icon, in its menus, its toolbar button and its window: a
/// character's sheet (the toolbar's "user-scan" is another command's).
pub const ICON: &str = "id-filled";

#[derive(Default)]
pub struct CharsheetUi {
    pub open: bool,
    /// The avatar chosen (nil: our own).
    pub subject: Uuid,
}

/// One avatar of the list.
pub struct Candidate {
    pub id: Uuid,
    pub name: String,
    pub you: bool,
}

/// What the app shows in the window this frame.
pub struct View<'a> {
    pub candidates: &'a [Candidate],
    /// "Height 1.78 m · Skeleton complete", or why it cannot be measured.
    pub info: Option<String>,
    /// The base folder, as shown.
    pub folder: String,
    pub folder_is_default: bool,
    /// The preview's step ("3/20 · Head left") while it is up.
    pub preview: Option<String>,
    /// A run is going.
    pub running: bool,
    /// The run's progress and its line.
    pub progress: Option<(f32, String)>,
    /// The last message, and whether it is a problem.
    pub status: Option<(String, bool)>,
    /// Preview and Capture may start (an avatar chosen, measurable, in world).
    pub can_start: bool,
    /// A finished series' folder: "Open folder".
    pub last_folder: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Select(Uuid),
    /// The preview one shot back or forward (it starts on the first press).
    Preview(i32),
    EndPreview,
    Start,
    Stop,
    Browse,
    DefaultFolder,
    OpenFolder,
    /// A setting changed: saved, and the preview follows.
    Changed,
    Closed,
}

impl CharsheetUi {
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, s: &mut Settings, view: &View<'_>) -> Vec<Action> {
        let mut actions = Vec::new();
        if !self.open {
            return actions;
        }
        let mut open = self.open;
        let screen = ctx.content_rect();
        let size = Vec2::new(420.0, 700.0);
        Floater::new(
            WINDOW,
            aurora_i18n::tr!("charsheet-title"),
            egui::pos2(screen.right() - size.x - 40.0, 90.0),
            size,
        )
        .framed_header(ICON)
        .show(ctx, p, &mut open, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                self.body(ui, p, s, view, &mut actions);
            });
        });
        if !open {
            self.open = false;
            actions.push(Action::Closed);
        }
        actions
    }

    fn body(&mut self, ui: &mut egui::Ui, p: &Palette, s: &mut Settings, view: &View<'_>, actions: &mut Vec<Action>) {
        let idle = !view.running;
        let mut changed = false;

        // ---- avatar
        widgets::section_header(ui, p, "user-filled", aurora_i18n::tr!("charsheet-section-avatar"));
        ui.add_enabled_ui(idle, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("charsheet-avatars")
                .max_height(110.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    if view.candidates.is_empty() {
                        ui.label(egui::RichText::new(aurora_i18n::tr!("charsheet-no-avatar")).color(p.muted));
                    }
                    for c in view.candidates {
                        let name = if c.you {
                            aurora_i18n::tr!("charsheet-you", name = c.name.as_str())
                        } else {
                            c.name.clone()
                        };
                        let chosen = c.id == self.subject;
                        if ui.selectable_label(chosen, name).clicked() && !chosen {
                            self.subject = c.id;
                            actions.push(Action::Select(c.id));
                        }
                    }
                });
        });
        if let Some(info) = &view.info {
            ui.label(egui::RichText::new(info).size(12.0).color(p.muted));
        }
        ui.add_space(8.0);

        // ---- setup
        widgets::section_header(ui, p, "adjustments-filled", aurora_i18n::tr!("charsheet-section-setup"));
        ui.add_enabled_ui(idle, |ui| {
            changed |= widgets::switch_labeled(ui, p, &mut s.a_pose, aurora_i18n::tr!("charsheet-a-pose"))
                .on_hover_text(aurora_i18n::tr!("charsheet-a-pose-tip"))
                .changed();
            changed |= widgets::switch_labeled(ui, p, &mut s.neutral_light, aurora_i18n::tr!("charsheet-neutral-light"))
                .on_hover_text(aurora_i18n::tr!("charsheet-neutral-light-tip"))
                .changed();
            ui.horizontal(|ui| {
                ui.label(aurora_i18n::tr!("charsheet-background"));
                // the user's own backdrop colour, sRGB 0..1 in the settings
                let [r, g, b] = s.background.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
                let mut color = [r, g, b, 255];
                if widgets::color_button(ui, p, "charsheet-background", &mut color, false)
                    .on_hover_text(aurora_i18n::tr!("charsheet-background-tip"))
                    .changed()
                {
                    s.background = [color[0], color[1], color[2]].map(|c| f32::from(c) / 255.0);
                    changed = true;
                }
                ui.add_space(12.0);
                changed |= widgets::checkbox(ui, p, &mut s.transparent, aurora_i18n::tr!("charsheet-transparent"))
                    .on_hover_text(aurora_i18n::tr!("charsheet-transparent-tip"))
                    .changed();
            });
        });
        ui.add_space(8.0);

        // ---- framing
        widgets::section_header(ui, p, "aspect-ratio-filled", aurora_i18n::tr!("charsheet-section-framing"));
        ui.add_enabled_ui(idle, |ui| {
            ui.spacing_mut().slider_width = 220.0;
            changed |= ui
                .add(
                    egui::Slider::new(&mut s.zoom_percent, charsheet::ZOOM_RANGE)
                        .step_by(1.0)
                        .suffix(" %")
                        .text(aurora_i18n::tr!("charsheet-zoom")),
                )
                .on_hover_text(aurora_i18n::tr!("charsheet-zoom-tip"))
                .changed();
            changed |= ui
                .add(
                    egui::Slider::new(&mut s.height_offset_m, charsheet::HEIGHT_OFFSET_RANGE)
                        .step_by(0.01)
                        .fixed_decimals(2)
                        .suffix(" m")
                        .text(aurora_i18n::tr!("charsheet-height-offset")),
                )
                .on_hover_text(aurora_i18n::tr!("charsheet-height-offset-tip"))
                .changed();
        });
        ui.add_space(8.0);

        // ---- output
        widgets::section_header(ui, p, "folder-filled", aurora_i18n::tr!("charsheet-section-output"));
        ui.add_enabled_ui(idle, |ui| {
            ui.horizontal(|ui| {
                let shown = if view.folder_is_default {
                    aurora_i18n::tr!("charsheet-folder-default", folder = view.folder.as_str())
                } else {
                    view.folder.clone()
                };
                let width = (ui.available_width() - 150.0).max(80.0);
                ui.allocate_ui(Vec2::new(width, 24.0), |ui| {
                    ui.set_width(width);
                    ui.add(egui::Label::new(egui::RichText::new(shown.as_str()).size(12.0)).truncate())
                        .on_hover_text(shown.as_str());
                });
                if widgets::action_button(
                    ui,
                    p,
                    "folder-filled",
                    aurora_i18n::tr!("charsheet-browse"),
                    ActionKind::Flat,
                    Vec2::new(0.0, 24.0),
                )
                .on_hover_text(aurora_i18n::tr!("charsheet-folder-tip"))
                .clicked()
                {
                    actions.push(Action::Browse);
                }
                if !view.folder_is_default && widgets::icon_button(ui, p, "x", aurora_i18n::tr!("charsheet-folder-reset"), true) {
                    actions.push(Action::DefaultFolder);
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(aurora_i18n::tr!("charsheet-resolution"));
                for r in Resolution::ALL {
                    let side = r.side();
                    changed |= widgets::radio_value(ui, p, &mut s.resolution, r, format!("{side} × {side}")).changed();
                }
            });
            changed |= widgets::checkbox(ui, p, &mut s.portrait_body, aurora_i18n::tr!("charsheet-portrait"))
                .on_hover_text(aurora_i18n::tr!("charsheet-portrait-tip"))
                .changed();
            changed |= widgets::checkbox(ui, p, &mut s.contact_sheet, aurora_i18n::tr!("charsheet-contact-sheet"))
                .on_hover_text(aurora_i18n::tr!("charsheet-contact-sheet-tip"))
                .changed();
            changed |= widgets::checkbox(ui, p, &mut s.supersample, aurora_i18n::tr!("charsheet-high-quality"))
                .on_hover_text(aurora_i18n::tr!("charsheet-high-quality-tip"))
                .changed();
        });
        ui.add_space(8.0);

        // ---- capture
        widgets::section_header(ui, p, "camera-filled", aurora_i18n::tr!("charsheet-section-capture"));
        ui.label(
            egui::RichText::new(aurora_i18n::tr!("charsheet-image-count", count = charsheet::plan::IMAGE_COUNT))
                .size(12.0)
                .color(p.muted),
        );
        let startable = idle && view.can_start;
        ui.horizontal(|ui| {
            let size = Vec2::new(0.0, 26.0);
            if widgets::action_button_if(
                ui,
                p,
                "chevron-left",
                aurora_i18n::tr!("charsheet-preview-prev"),
                ActionKind::Flat,
                size,
                startable,
            )
            .on_hover_text(aurora_i18n::tr!("charsheet-preview-tip"))
            .clicked()
            {
                actions.push(Action::Preview(-1));
            }
            if widgets::action_button_if(
                ui,
                p,
                "chevron-right",
                aurora_i18n::tr!("charsheet-preview-next"),
                ActionKind::Flat,
                size,
                startable,
            )
            .on_hover_text(aurora_i18n::tr!("charsheet-preview-tip"))
            .clicked()
            {
                actions.push(Action::Preview(1));
            }
            if view.preview.is_some()
                && idle
                && widgets::action_button(ui, p, "x", aurora_i18n::tr!("charsheet-preview-end"), ActionKind::Flat, size)
                    .on_hover_text(aurora_i18n::tr!("charsheet-preview-end-tip"))
                    .clicked()
            {
                actions.push(Action::EndPreview);
            }
        });
        if let Some(step) = &view.preview {
            ui.label(egui::RichText::new(step).size(12.0).color(p.ink));
            ui.label(
                egui::RichText::new(aurora_i18n::tr!("charsheet-camera-locked"))
                    .size(11.0)
                    .color(p.muted),
            );
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(startable, |ui| {
                if widgets::primary_button_sized(ui, p, aurora_i18n::tr!("charsheet-capture"), Vec2::new(120.0, 28.0)).clicked() {
                    actions.push(Action::Start);
                }
            });
            if view.running
                && widgets::action_button(
                    ui,
                    p,
                    "player-stop-filled",
                    aurora_i18n::tr!("charsheet-stop"),
                    ActionKind::Danger,
                    Vec2::new(0.0, 28.0),
                )
                .clicked()
            {
                actions.push(Action::Stop);
            }
            ui.add_space(8.0);
            ui.add_enabled_ui(idle, |ui| {
                ui.spacing_mut().slider_width = 90.0;
                changed |= ui
                    .add(
                        egui::Slider::new(&mut s.delay_s, 0..=charsheet::DELAY_MAX_S)
                            .suffix(" s")
                            .text(aurora_i18n::tr!("charsheet-delay")),
                    )
                    .on_hover_text(aurora_i18n::tr!("charsheet-delay-tip"))
                    .changed();
            });
        });
        if let Some((fraction, line)) = &view.progress {
            ui.add_space(6.0);
            let width = ui.available_width();
            widgets::progress(ui, p, "charsheet-progress", *fraction, width, line, None);
        }
        if let Some((text, problem)) = &view.status {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(text)
                    .size(12.0)
                    .color(if *problem { p.danger } else { p.muted }),
            );
        }
        if view.last_folder
            && idle
            && widgets::action_button(
                ui,
                p,
                "folder-open-filled",
                aurora_i18n::tr!("charsheet-open-folder"),
                ActionKind::Flat,
                Vec2::new(0.0, 24.0),
            )
            .clicked()
        {
            actions.push(Action::OpenFolder);
        }
        if changed {
            actions.push(Action::Changed);
        }
    }
}
