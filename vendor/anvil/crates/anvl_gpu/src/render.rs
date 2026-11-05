//! wgpu renderer. Draws layers in order: each layer's rounded shapes, icons, then its text.
//!
//! Backdrop blur (ADL-V3 section 3.5): when a layer holds blur areas, the layers beneath it are
//! drawn to an offscreen texture first. That texture is blurred with two dual-Kawase downsamples
//! (to a quarter size) and one upsample (back to half), copied to the screen, and each blur area
//! then shows the blurred copy through a rounded mask before its layer draws. A frame without
//! blur draws straight to the screen in one pass, as before.

use std::ops::Range;
use std::sync::Arc;

use bytemuck::Pod;
use wgpu::CurrentSurfaceTexture;


use crate::draw::{DrawList, IconInstance, ImageInstance, PathInstance, RectInstance, TextureId, LAYER_COUNT, NO_CLIP};
use crate::icons::IconAtlas;
use crate::media::Frame;
use crate::glyphs::{clip_quad, GlyphAtlas, Lookup};
use crate::text::TextSystem;
use crate::color::Color;

const SHADER: &str = include_str!("shaders.wgsl");

/// Smallest instance buffer allocated, in bytes.
const MIN_INSTANCE_BYTES: u64 = 4096;

/// A picture on the GPU, with the bind group that exposes it to the image shader.
struct ImageTexture {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
}

/// Why the GPU could not be started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GpuError {
    /// No graphics adapter on this machine can draw into the window.
    NoAdapter,
    /// The window could not give the GPU a surface.
    Surface(String),
    /// The adapter would not make a device.
    Device(String),
}

impl std::fmt::Display for GpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GpuError::NoAdapter => write!(f, "no compatible GPU adapter found"),
            GpuError::Surface(e) => write!(f, "failed to create a GPU surface: {e}"),
            GpuError::Device(e) => write!(f, "failed to create a GPU device: {e}"),
        }
    }
}

impl std::error::Error for GpuError {}

pub struct Renderer {
    /// Kept so a new surface can be made on resume. Creating the instance again would start a
    /// new device.
    instance: wgpu::Instance,
    /// `None` for a headless renderer, which draws only with [`Renderer::render_to_frame`].
    surface: Option<wgpu::Surface<'static>>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    rect_pipeline: wgpu::RenderPipeline,
    path_pipeline: wgpu::RenderPipeline,
    path_buffer: wgpu::Buffer,
    path_capacity: u64,
    path_scratch: Vec<PathInstance>,
    icon_pipeline: wgpu::RenderPipeline,
    rect_buffer: wgpu::Buffer,
    rect_capacity: u64,
    icon_buffer: wgpu::Buffer,
    icon_capacity: u64,
    image_pipeline: wgpu::RenderPipeline,
    image_layout: wgpu::BindGroupLayout,
    image_buffer: wgpu::Buffer,
    image_capacity: u64,
    /// Every picture uploaded so far. `TextureId::index` points into this list.
    textures: Vec<Option<ImageTexture>>,
    /// Slots in `textures` freed by [`Renderer::release`], reused by the next upload.
    free_textures: Vec<u32>,
    /// Scratch space that concatenates every layer's instances for one upload.
    rect_scratch: Vec<RectInstance>,
    icon_scratch: Vec<IconInstance>,
    image_scratch: Vec<ImageInstance>,
    /// Glyphs: the CPU-side atlas, its texture and bind group, and the quads for this frame.
    glyphs: GlyphAtlas,
    glyph_texture: GlyphTexture,
    glyph_pipeline: wgpu::RenderPipeline,
    glyph_buffer: wgpu::Buffer,
    glyph_capacity: u64,
    glyph_scratch: Vec<IconInstance>,
    blur: BlurPipelines,
    blur_buffer: wgpu::Buffer,
    blur_capacity: u64,
    blur_scratch: Vec<RectInstance>,
    /// Offscreen targets, made on the first frame that needs them and again on resize.
    blur_targets: Option<BlurTargets>,
    /// Where the last [`Renderer::render`] spent its time.
    timings: RenderTimings,
    /// Persona patch: the off-screen target and read-back buffer of `render_to_frame` are kept
    /// between frames instead of being created and freed every time.
    offscreen: Option<(u32, u32, wgpu::Texture)>,
    readback: Option<(u64, wgpu::Buffer)>,
}

