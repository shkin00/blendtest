//! The painting window: the terrain in the middle, the controls on the left.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, anyhow};
use blendtest::blend::{LAYERS, clip, texel_alpha};
use blendtest::gpu::{self, Settings, Terrain, View, ViewMode};
use blendtest::mask::{Brush, Mask};
use blendtest::noise::{self, AlphaMap, KINDS, NoiseKind, NoiseParams};
use blendtest::{scene, textures};
use egui::{Color32, PointerButton, Sense, Stroke};

use crate::slider::{Notch, slider};
use egui_wgpu::ScreenDescriptor;
use glam::{UVec2, Vec2};
use image::RgbaImage;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

const MAX_ZOOM: u32 = 12;
const SPACINGS: [u32; 5] = [4, 8, 16, 32, 64];
const NOISE_SIZES: [u32; 5] = [32, 64, 128, 256, 512];
/// Behind the world and in the gap the side panel leaves.
const BACKGROUND: wgpu::Color = wgpu::Color {
    r: 0.08,
    g: 0.08,
    b: 0.09,
    a: 1.0,
};

pub struct Options {
    /// Start with the demo painting rather than bare soil.
    pub demo: bool,
    /// Write the window's third frame to this PNG and exit.
    pub shot: Option<PathBuf>,
}

