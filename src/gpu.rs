//! The terrain pass, shared by the app's window and the headless renders the checks use.

use anyhow::{Context, anyhow};
use glam::{UVec2, Vec2};
use image::RgbaImage;

use crate::mask::Mask;
use crate::noise::AlphaMap;

pub const SHADER: &str = include_str!("terrain.wgsl");

/// The part of the world a viewport shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    origin: Vec2,
    zoom: u32,
    size: UVec2,
}

impl View {
    /// `origin` is the world point at the viewport's top-left. It is rounded to a whole
    /// viewport pixel, so every texel covers exactly zoom x zoom pixels.
    pub fn new(origin: Vec2, zoom: u32, size: UVec2) -> Self {
        let zoom = zoom.max(1);
        Self {
            origin: (origin * zoom as f32).round() / zoom as f32,
            zoom,
            size: size.max(UVec2::ONE),
        }
    }

    pub fn origin(&self) -> Vec2 {
        self.origin
    }

    pub fn zoom(&self) -> u32 {
        self.zoom
    }

    pub fn size(&self) -> UVec2 {
        self.size
    }

    /// The world point at the centre of a viewport pixel, as `terrain.wgsl` computes it.
    pub fn world_at(&self, pixel: UVec2) -> Vec2 {
        self.origin + (pixel.as_vec2() + 0.5) / self.zoom as f32
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    Final,
    /// The active layer's mask as the clip reads it.
    Mask,
    /// The active layer's alpha noise.
    Alpha,
    /// The layer that won each pixel, in flat colours.
    Winner,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// 0 is the hard clip; UFO uses 0.3.
    pub spread: f32,
    /// Read the masks at texel centres rather than at every pixel.
    pub snap: bool,
    pub visible: [bool; 3],
    pub mode: ViewMode,
    /// The layer the mask and alpha views show.
    pub active: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            spread: 0.0,
            snap: true,
            visible: [true; 3],
            mode: ViewMode::Final,
            active: 1,
        }
    }
}

/// `Params` in `terrain.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    viewport: [f32; 4],
    origin: [f32; 2],
    zoom: f32,
    spread: f32,
    world: f32,
    mask_spacing: f32,
    mask_side: f32,
    view_mode: u32,
    active: u32,
    visible: u32,
    snap: u32,
    pad: u32,
}

/// The pipeline and the textures it reads: three layers and two masks.
pub struct Terrain {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    params: wgpu::Buffer,
    layers: [wgpu::TextureView; 3],
    alphas: [wgpu::TextureView; 3],
    masks: [wgpu::Texture; 2],
    mask_spacing: u32,
    mask_side: u32,
    bind_group: wgpu::BindGroup,
}