/// Where one [`Renderer::render`] spent its time, in seconds.
#[derive(Clone, Copy, Debug, Default)]
pub struct RenderTimings {
    /// Waiting for a swapchain image: with vsync, the wait for the display.
    pub acquire: f32,
    /// CPU work: glyphs, instance uploads, encoding, submit and present.
    pub work: f32,
}

/// The GPU copy of the glyph atlas: one coverage byte per pixel.
struct GlyphTexture {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

/// One frame's instances, uploaded and split by layer, ready to encode.
struct Prepared {
    rects: [Range<u32>; LAYER_COUNT],
    paths: [Range<u32>; LAYER_COUNT],
    icons: [Range<u32>; LAYER_COUNT],
    images: [Range<u32>; LAYER_COUNT],
    blurs: [Range<u32>; LAYER_COUNT],
    glyphs: [Range<u32>; LAYER_COUNT],
    /// The first layer with backdrop blur, if any.
    blur_from: Option<usize>,
}

/// The pipelines for backdrop blur.
struct BlurPipelines {
    blit: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    /// Rounded windows onto the blurred texture. Uses the rect instance layout.
    window: wgpu::RenderPipeline,
}

/// A texture the blur passes render to and then read from.
struct Target {
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
}

/// The offscreen chain for one window size: the scene at full size, a half and a quarter for
/// the downsamples, and the blurred result at half size.
struct BlurTargets {
    size: (u32, u32),
    scene: Target,
    half: Target,
    quarter: Target,
    blurred: Target,
}

impl Renderer {
    /// Starts the GPU on `window` (anything wgpu can draw into, such as a winit window) at
    /// `width` by `height` physical pixels.
    pub fn new(
        window: impl Into<wgpu::SurfaceTarget<'static>>,
        width: u32,
        height: u32,
        icons: &IconAtlas,
    ) -> Result<Self, GpuError> {
        let instance = make_instance();
        let surface = instance
            .create_surface(window)
            .map_err(|e| GpuError::Surface(e.to_string()))?;
        Self::build(instance, Some(surface), width, height, icons)
    }

    /// A renderer with no window, for tests and offscreen work. It draws with
    /// [`Renderer::render_to_frame`]. `None` when the machine has no usable GPU adapter.
    pub fn new_headless(width: u32, height: u32, icons: &IconAtlas) -> Option<Self> {
        Self::build(make_instance(), None, width, height, icons).ok()
    }

    fn build(
        instance: wgpu::Instance,
        surface: Option<wgpu::Surface<'static>>,
        width: u32,
        height: u32,
        icons: &IconAtlas,
    ) -> Result<Self, GpuError> {
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            // Persona patch: the chrome is a few small frames, so prefer the integrated GPU (less memory).
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: surface.as_ref(),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|_| GpuError::NoAdapter)?;

        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("anvil-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            // Small allocation blocks. The default (Performance) reserves large ones up front:
            // measured on DX12, that more than doubled an app's memory (`mem_probe` example).
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| GpuError::Device(e.to_string()))?;
        // Report every GPU error with its full message. Without this, a validation error only
        // shows up later as a bare "device lost" panic.
        device.on_uncaptured_error(Arc::new(|error| eprintln!("anvil: GPU error: {error}")));
        device.set_device_lost_callback(|reason, message| {
            eprintln!("anvil: GPU device lost ({reason:?}): {message}")
        });