pub fn run(assets: &Path, options: Options) -> anyhow::Result<()> {
    let layers = textures::load(assets)?;
    let event_loop = EventLoop::new()?;
    let mut app = App {
        editor: Editor::new(assets.to_path_buf(), layers, options.demo),
        egui_ctx: egui::Context::default(),
        egui_state: None,
        gfx: None,
        shot: options.shot,
        frames: 0,
        error: None,
    };
    event_loop.run_app(&mut app)?;
    match app.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// What is painted and how it is shown.
struct Editor {
    assets: PathBuf,
    layers: [RgbaImage; 3],
    layers_changed: bool,
    noise: [NoiseParams; 3],
    alphas: [AlphaMap; 3],
    /// The options each alpha map was made from.
    alpha_params: [NoiseParams; 3],
    alphas_changed: bool,
    masks: [Mask; 2],
    masks_changed: bool,
    settings: Settings,
    brush: Brush,
    /// Mask units per second at the brush's centre.
    strength: f32,
    zoom: u32,
    /// The world point at the viewport's centre, so a resize keeps the view where it was.
    centre: Vec2,
    /// Where the last stamp of the current stroke went.
    last_stamp: Option<Vec2>,
    /// The viewport's top-left in window pixels and what it shows, as laid out this frame.
    view: Option<(UVec2, View)>,
    message: String,
}

impl Editor {
    fn new(assets: PathBuf, layers: [RgbaImage; 3], demo: bool) -> Self {
        let masks = if demo {
            scene::demo(scene::SPACING)
        } else {
            std::array::from_fn(|_| Mask::new(scene::WORLD, scene::SPACING))
        };
        let noise = std::array::from_fn(noise::layer_defaults);
        let alphas = noise::alpha_maps(&noise, &layers);
        Self {
            assets,
            layers,
            layers_changed: false,
            noise,
            alphas,
            alpha_params: noise,
            alphas_changed: false,
            masks,
            masks_changed: false,
            settings: Settings::default(),
            brush: Brush {
                radius: 48.0,
                hardness: 0.3,
            },
            strength: 1.0,
            zoom: 3,
            centre: Vec2::splat(scene::WORLD as f32 / 2.0),
            last_stamp: None,
            view: None,
            message: String::new(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        self.shortcuts(ui);
        egui::Panel::left("controls")
            .default_size(270.0)
            .resizable(false)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.controls(ui));
            });
        egui::CentralPanel::no_frame().show(ui, |ui| self.viewport(ui));
        self.refresh_alphas();
    }

    /// 1 to 3 pick a layer, [ and ] shrink and grow the brush.
    fn shortcuts(&mut self, ui: &egui::Ui) {
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        ui.input(|input| {
            for (key, layer) in [
                (egui::Key::Num1, 0),
                (egui::Key::Num2, 1),
                (egui::Key::Num3, 2),
            ] {
                if input.key_pressed(key) {
                    self.settings.active = layer;
                }
            }
            if input.key_pressed(egui::Key::OpenBracket) {
                self.brush.radius = (self.brush.radius / 1.25).max(2.0);
            }
            if input.key_pressed(egui::Key::CloseBracket) {
                self.brush.radius = (self.brush.radius * 1.25).min(256.0);
            }
        });
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        ui.heading("Layers");
        for layer in (0..LAYERS.len()).rev() {
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.settings.visible[layer], "")
                    .on_hover_text("Draw this layer");
                ui.radio_value(
                    &mut self.settings.active,
                    layer,
                    format!("{} {}", layer + 1, LAYERS[layer]),
                );
                let (width, height) = self.layers[layer].dimensions();
                ui.weak(format!("{width}x{height}"));
            });
        }
        ui.weak("Soil is the base. Dirt, then grass, are clipped over it.");
        if ui.button("Reload textures").clicked() {
            match textures::load(&self.assets) {
                Ok(layers) => {
                    self.layers = layers;
                    self.layers_changed = true;
                    // A map can follow the albedo's size or brightness.
                    self.alphas = noise::alpha_maps(&self.noise, &self.layers);
                    self.alphas_changed = true;
                    self.message = "Textures reloaded.".into();
                }
                Err(e) => self.message = format!("{e:#}"),
            }
        }

        ui.separator();
        self.noise_controls(ui);

        ui.separator();
        ui.heading("Brush");
        let brush = &mut self.brush;
        slider(
            ui,
            true,
            &mut brush.radius,
            2.0..=256.0,
            "radius, px",
            Notch::Times(1.1),
        );
        slider(
            ui,
            true,
            &mut brush.hardness,
            0.0..=1.0,
            "hardness",
            Notch::Add(0.05),
        );
        let strength = &mut self.strength;
        slider(
            ui,
            true,
            strength,
            0.05..=8.0,
            "strength / s",
            Notch::Times(1.1),
        );
        ui.weak("Left drag paints the selected layer; right or Ctrl+left drag erases it.");

        ui.separator();
        ui.heading("Clip");
        let spread = &mut self.settings.spread;
        slider(ui, true, spread, 0.0..=0.5, "spread", Notch::Add(0.01));
        ui.horizontal(|ui| {
            if ui.button("Hard").clicked() {
                self.settings.spread = 0.0;
            }
            if ui.button("Soft (UFO)").clicked() {
                self.settings.spread = 0.3;
            }
        });
        ui.checkbox(&mut self.settings.snap, "Read masks at texel centres");

        ui.separator();
        ui.heading("Masks");
        let mut spacing = self.masks[0].spacing();
        egui::ComboBox::from_label("px per mask texel")
            .selected_text(spacing.to_string())
            .show_ui(ui, |ui| {
                for option in SPACINGS {
                    ui.selectable_value(&mut spacing, option, option.to_string());
                }
            });
        if spacing != self.masks[0].spacing() {
            self.masks = self
                .masks
                .each_ref()
                .map(|mask| mask.resampled(scene::WORLD, spacing));
            self.masks_changed = true;
        }
        let painted = self.settings.active.checked_sub(1);
        ui.horizontal(|ui| {
            for (label, value) in [("Clear layer", 0.0), ("Fill layer", 1.0)] {
                if ui
                    .add_enabled(painted.is_some(), egui::Button::new(label))
                    .clicked()
                    && let Some(mask) = painted
                {
                    self.masks[mask].fill(value);
                    self.masks_changed = true;
                }
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Clear all").clicked() {
                self.masks.iter_mut().for_each(|mask| mask.fill(0.0));
                self.masks_changed = true;
            }
            if ui.button("Demo painting").clicked() {
                self.masks = scene::demo(spacing);
                self.masks_changed = true;
            }
        });

        ui.separator();
        ui.heading("View");
        ui.horizontal_wrapped(|ui| {
            let mode = &mut self.settings.mode;
            ui.radio_value(mode, ViewMode::Final, "Final");
            ui.radio_value(mode, ViewMode::Mask, "Mask");
            ui.radio_value(mode, ViewMode::Alpha, "Alpha");
            ui.radio_value(mode, ViewMode::Winner, "Winner");
        });
        ui.label(format!("Zoom {}x", self.zoom));
        ui.weak("Wheel zooms, middle drag pans. 1-3 pick a layer, [ and ] size the brush.");

        if !self.message.is_empty() {
            ui.separator();
            ui.label(&self.message);
        }
    }

    /// The selected layer's alpha noise; `refresh_alphas` makes the map again after a change.
    fn noise_controls(&mut self, ui: &mut egui::Ui) {
        let layer = self.settings.active;
        let albedo = self.layers[layer].width();
        ui.heading(format!("Alpha noise: {}", LAYERS[layer]));
        if layer == 0 {
            ui.weak("Soil is never clipped; its alpha shows only in the Alpha view.");
        }
        let params = &mut self.noise[layer];
        egui::ComboBox::from_label("kind")
            .selected_text(params.kind.name())
            .show_ui(ui, |ui| {
                for kind in KINDS {
                    ui.selectable_value(&mut params.kind, kind, kind.name());
                }
            });
        let luminance = params.kind == NoiseKind::Luminance;
        let size_text = |size: Option<u32>| match size {
            None => format!("albedo ({albedo})"),
            Some(size) => size.to_string(),
        };
        ui.add_enabled_ui(!luminance, |ui| {
            egui::ComboBox::from_label("size, texels")
                // Luminance is always the albedo's size, whatever the option holds.
                .selected_text(size_text(if luminance { None } else { params.size }))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut params.size, None, size_text(None));
                    for size in NOISE_SIZES {
                        ui.selectable_value(&mut params.size, Some(size), size_text(Some(size)));
                    }
                });
        });
        let feature = if params.kind == NoiseKind::Cellular {
            "cell, texels"
        } else {
            "feature, texels"
        };
        let sized = params.kind != NoiseKind::White;
        slider(
            ui,
            sized,
            &mut params.feature,
            1.0..=64.0,
            feature,
            Notch::Times(1.1),
        );
        let octaves = matches!(params.kind, NoiseKind::Fbm | NoiseKind::Luminance);
        slider(
            ui,
            octaves,
            &mut params.octaves,
            1..=6,
            "octaves",
            Notch::Add(1.0),
        );
        let persistence = &mut params.persistence;
        slider(
            ui,
            octaves,
            persistence,
            0.1..=1.0,
            "persistence",
            Notch::Add(0.05),
        );
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut params.seed).prefix("seed "));
            if ui.button("New seed").clicked() {
                params.seed = params
                    .seed
                    .wrapping_mul(1_664_525)
                    .wrapping_add(1_013_904_223);
            }
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut params.equalise, "Equalise");
            ui.checkbox(&mut params.invert, "Invert");
        });
        if luminance {
            ui.weak("Bright texels show first (dark with Invert); FBM orders equal colours.");
        }
    }

    /// Make again every alpha map whose options changed since it was made.
    fn refresh_alphas(&mut self) {
        for (layer, name) in LAYERS.iter().enumerate() {
            if self.noise[layer] == self.alpha_params[layer] {
                continue;
            }
            let started = std::time::Instant::now();
            self.alphas[layer] = noise::alpha_map(&self.noise[layer], &self.layers[layer]);
            self.alpha_params[layer] = self.noise[layer];
            self.alphas_changed = true;
            let map = &self.alphas[layer];
            self.message = format!(
                "{name} alpha: {}x{} in {:.0} ms.",
                map.width,
                map.height,
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
    }

    fn viewport(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        let ppp = ui.ctx().pixels_per_point();
        let corner = UVec2::new(
            (rect.min.x * ppp).round().max(0.0) as u32,
            (rect.min.y * ppp).round().max(0.0) as u32,
        );
        let size = UVec2::new(
            (rect.width() * ppp).round().max(1.0) as u32,
            (rect.height() * ppp).round().max(1.0) as u32,
        );
        let half = size.as_vec2() / 2.0;
        // From a point in the viewport to window pixels from its centre.
        let from_centre = |pointer: egui::Pos2| {
            Vec2::new(pointer.x - rect.min.x, pointer.y - rect.min.y) * ppp - half
        };

        if response.dragged_by(PointerButton::Middle) {
            let drag = response.drag_delta() * ppp;
            self.centre -= Vec2::new(drag.x, drag.y) / self.zoom as f32;
        }
        if let Some(pointer) = response.hover_pos() {
            // One step per wheel notch, about the point under the cursor.
            let scroll: f32 = ui.input(|input| {
                input
                    .events
                    .iter()
                    .filter_map(|event| match event {
                        egui::Event::MouseWheel { delta, .. } => Some(delta.y),
                        _ => None,
                    })
                    .sum()
            });
            let to = match scroll {
                s if s > 0.0 => (self.zoom + 1).min(MAX_ZOOM),
                s if s < 0.0 => self.zoom.saturating_sub(1).max(1),
                _ => self.zoom,
            };
            if to != self.zoom {
                let under = self.centre + from_centre(pointer) / self.zoom as f32;
                self.centre = under - from_centre(pointer) / to as f32;
                self.zoom = to;
            }
        }
        let view = View::new(self.centre - half / self.zoom as f32, self.zoom, size);
        self.view = Some((corner, view));
        let world_at = |pointer| view.origin() + (from_centre(pointer) + half) / view.zoom() as f32;

        let (primary, secondary, ctrl, dt) = ui.input(|input| {
            (
                input.pointer.primary_down(),
                input.pointer.secondary_down(),
                input.modifiers.ctrl,
                input.stable_dt.min(0.1),
            )
        });
        let painting = response.is_pointer_button_down_on() && (primary || secondary);
        match (
            painting,
            self.settings.active.checked_sub(1),
            response.hover_pos(),
        ) {
            (true, Some(mask), Some(pointer)) => {
                let at = world_at(pointer);
                let sign = if secondary || ctrl { -1.0 } else { 1.0 };
                // Stamps at most a quarter radius apart, sharing this frame's paint.
                let from = self.last_stamp.unwrap_or(at);
                let gap = (self.brush.radius * 0.25).max(1.0);
                let stamps = (from.distance(at) / gap).ceil().max(1.0);
                let amount = sign * self.strength * dt / stamps;
                for stamp in 1..=stamps as u32 {
                    let centre = from.lerp(at, stamp as f32 / stamps);
                    self.masks_changed |= self.masks[mask].stamp(centre, self.brush, amount);
                }
                self.last_stamp = Some(at);
            }
            _ => self.last_stamp = None,
        }

        if let Some(pointer) = response.hover_pos() {
            let painter = ui.painter_at(rect);
            let scale = self.zoom as f32 / ppp;
            painter.circle_stroke(
                pointer,
                self.brush.radius * scale,
                Stroke::new(1.5, Color32::from_white_alpha(220)),
            );
            if self.brush.hardness > 0.0 {
                painter.circle_stroke(
                    pointer,
                    self.brush.radius * self.brush.hardness * scale,
                    Stroke::new(1.0, Color32::from_white_alpha(90)),
                );
            }
            let at = world_at(pointer);
            painter.text(
                rect.left_bottom() + egui::vec2(8.0, -8.0),
                egui::Align2::LEFT_BOTTOM,
                self.readout(at),
                egui::FontId::monospace(13.0),
                Color32::from_white_alpha(230),
            );
        }
    }

    /// The texel under the cursor: its alpha and mask in the active layer, and whether the
    /// hard clip shows it.
    fn readout(&self, at: Vec2) -> String {
        let world = scene::WORLD as f32;
        if at.x < 0.0 || at.y < 0.0 || at.x >= world || at.y >= world {
            return String::new();
        }
        let texel = at.floor();
        let layer = self.settings.active;
        let a8 = self.alphas[layer].get(texel.as_ivec2());
        let line = format!("texel {:>4} {:>4}  {}", texel.x, texel.y, LAYERS[layer]);
        match layer.checked_sub(1) {
            None => format!("{line}  alpha {a8:>3}"),
            Some(mask) => {
                let value = self.masks[mask].quantized().sample(texel + 0.5);
                let shown = clip(value, texel_alpha(a8), 0.0) >= 1.0;
                format!(
                    "{line}  alpha {a8:>3}  mask {:>3}  {}",
                    (value * 255.0).round(),
                    if shown { "shown" } else { "clipped" }
                )
            }
        }
    }
}

/// The window's device, surface and passes.
struct Gfx {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    queue: wgpu::Queue,
    terrain: Terrain,
    egui: egui_wgpu::Renderer,
}

/// One frame of egui, tessellated. Its texture changes go through `Gfx::upload_ui_textures`
/// and `Gfx::free_ui_textures`, which run whether or not the frame is drawn.
struct UiFrame {
    primitives: Vec<egui::ClippedPrimitive>,
    pixels_per_point: f32,
}

impl Gfx {
    async fn new(window: Arc<Window>, editor: &Editor) -> anyhow::Result<Self> {
        let instance = gpu::instance();
        let surface = instance.create_surface(window.clone())?;
        let (adapter, device, queue) = gpu::device(&instance, Some(&surface)).await?;
        let info = adapter.get_info();
        eprintln!("adapter: {} ({:?})", info.name, info.backend);
        let size = window.inner_size();
        let capabilities = surface.get_capabilities(&adapter);
        // Not sRGB, as in Krust: texel bytes reach the screen unchanged.
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(|format| !format.is_srgb())
            .or(capabilities.formats.first().copied())
            .ok_or_else(|| anyhow!("surface offers no formats"))?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: capabilities.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);
        let terrain = Terrain::new(
            &device,
            &queue,
            format,
            &editor.layers,
            &editor.alphas,
            &editor.masks,
        );
        let egui = egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());
        Ok(Self {
            surface,
            config,
            device,
            queue,
            terrain,
            egui,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
        }
    }

    /// Bring the textures up to date with the editor's edits.
    fn sync(&mut self, editor: &mut Editor) {
        if std::mem::take(&mut editor.layers_changed) {
            self.terrain
                .set_layers(&self.device, &self.queue, &editor.layers);
        }
        if std::mem::take(&mut editor.alphas_changed) {
            self.terrain
                .set_alphas(&self.device, &self.queue, &editor.alphas);
        }
        if std::mem::take(&mut editor.masks_changed) {
            self.terrain
                .set_masks(&self.device, &self.queue, &editor.masks);
        }
    }

    /// egui's new and changed textures, before the frame that uses them.
    fn upload_ui_textures(&mut self, textures: &mut egui::TexturesDelta) {
        for (id, deltas) in textures.set.drain() {
            for delta in &deltas {
                self.egui
                    .update_texture(&self.device, &self.queue, id, delta);
            }
        }
    }

    /// The textures egui is done with, after the frame.
    fn free_ui_textures(&mut self, textures: &mut egui::TexturesDelta) {
        for id in textures.free.drain() {
            self.egui.free_texture(&id);
        }
    }

    fn present(&mut self, editor: &Editor, ui: UiFrame) -> anyhow::Result<()> {
        use wgpu::CurrentSurfaceTexture::*;
        let frame = match self.surface.get_current_texture() {
            Success(frame) | Suboptimal(frame) => frame,
            Outdated | Lost => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            Timeout | Occluded => return Ok(()),
            Validation => return Err(anyhow!("surface configuration rejected by the backend")),
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.draw(&view, editor, ui);
        self.queue.present(frame);
        Ok(())
    }

    /// Draw a frame into a texture like the window's and write it as a PNG.
    fn screenshot(&mut self, editor: &Editor, ui: UiFrame, path: &Path) -> anyhow::Result<()> {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("screenshot"),
            size: wgpu::Extent3d {
                width: self.config.width,
                height: self.config.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.draw(&view, editor, ui);
        gpu::read_back(&self.device, &self.queue, &texture)?
            .save(path)
            .with_context(|| format!("write {}", path.display()))
    }

    /// The terrain into its viewport, then egui over the whole target.
    fn draw(&mut self, target: &wgpu::TextureView, editor: &Editor, ui: UiFrame) {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        let surface = UVec2::new(self.config.width, self.config.height);
        // The viewport, cut to the target: egui's layout may run a frame behind a resize.
        let terrain_view = editor.view.and_then(|(corner, view)| {
            let size = view.size().min(surface.saturating_sub(corner));
            (size.x > 0 && size.y > 0)
                .then(|| (corner, View::new(view.origin(), view.zoom(), size)))
        });
        if let Some((corner, view)) = &terrain_view {
            self.terrain
                .prepare(&self.queue, view, *corner, &editor.settings);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("terrain"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(BACKGROUND),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some((corner, view)) = &terrain_view {
                self.terrain.draw(&mut pass, view, *corner);
            }
        }

        let UiFrame {
            primitives,
            pixels_per_point,
        } = ui;
        let screen = ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point,
        };
        let egui_commands = self.egui.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            &primitives,
            &screen,
        );
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            self.egui.render(&mut pass, &primitives, &screen);
        }
        self.queue
            .submit(egui_commands.into_iter().chain([encoder.finish()]));
    }
}