impl Terrain {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        layers: &[RgbaImage; 3],
        alphas: &[AlphaMap; 3],
        masks: &[Mask; 2],
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("terrain"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("terrain"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture(1),
                texture(2),
                texture(3),
                texture(4),
                texture(5),
                texture(6),
                texture(7),
                texture(8),
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terrain"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        // The masks are the only filtered textures; the layers and alphas are read texel by
        // texel.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("mask"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("terrain params"),
            size: size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layer_views = upload_layers(device, queue, layers);
        let alpha_views = upload_alphas(device, queue, alphas);
        let mask_textures = create_masks(device, masks[0].side());
        let bind_group = bind(
            device,
            &layout,
            &params,
            [&layer_views, &alpha_views],
            &mask_textures,
            &sampler,
        );
        let mut terrain = Self {
            pipeline,
            layout,
            sampler,
            params,
            layers: layer_views,
            alphas: alpha_views,
            masks: mask_textures,
            mask_spacing: masks[0].spacing(),
            mask_side: masks[0].side(),
            bind_group,
        };
        terrain.set_masks(device, queue, masks);
        terrain
    }

    /// The world's size in pixels, as the masks cover it.
    pub fn world(&self) -> u32 {
        (self.mask_side - 1) * self.mask_spacing
    }

    pub fn set_layers(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layers: &[RgbaImage; 3],
    ) {
        self.layers = upload_layers(device, queue, layers);
        self.rebind(device);
    }

    pub fn set_alphas(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        alphas: &[AlphaMap; 3],
    ) {
        self.alphas = upload_alphas(device, queue, alphas);
        self.rebind(device);
    }

    /// Upload the masks, making new textures when their size changed.
    pub fn set_masks(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, masks: &[Mask; 2]) {
        let side = masks[0].side();
        if side != self.mask_side {
            self.masks = create_masks(device, side);
            self.mask_side = side;
            self.rebind(device);
        }
        self.mask_spacing = masks[0].spacing();
        for (texture, mask) in self.masks.iter().zip(masks) {
            queue.write_texture(
                texture.as_image_copy(),
                &mask.to_r8(),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(side),
                    rows_per_image: Some(side),
                },
                wgpu::Extent3d {
                    width: side,
                    height: side,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    /// Write this frame's parameters; `corner` is the viewport's top-left in target pixels.
    pub fn prepare(&self, queue: &wgpu::Queue, view: &View, corner: UVec2, settings: &Settings) {
        let visible = settings
            .visible
            .iter()
            .enumerate()
            .fold(0, |bits, (layer, &on)| bits | (u32::from(on) << layer));
        let params = Params {
            viewport: [
                corner.x as f32,
                corner.y as f32,
                view.size.x as f32,
                view.size.y as f32,
            ],
            origin: view.origin.to_array(),
            zoom: view.zoom as f32,
            spread: settings.spread,
            world: self.world() as f32,
            mask_spacing: self.mask_spacing as f32,
            mask_side: self.mask_side as f32,
            view_mode: settings.mode as u32,
            active: settings.active.min(2) as u32,
            visible,
            snap: u32::from(settings.snap),
            pad: 0,
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
    }

    /// Draw into the viewport at `corner`, which must lie inside the target.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, view: &View, corner: UVec2) {
        pass.set_viewport(
            corner.x as f32,
            corner.y as f32,
            view.size.x as f32,
            view.size.y as f32,
            0.0,
            1.0,
        );
        pass.set_scissor_rect(corner.x, corner.y, view.size.x, view.size.y);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    fn rebind(&mut self, device: &wgpu::Device) {
        self.bind_group = bind(
            device,
            &self.layout,
            &self.params,
            [&self.layers, &self.alphas],
            &self.masks,
            &self.sampler,
        );
    }
}

fn upload_alphas(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    alphas: &[AlphaMap; 3],
) -> [wgpu::TextureView; 3] {
    alphas.each_ref().map(|alpha| {
        let size = wgpu::Extent3d {
            width: alpha.width,
            height: alpha.height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("alpha"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            &alpha.values,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(alpha.width),
                rows_per_image: Some(alpha.height),
            },
            size,
        );
        texture.create_view(&wgpu::TextureViewDescriptor::default())
    })
}

fn upload_layers(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layers: &[RgbaImage; 3],
) -> [wgpu::TextureView; 3] {
    layers.each_ref().map(|image| {
        let size = wgpu::Extent3d {
            width: image.width(),
            height: image.height(),
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("layer"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Not sRGB: the bytes go to a non-sRGB target unchanged.
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            image.as_raw(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(image.width() * 4),
                rows_per_image: Some(image.height()),
            },
            size,
        );
        texture.create_view(&wgpu::TextureViewDescriptor::default())
    })
}

fn create_masks(device: &wgpu::Device, side: u32) -> [wgpu::Texture; 2] {
    std::array::from_fn(|_| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mask"),
            size: wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    })
}

fn bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    params: &wgpu::Buffer,
    [layers, alphas]: [&[wgpu::TextureView; 3]; 2],
    masks: &[wgpu::Texture; 2],
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    let mask_views = masks
        .each_ref()
        .map(|mask| mask.create_view(&wgpu::TextureViewDescriptor::default()));
    let view = |binding, view| wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    };
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("terrain"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: params.as_entire_binding(),
            },
            view(1, &layers[0]),
            view(2, &layers[1]),
            view(3, &layers[2]),
            view(4, &alphas[0]),
            view(5, &alphas[1]),
            view(6, &alphas[2]),
            view(7, &mask_views[0]),
            view(8, &mask_views[1]),
            wgpu::BindGroupEntry {
                binding: 9,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

/// Krust's instance: DirectX 12 on Windows unless WGPU_BACKEND names another, with FXC, since
/// no dxcompiler.dll ships beside this executable.
pub fn instance() -> wgpu::Instance {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
    if cfg!(windows) && wgpu::Backends::from_env().is_none() {
        descriptor.backends = wgpu::Backends::DX12;
    }
    descriptor.backend_options.dx12.shader_compiler = wgpu::Dx12Compiler::Fxc;
    wgpu::Instance::new(descriptor)
}

pub async fn device(
    instance: &wgpu::Instance,
    surface: Option<&wgpu::Surface<'_>>,
) -> anyhow::Result<(wgpu::Adapter, wgpu::Device, wgpu::Queue)> {
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: surface,
            ..Default::default()
        })
        .await
        .context("no compatible graphics adapter")?;
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("blendtest"),
            ..Default::default()
        })
        .await
        .context("request device")?;
    Ok((adapter, device, queue))
}

/// Read an RGBA8 or BGRA8 texture back as RGBA.
pub fn read_back(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> anyhow::Result<RgbaImage> {
    let (width, height) = (texture.width(), texture.height());
    let unpadded = width * 4;
    let padded =
        unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("read back"),
        size: u64::from(padded * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("read back"),
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| anyhow!("wait for the read back: {e:?}"))?;
    rx.recv().context("map callback dropped")??;
    let swap_red_blue = matches!(
        texture.format(),
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
    );
    let mut rgba = Vec::with_capacity((unpadded * height) as usize);
    {
        let data = slice
            .get_mapped_range()
            .map_err(|e| anyhow!("map the read back buffer: {e:?}"))?;
        for row in data.chunks(padded as usize) {
            for pixel in row[..unpadded as usize].chunks(4) {
                if swap_red_blue {
                    rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
                } else {
                    rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
                }
            }
        }
    }
    buffer.unmap();
    RgbaImage::from_raw(width, height, rgba).context("read back size")
}

/// A device without a window, for screenshots and tests.
pub struct Headless {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl Headless {
    pub fn new() -> anyhow::Result<Self> {
        let instance = instance();
        let (adapter, device, queue) = pollster::block_on(device(&instance, None))?;
        let info = adapter.get_info();
        eprintln!("adapter: {} ({:?})", info.name, info.backend);
        Ok(Self { device, queue })
    }

    /// Draw the terrain into a texture of the view's size and read it back.
    pub fn render(
        &mut self,
        layers: &[RgbaImage; 3],
        alphas: &[AlphaMap; 3],
        masks: &[Mask; 2],
        view: &View,
        settings: &Settings,
    ) -> anyhow::Result<RgbaImage> {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let terrain = Terrain::new(&self.device, &self.queue, format, layers, alphas, masks);
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("headless"),
            size: wgpu::Extent3d {
                width: view.size.x,
                height: view.size.y,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
        terrain.prepare(&self.queue, view, UVec2::ZERO, settings);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("headless"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("terrain"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            terrain.draw(&mut pass, view, UVec2::ZERO);
        }
        self.queue.submit([encoder.finish()]);
        read_back(&self.device, &self.queue, &target)
    }
}