        // Non-sRGB target: the colors we specify are the colors that reach the screen. A headless
        // renderer draws RGBA8, the same bytes a `Frame` holds.
        let (format, alpha_mode) = match &surface {
            Some(surface) => {
                let caps = surface.get_capabilities(&adapter);
                let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
                (format, caps.alpha_modes[0])
            }
            None => (wgpu::TextureFormat::Rgba8Unorm, wgpu::CompositeAlphaMode::Opaque),
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode,
            view_formats: vec![],
            // One frame in flight: what is shown is at most a frame behind input. A frame takes
            // about a millisecond, so there is no throughput to gain from queueing more.
            desired_maximum_frame_latency: 1,
            color_space: wgpu::SurfaceColorSpace::Srgb,
        };
        if let Some(surface) = &surface {
            surface.configure(&device, &config);
        }

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("anvil-globals"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Icon atlas texture.
        let icon_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("anvil-icon-atlas"),
            size: wgpu::Extent3d {
                width: icons.width,
                height: icons.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &icon_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &icons.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(icons.width * 4),
                rows_per_image: Some(icons.height),
            },
            wgpu::Extent3d {
                width: icons.width,
                height: icons.height,
                depth_or_array_layers: 1,
            },
        );
        let icon_view = icon_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("anvil-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("anvil-bind-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("anvil-bind-group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&icon_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("anvil-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("anvil-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let rect_pipeline = make_pipeline(
            &device,
            &pipeline_layout,
            &shader,
            "anvil-rect-pipeline",
            "vs_rect",
            "fs_rect",
            RectInstance::layout(),
            format,
        );
        let path_pipeline = make_pipeline(
            &device,
            &pipeline_layout,
            &shader,
            "anvil-path-pipeline",
            "vs_path",
            "fs_path",
            PathInstance::layout(),
            format,
        );
        let icon_pipeline = make_pipeline(
            &device,
            &pipeline_layout,
            &shader,
            "anvil-icon-pipeline",
            "vs_icon",
            "fs_icon",
            IconInstance::layout(),
            format,
        );

        // Pictures bind their own texture in group 1. The sampler stays in group 0.
        let image_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("anvil-image-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let image_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("anvil-image-pipeline-layout"),
                bind_group_layouts: &[Some(&layout), Some(&image_layout)],
                immediate_size: 0,
            });
        let image_pipeline = make_pipeline(
            &device,
            &image_pipeline_layout,
            &shader,
            "anvil-image-pipeline",
            "vs_image",
            "fs_image",
            ImageInstance::layout(),
            format,
        );
        // The blur passes read a texture bound in group 1, like images, and write the surface
        // format, so the scene can be copied to the screen unchanged.
        let full = |label: &str, fs: &str, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&image_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_full"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let blur = BlurPipelines {
            blit: full("anvil-blit", "fs_blit", None),
            down: full("anvil-blur-down", "fs_down", None),
            up: full("anvil-blur-up", "fs_up", None),
            window: make_pipeline(
                &device,
                &image_pipeline_layout,
                &shader,
                "anvil-blur-window",
                "vs_rect",
                "fs_blur",
                RectInstance::layout(),
                format,
            ),
        };

        let rect_capacity = MIN_INSTANCE_BYTES;
        let rect_buffer = instance_buffer(&device, "anvil-rects", rect_capacity);
        let icon_capacity = MIN_INSTANCE_BYTES;
        let icon_buffer = instance_buffer(&device, "anvil-icons", icon_capacity);
        let image_capacity = MIN_INSTANCE_BYTES;
        let image_buffer = instance_buffer(&device, "anvil-images", image_capacity);
        let path_capacity = MIN_INSTANCE_BYTES;
        let path_buffer = instance_buffer(&device, "anvil-paths", path_capacity);
        let blur_capacity = MIN_INSTANCE_BYTES;
        let blur_buffer = instance_buffer(&device, "anvil-blurs", blur_capacity);
        // Text: our own glyph atlas, drawn as coverage masks tinted per glyph. The surface is
        // non-sRGB, so colors reach the screen exactly as given (#71717a stays #71717a).
        let glyph_pipeline = make_pipeline(
            &device,
            &image_pipeline_layout,
            &shader,
            "anvil-glyph-pipeline",
            "vs_glyph",
            "fs_glyph",
            IconInstance::layout(),
            format,
        );
        let glyphs = GlyphAtlas::default();
        let glyph_texture = make_glyph_texture(&device, &image_layout, glyphs.size());
        let glyph_capacity = MIN_INSTANCE_BYTES;
        let glyph_buffer = instance_buffer(&device, "anvil-glyphs", glyph_capacity);

        Ok(Self {
            instance,
            surface,
            device,
            queue,
            config,
            globals,
            bind_group,
            rect_pipeline,
            path_pipeline,
            path_buffer,
            path_capacity,
            path_scratch: Vec::new(),
            icon_pipeline,
            rect_buffer,
            rect_capacity,
            icon_buffer,
            icon_capacity,
            image_pipeline,
            image_layout,
            image_buffer,
            image_capacity,
            textures: Vec::new(),
            free_textures: Vec::new(),
            rect_scratch: Vec::new(),
            icon_scratch: Vec::new(),
            image_scratch: Vec::new(),
            glyphs,
            glyph_texture,
            glyph_pipeline,
            glyph_buffer,
            glyph_capacity,
            glyph_scratch: Vec::new(),
            blur,
            blur_buffer,
            blur_capacity,
            blur_scratch: Vec::new(),
            blur_targets: None,
            timings: RenderTimings::default(),
            offscreen: None,
            readback: None,
        })
    }