struct App {
    editor: Editor,
    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    gfx: Option<(Arc<Window>, Gfx)>,
    shot: Option<PathBuf>,
    frames: u32,
    error: Option<anyhow::Error>,
}

impl App {
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: anyhow::Error) {
        self.error = Some(error);
        event_loop.exit();
    }

    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        let (Some((window, gfx)), Some(state)) = (&mut self.gfx, &mut self.egui_state) else {
            return;
        };
        let raw_input = state.take_egui_input(window);
        let ctx = self.egui_ctx.clone();
        let editor = &mut self.editor;
        let output = ctx.run_ui(raw_input, |ui| editor.ui(ui));
        state.handle_platform_output(window, output.platform_output);
        let ui = UiFrame {
            primitives: ctx.tessellate(output.shapes, output.pixels_per_point),
            pixels_per_point: output.pixels_per_point,
        };
        let mut textures = output.textures_delta;
        gfx.upload_ui_textures(&mut textures);
        gfx.sync(editor);
        self.frames += 1;
        // egui lays itself out over the first frames; the screenshot waits for the third.
        let result = match &self.shot {
            Some(path) if self.frames >= 3 => gfx
                .screenshot(editor, ui, path)
                .map(|()| println!("{}", path.display())),
            _ => gfx.present(editor, ui),
        };
        gfx.free_ui_textures(&mut textures);
        let window = window.clone();
        match result {
            Err(e) => self.fail(event_loop, e),
            Ok(()) if self.shot.is_some() && self.frames >= 3 => event_loop.exit(),
            Ok(()) => window.request_redraw(),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("blendtest")
            .with_inner_size(LogicalSize::new(1600, 1000));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(e) => return self.fail(event_loop, e.into()),
        };
        match pollster::block_on(Gfx::new(window.clone(), &self.editor)) {
            Ok(gfx) => {
                self.egui_state = Some(egui_winit::State::new(
                    self.egui_ctx.clone(),
                    egui::ViewportId::ROOT,
                    window.as_ref(),
                    Some(window.scale_factor() as f32),
                    None,
                    None,
                ));
                window.request_redraw();
                self.gfx = Some((window, gfx));
            }
            Err(e) => self.fail(event_loop, e),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        if let (Some((window, _)), Some(state)) = (&self.gfx, &mut self.egui_state) {
            let _ = state.on_window_event(window, &event);
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some((_, gfx)) = &mut self.gfx {
                    gfx.resize(size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => self.frame(event_loop),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor() -> Editor {
        Editor::new(PathBuf::new(), textures::placeholders(), false)
    }

    /// One egui frame of a 1200 x 800 window at one pixel per point.
    fn frame(ctx: &egui::Context, editor: &mut Editor, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 800.0),
            )),
            events,
            ..Default::default()
        };
        // No renderer here to take egui's textures.
        ctx.run_ui(input, |ui| editor.ui(ui)).textures_delta.clear();
    }

    /// A point 400 x 300 points inside the viewport, and the world point under it.
    fn inside(editor: &Editor, ppp: f32) -> (egui::Pos2, Vec2) {
        let (corner, view) = editor.view.expect("the viewport is laid out");
        let offset = Vec2::new(400.0, 300.0);
        let pos = egui::pos2(
            corner.x as f32 / ppp + offset.x,
            corner.y as f32 / ppp + offset.y,
        );
        (pos, view.origin() + offset * ppp / view.zoom() as f32)
    }

    fn press(
        pos: egui::Pos2,
        button: PointerButton,
        modifiers: egui::Modifiers,
    ) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button,
                pressed: true,
                modifiers,
            },
        ]
    }

    /// Paint dirt at 400 x 300 points into the viewport at this display scale.
    fn paints_under_the_cursor(ppp: f32) {
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(ppp);
        let mut editor = editor();
        // egui lays out its first frame after a scale change on a placeholder screen.
        frame(&ctx, &mut editor, vec![]);
        frame(&ctx, &mut editor, vec![]);
        assert_eq!(ctx.pixels_per_point(), ppp);
        let (pos, target) = inside(&editor, ppp);
        let left = press(pos, PointerButton::Primary, egui::Modifiers::NONE);
        frame(&ctx, &mut editor, left);
        frame(&ctx, &mut editor, vec![]);
        let [dirt, grass] = &editor.masks;
        assert!(dirt.sample(target) > 0.0, "scale {ppp}");
        assert_eq!(dirt.sample(target + Vec2::splat(64.0)), 0.0, "scale {ppp}");
        assert_eq!(grass.sample(target), 0.0, "scale {ppp}");
        assert!(editor.masks_changed);
    }

    #[test]
    fn a_left_drag_paints_the_selected_layer_under_the_cursor() {
        paints_under_the_cursor(1.0);
    }

    #[test]
    fn painting_lands_under_the_cursor_at_150_percent_scaling() {
        paints_under_the_cursor(1.5);
    }

    /// Erase a full grass layer at 400 x 300 points into the viewport.
    fn erases(button: PointerButton, modifiers: egui::Modifiers) {
        let ctx = egui::Context::default();
        let mut editor = editor();
        editor.settings.active = 2;
        editor.masks[1].fill(1.0);
        frame(&ctx, &mut editor, vec![]);
        let (pos, target) = inside(&editor, 1.0);
        // Held keys stay held from this event on.
        let mut events = vec![egui::Event::ModifiersChanged(modifiers)];
        events.extend(press(pos, button, modifiers));
        frame(&ctx, &mut editor, events);
        frame(&ctx, &mut editor, vec![]);
        assert!(editor.masks[1].sample(target) < 1.0);
        assert_eq!(editor.masks[1].sample(target + Vec2::splat(64.0)), 1.0);
    }

    #[test]
    fn ctrl_drag_erases() {
        erases(PointerButton::Primary, egui::Modifiers::CTRL);
    }

    #[test]
    fn right_drag_erases() {
        erases(PointerButton::Secondary, egui::Modifiers::NONE);
    }

    #[test]
    fn a_changed_noise_option_makes_that_layers_map_again() {
        let ctx = egui::Context::default();
        let mut editor = editor();
        frame(&ctx, &mut editor, vec![]);
        let before = editor.alphas.clone();
        editor.alphas_changed = false;
        editor.noise[2].kind = NoiseKind::Cellular;
        editor.noise[2].size = Some(64);
        frame(&ctx, &mut editor, vec![]);
        assert!(editor.alphas_changed);
        assert_eq!(
            editor.alphas[2],
            noise::alpha_map(&editor.noise[2], &editor.layers[2])
        );
        assert_eq!((editor.alphas[2].width, editor.alphas[2].height), (64, 64));
        assert_eq!(editor.alphas[..2], before[..2]);
        // Nothing changed since: nothing is made again.
        editor.alphas_changed = false;
        frame(&ctx, &mut editor, vec![]);
        assert!(!editor.alphas_changed);
    }

    #[test]
    fn the_wheel_zooms_about_the_cursor() {
        let ctx = egui::Context::default();
        let mut editor = editor();
        frame(&ctx, &mut editor, vec![]);
        let (pos, before) = inside(&editor, 1.0);
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, 1.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        };
        frame(
            &ctx,
            &mut editor,
            vec![egui::Event::PointerMoved(pos), wheel],
        );
        assert_eq!(editor.zoom, 4);
        let (_, after) = inside(&editor, 1.0);
        // The origin snaps to a whole screen pixel: a quarter world pixel at 4x.
        assert!(
            before.distance(after) <= 0.25 * 2f32.sqrt(),
            "{before} {after}"
        );
    }
}