    /// Makes (or remakes, after a resize) the offscreen textures the blur passes use.
    fn ensure_blur_targets(&mut self) {
        let (w, h) = (self.config.width.max(1), self.config.height.max(1));
        if self.blur_targets.as_ref().is_some_and(|t| t.size == (w, h)) {
            return;
        }
        let make = |label: &str, tw: u32, th: u32| {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: tw.max(1),
                    height: th.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.config.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &self.image_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                }],
            });
            Target { view, bind_group }
        };
        self.blur_targets = Some(BlurTargets {
            size: (w, h),
            scene: make("anvil-blur-scene", w, h),
            half: make("anvil-blur-half", w / 2, h / 2),
            quarter: make("anvil-blur-quarter", w / 4, h / 4),
            blurred: make("anvil-blur-result", w / 2, h / 2),
        });
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.reconfigure();
    }

    pub fn reconfigure(&mut self) {
        if let Some(surface) = &self.surface {
            surface.configure(&self.device, &self.config);
        }
    }

    /// Replaces the surface with one for `window`, and sizes it to the window. Android hands the
    /// app a new surface each time it comes back to the foreground. The device and the pipelines
    /// are kept, so nothing is compiled again.
    pub fn recreate_surface(
        &mut self,
        window: impl Into<wgpu::SurfaceTarget<'static>>,
        width: u32,
        height: u32,
    ) -> Result<(), GpuError> {
        let surface = self
            .instance
            .create_surface(window)
            .map_err(|e| GpuError::Surface(e.to_string()))?;
        self.surface = Some(surface);
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.reconfigure();
        Ok(())
    }

    /// Puts a picture on the GPU and returns the id that `Ui::image` takes.
    pub fn upload(&mut self, frame: &Frame) -> TextureId {
        let texture = Some(self.create_image_texture(frame.width, frame.height));
        let index = match self.free_textures.pop() {
            Some(index) => {
                self.textures[index as usize] = texture;
                index
            }
            None => {
                self.textures.push(texture);
                (self.textures.len() - 1) as u32
            }
        };
        self.write_image(index, frame);
        TextureId {
            index,
            width: frame.width,
            height: frame.height,
        }
    }

    /// Frees the GPU memory of an uploaded picture. Its slot is reused by a later upload, so the
    /// id must not be drawn or updated afterwards. Releasing an id twice does nothing.
    pub fn release(&mut self, id: TextureId) {
        if let Some(slot) = self.textures.get_mut(id.index as usize) {
            if slot.take().is_some() {
                self.free_textures.push(id.index);
            }
        }
    }

    /// Replaces the pixels of an uploaded picture, such as the next video frame. A frame of a
    /// different size is ignored, because the texture keeps its size.
    pub fn update(&mut self, id: TextureId, frame: &Frame) {
        let Some(Some(texture)) = self.textures.get(id.index as usize) else {
            return;
        };
        if texture.width == frame.width && texture.height == frame.height {
            self.write_image(id.index, frame);
        }
    }

    fn create_image_texture(&self, width: u32, height: u32) -> ImageTexture {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("anvil-image"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("anvil-image-bind-group"),
            layout: &self.image_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        ImageTexture {
            texture,
            bind_group,
            width,
            height,
        }
    }

    fn write_image(&self, index: u32, frame: &Frame) {
        let Some(Some(slot)) = self.textures.get(index as usize) else {
            return;
        };
        let texture = &slot.texture;
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &frame.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(frame.width * 4),
                rows_per_image: Some(frame.height),
            },
            wgpu::Extent3d {
                width: frame.width,
                height: frame.height,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Draws one frame to the window. `text` supplies the shaped buffers referenced by `draw`.
    pub fn render(&mut self, draw: &DrawList, text: &mut TextSystem, clear: Color) -> FrameStatus {
        let Some(surface) = &self.surface else {
            return FrameStatus::Skipped;
        };
        let start = std::time::Instant::now();
        let frame = match surface.get_current_texture() {
            CurrentSurfaceTexture::Success(frame) | CurrentSurfaceTexture::Suboptimal(frame) => frame,
            CurrentSurfaceTexture::Timeout | CurrentSurfaceTexture::Occluded => return FrameStatus::Skipped,
            CurrentSurfaceTexture::Outdated | CurrentSurfaceTexture::Lost | CurrentSurfaceTexture::Validation => {
                return FrameStatus::Reconfigure;
            }
        };
        let acquired = std::time::Instant::now();
        let prepared = self.prepare(draw, text);
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("anvil-encoder"),
        });
        self.encode(&mut encoder, &view, draw, clear, &prepared);
        self.queue.submit(std::iter::once(encoder.finish()));
        self.queue.present(frame);
        self.timings = RenderTimings {
            acquire: (acquired - start).as_secs_f32(),
            work: acquired.elapsed().as_secs_f32(),
        };
        FrameStatus::Presented
    }

    /// Where the last [`Renderer::render`] spent its time.
    pub fn timings(&self) -> RenderTimings {
        self.timings
    }

    /// Draws one frame into memory and returns its pixels: RGBA8, straight alpha, top row first.
    /// Works with or without a window. Tests use it to check what a frame looks like.
    pub fn render_to_frame(&mut self, draw: &DrawList, text: &mut TextSystem, clear: Color) -> Frame {
        let (w, h) = (self.config.width.max(1), self.config.height.max(1));
        let prepared = self.prepare(draw, text);
        if self.offscreen.as_ref().is_none_or(|(ow, oh, _)| (*ow, *oh) != (w, h)) {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("anvil-offscreen"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.config.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            self.offscreen = Some((w, h, texture));
        }
        let target = match &self.offscreen {
            Some((_, _, t)) => t.clone(),
            None => return Frame { width: w, height: h, rgba: vec![0; (w * h * 4) as usize] },
        };
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("anvil-offscreen-encoder"),
        });
        self.encode(&mut encoder, &view, draw, clear, &prepared);
        // Rows of a texture copy are padded to 256 bytes.
        let row = w * 4;
        let padded = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let size = (padded * h) as u64;
        if self.readback.as_ref().is_none_or(|(sz, _)| *sz != size) {
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("anvil-readback"),
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            self.readback = Some((size, buffer));
        }
        let readback = match &self.readback {
            Some((_, b)) => b.clone(),
            None => return Frame { width: w, height: h, rgba: vec![0; (w * h * 4) as usize] },
        };
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(std::iter::once(encoder.finish()));
        readback.map_async(wgpu::MapMode::Read, .., |_| {});
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        let mut rgba = Vec::with_capacity((row * h) as usize);
        match readback.get_mapped_range(..) {
            Ok(data) => {
                for y in 0..h {
                    let start = (y * padded) as usize;
                    rgba.extend_from_slice(&data[start..start + row as usize]);
                }
            }
            Err(e) => {
                // The copy did not land. Return a black frame instead of crashing the host.
                eprintln!("anvil: could not read back the frame: {e}");
                rgba.resize((row * h) as usize, 0);
            }
        }
        readback.unmap();
        if matches!(self.config.format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb) {
            for px in rgba.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
        }
        Frame { width: w, height: h, rgba }
    }

    /// Uploads one frame's instances and glyphs. Shared by both ways of drawing a frame.
    fn prepare(&mut self, draw: &DrawList, text: &mut TextSystem) -> Prepared {
        let (w, h) = (self.config.width, self.config.height);
        let globals = [w as f32, h as f32, 0.0, 0.0];
        self.queue
            .write_buffer(&self.globals, 0, bytemuck::cast_slice(&globals));

        // Concatenate every layer's instances. Each layer then draws its own range.
        self.rect_scratch.clear();
        self.icon_scratch.clear();
        self.image_scratch.clear();
        let mut rect_ranges: [Range<u32>; LAYER_COUNT] = Default::default();
        let mut path_ranges: [Range<u32>; LAYER_COUNT] = Default::default();
        self.path_scratch.clear();
        let mut icon_ranges: [Range<u32>; LAYER_COUNT] = Default::default();
        let mut image_ranges: [Range<u32>; LAYER_COUNT] = Default::default();
        let mut blur_ranges: [Range<u32>; LAYER_COUNT] = Default::default();
        self.blur_scratch.clear();
        for (i, layer) in draw.layers.iter().enumerate() {
            let start = self.blur_scratch.len() as u32;
            self.blur_scratch.extend_from_slice(&layer.blurs);
            blur_ranges[i] = start..self.blur_scratch.len() as u32;

            let start = self.rect_scratch.len() as u32;
            self.rect_scratch.extend_from_slice(&layer.rects);
            rect_ranges[i] = start..self.rect_scratch.len() as u32;

            let start = self.path_scratch.len() as u32;
            self.path_scratch.extend_from_slice(&layer.paths);
            path_ranges[i] = start..self.path_scratch.len() as u32;

            let start = self.image_scratch.len() as u32;
            self.image_scratch
                .extend(layer.images.iter().map(|cmd| cmd.instance));
            image_ranges[i] = start..self.image_scratch.len() as u32;

            let start = self.icon_scratch.len() as u32;
            self.icon_scratch.extend_from_slice(&layer.icons);
            icon_ranges[i] = start..self.icon_scratch.len() as u32;
        }
        write_instances(
            &self.device,
            &self.queue,
            &mut self.image_buffer,
            &mut self.image_capacity,
            "anvil-images",
            &self.image_scratch,
        );
        write_instances(
            &self.device,
            &self.queue,
            &mut self.rect_buffer,
            &mut self.rect_capacity,
            "anvil-rects",
            &self.rect_scratch,
        );
        write_instances(
            &self.device,
            &self.queue,
            &mut self.icon_buffer,
            &mut self.icon_capacity,
            "anvil-icons",
            &self.icon_scratch,
        );
        write_instances(
            &self.device,
            &self.queue,
            &mut self.path_buffer,
            &mut self.path_capacity,
            "anvil-paths",
            &self.path_scratch,
        );
        write_instances(
            &self.device,
            &self.queue,
            &mut self.blur_buffer,
            &mut self.blur_capacity,
            "anvil-blurs",
            &self.blur_scratch,
        );
        // Layers below this one are blurred behind it. `None` keeps the single-pass path.
        let blur_from = draw.first_blur_layer();
        if blur_from.is_some() {
            self.ensure_blur_targets();
        }

        // Text: every glyph of every layer becomes a quad over its slot in the glyph atlas.
        let glyph_ranges = self.layout_glyphs(draw, text);
        write_instances(
            &self.device,
            &self.queue,
            &mut self.glyph_buffer,
            &mut self.glyph_capacity,
            "anvil-glyphs",
            &self.glyph_scratch,
        );

        Prepared {
            rects: rect_ranges,
            paths: path_ranges,
            icons: icon_ranges,
            images: image_ranges,
            blurs: blur_ranges,
            glyphs: glyph_ranges,
            blur_from,
        }
    }

    /// Records the passes that draw a prepared frame into `view`.
    fn encode(&self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, draw: &DrawList, clear: Color, p: &Prepared) {
        let (blur_from, blur_ranges) = (p.blur_from, &p.blurs);
        let clear_color = wgpu::Color {
            r: clear[0] as f64,
            g: clear[1] as f64,
            b: clear[2] as f64,
            a: clear[3] as f64,
        };
        let first_on_screen = match (blur_from, self.blur_targets.as_ref()) {
            (Some(k), Some(t)) => {
                // 1. The layers beneath the first blur, to the offscreen scene.
                {
                    let mut pass = begin_pass(encoder, &t.scene.view, wgpu::LoadOp::Clear(clear_color));
                    for i in 0..k {
                        self.draw_layer(&mut pass, draw, i, p);
                    }
                }
                // 2. Blur it: scene -> half -> quarter (down), quarter -> blurred (up, half size).
                for (pipeline, src, dst) in [
                    (&self.blur.down, &t.scene, &t.half),
                    (&self.blur.down, &t.half, &t.quarter),
                    (&self.blur.up, &t.quarter, &t.blurred),
                ] {
                    let mut pass = begin_pass(encoder, &dst.view, wgpu::LoadOp::Clear(wgpu::Color::BLACK));
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(0, &self.bind_group, &[]);
                    pass.set_bind_group(1, &src.bind_group, &[]);
                    pass.draw(0..3, 0..1);
                }
                k
            }
            _ => 0,
        };
        {
            let mut pass = begin_pass(encoder, view, wgpu::LoadOp::Clear(clear_color));
            if let (Some(t), true) = (self.blur_targets.as_ref(), first_on_screen > 0) {
                // 3. The sharp scene, copied to the screen.
                pass.set_pipeline(&self.blur.blit);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_bind_group(1, &t.scene.bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
            for (i, blurs) in blur_ranges.iter().enumerate().skip(first_on_screen) {
                // 4. Each layer's blur windows go under its own shapes.
                let blurs = blurs.clone();
                if let (Some(t), false) = (self.blur_targets.as_ref(), blurs.is_empty() || blur_from.is_none()) {
                    pass.set_pipeline(&self.blur.window);
                    pass.set_bind_group(0, &self.bind_group, &[]);
                    pass.set_bind_group(1, &t.blurred.bind_group, &[]);
                    pass.set_vertex_buffer(0, self.blur_buffer.slice(..));
                    pass.draw(0..6, blurs);
                }
                self.draw_layer(&mut pass, draw, i, p);
            }
        }
    }
}

impl Renderer {
    /// Builds this frame's glyph quads in `glyph_scratch`, one range per layer, rasterizing new
    /// glyphs into the atlas and uploading them. If the atlas fills up, it grows (or starts over
    /// at its largest size) and the frame's glyphs are laid out again.
    fn layout_glyphs(&mut self, draw: &DrawList, text: &mut TextSystem) -> [Range<u32>; LAYER_COUNT] {
        let (fonts, swash, buffers) = text.parts();
        let mut ranges: [Range<u32>; LAYER_COUNT] = Default::default();
        for _ in 0..3 {
            self.glyph_scratch.clear();
            let size = self.glyphs.size() as f32;
            let mut full = false;
            'layers: for (i, layer) in draw.layers.iter().enumerate() {
                let start = self.glyph_scratch.len() as u32;
                for cmd in &layer.texts {
                    let Some(cached) = buffers.get(&cmd.key) else {
                        continue;
                    };
                    let clip = [cmd.clip.x(), cmd.clip.y(), cmd.clip.right(), cmd.clip.bottom()];
                    for run in cached.buffer.layout_runs() {
                        let line_y = run.line_y.round() as i32;
                        for g in run.glyphs {
                            let pg = g.physical((cmd.x, cmd.y), 1.0);
                            let slot = match self.glyphs.lookup(pg.cache_key, fonts, swash) {
                                Lookup::Ready(Some(slot)) => slot,
                                Lookup::Ready(None) => continue,
                                Lookup::Full => {
                                    full = true;
                                    break 'layers;
                                }
                            };
                            let x = (pg.x + slot.left) as f32;
                            let y = (line_y + pg.y - slot.top) as f32;
                            let quad = [x, y, x + slot.w as f32, y + slot.h as f32];
                            let uv = [
                                slot.x as f32 / size,
                                slot.y as f32 / size,
                                (slot.x + slot.w) as f32 / size,
                                (slot.y + slot.h) as f32 / size,
                            ];
                            let Some((q, uv)) = clip_quad(quad, uv, clip) else {
                                continue;
                            };
                            // A styled span carries its own color; plain text takes the command's.
                            let tint = match g.color_opt {
                                Some(c) => [
                                    c.r() as f32 / 255.0,
                                    c.g() as f32 / 255.0,
                                    c.b() as f32 / 255.0,
                                    c.a() as f32 / 255.0,
                                ],
                                None => cmd.color,
                            };
                            self.glyph_scratch.push(IconInstance {
                                rect: [q[0], q[1], q[2] - q[0], q[3] - q[1]],
                                uv,
                                tint,
                                params: [0.0; 4],
                                clip: NO_CLIP,
                            });
                        }
                    }
                }
                ranges[i] = start..self.glyph_scratch.len() as u32;
            }
            if !full {
                break;
            }
            // Full: a bigger texture if there is room to grow, otherwise start over. Either way
            // only this frame's glyphs come back.
            let grew = self.glyphs.grow();
            self.glyphs.reset();
            if grew {
                self.glyph_texture = make_glyph_texture(&self.device, &self.image_layout, self.glyphs.size());
            }
            eprintln!("anvil: glyph atlas full, now {} px", self.glyphs.size());
        }
        for (slot, coverage) in self.glyphs.pending.drain(..) {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.glyph_texture.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: slot.x, y: slot.y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &coverage,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(slot.w),
                    rows_per_image: Some(slot.h),
                },
                wgpu::Extent3d {
                    width: slot.w,
                    height: slot.h,
                    depth_or_array_layers: 1,
                },
            );
        }
        ranges
    }

    /// Draws one layer into `pass`: shapes, pictures, icons, then text.
    fn draw_layer<'p>(&'p self, pass: &mut wgpu::RenderPass<'p>, draw: &DrawList, i: usize, p: &Prepared) {
        let rects = p.rects[i].clone();
        if !rects.is_empty() {
            pass.set_pipeline(&self.rect_pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.rect_buffer.slice(..));
            pass.draw(0..6, rects);
        }
        let paths = p.paths[i].clone();
        if !paths.is_empty() {
            pass.set_pipeline(&self.path_pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.path_buffer.slice(..));
            pass.draw(0..6, paths);
        }
        // Each picture is its own draw, with its texture bound in group 1.
        for (k, cmd) in draw.layers[i].images.iter().enumerate() {
            let Some(Some(texture)) = self.textures.get(cmd.texture as usize) else {
                continue;
            };
            let index = p.images[i].start + k as u32;
            pass.set_pipeline(&self.image_pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_bind_group(1, &texture.bind_group, &[]);
            pass.set_vertex_buffer(0, self.image_buffer.slice(..));
            pass.draw(0..6, index..index + 1);
        }
        let icons = p.icons[i].clone();
        if !icons.is_empty() {
            pass.set_pipeline(&self.icon_pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.icon_buffer.slice(..));
            pass.draw(0..6, icons);
        }
        let glyphs = p.glyphs[i].clone();
        if !glyphs.is_empty() {
            pass.set_pipeline(&self.glyph_pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_bind_group(1, &self.glyph_texture.bind_group, &[]);
            pass.set_vertex_buffer(0, self.glyph_buffer.slice(..));
            pass.draw(0..6, glyphs);
        }
    }
}

/// An empty glyph atlas texture of `size` by `size` coverage bytes.
fn make_glyph_texture(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, size: u32) -> GlyphTexture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("anvil-glyph-atlas"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("anvil-glyph-bind-group"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(&view),
        }],
    });
    GlyphTexture { texture, bind_group }
}

/// Runs a future to completion on this thread. wgpu's adapter and device requests are the only
/// futures the renderer waits on, and they finish without needing an executor.
pub fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};
    struct Unpark(std::thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut fut = std::pin::pin!(fut);
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::thread::park(),
        }
    }
}

/// A wgpu instance on every backend. Debug builds turn on InstanceFlags::DEBUG, and wgpu then
/// passes every shader's source to the driver as debug info. The PowerVR shader compiler aborts
/// on that, so Android builds leave the debug flags off.
fn make_instance() -> wgpu::Instance {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::all();
    desc.flags = if cfg!(target_os = "android") {
        wgpu::InstanceFlags::empty()
    } else {
        wgpu::InstanceFlags::from_build_config()
    };
    wgpu::Instance::new(desc)
}

/// Starts a render pass that writes `view`.
fn begin_pass<'e>(
    encoder: &'e mut wgpu::CommandEncoder,
    view: &'e wgpu::TextureView,
    load: wgpu::LoadOp<wgpu::Color>,
) -> wgpu::RenderPass<'e> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("anvil-pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

/// What happened to one frame. The app reconfigures the surface when the GPU asks for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameStatus {
    Presented,
    /// The surface had no image this time, such as while minimized. Nothing was drawn.
    Skipped,
    /// The surface changed or was lost, and must be configured again before the next frame.
    Reconfigure,
}

fn instance_buffer(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Uploads instance data, growing the GPU buffer to the next power of two when needed.
fn write_instances<T: Pod>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    buffer: &mut wgpu::Buffer,
    capacity: &mut u64,
    label: &str,
    data: &[T],
) {
    let bytes: &[u8] = bytemuck::cast_slice(data);
    if bytes.is_empty() {
        return;
    }
    if bytes.len() as u64 > *capacity {
        *capacity = (bytes.len() as u64).next_power_of_two();
        *buffer = instance_buffer(device, label, *capacity);
    }
    queue.write_buffer(buffer, 0, bytes);
}

#[allow(clippy::too_many_arguments)]
fn make_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    label: &str,
    vs: &str,
    fs: &str,
    instance: wgpu::VertexBufferLayout<'_>,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(vs),
            buffers: &[Some(instance)],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fs),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

impl RectInstance {
    // Seven vec4 fields, matching the 112-byte struct.
    const ATTRIBS: [wgpu::VertexAttribute; 7] = wgpu::vertex_attr_array![
        0 => Float32x4,
        1 => Float32x4,
        2 => Float32x4,
        3 => Float32x4,
        4 => Float32x4,
        5 => Float32x4,
        6 => Float32x4,
    ];

    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<RectInstance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBS,
        }
    }
}

impl ImageInstance {
    const ATTRIBS: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
        0 => Float32x4,
        1 => Float32x4,
        2 => Float32x4,
        3 => Float32x4,
        4 => Float32x4,
    ];

    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<ImageInstance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBS,
        }
    }
}

impl PathInstance {
    const ATTRIBS: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
        0 => Float32x4,
        1 => Float32x4,
        2 => Float32x4,
        3 => Float32x4,
    ];

    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<PathInstance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBS,
        }
    }
}

impl IconInstance {
    const ATTRIBS: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
        0 => Float32x4,
        1 => Float32x4,
        2 => Float32x4,
        3 => Float32x4,
        4 => Float32x4,
    ];

    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<IconInstance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBS,
        }
    }
}

