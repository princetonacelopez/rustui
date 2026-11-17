//! wgpu + Vello rendering backend with batched WGSL SDF pipeline, GPU resource retention,
//! and deterministic software offscreen fallback.

use std::collections::HashMap;
use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};
use rustui_core::{Color, CornerRadii, Point, Rect};
use vello::kurbo::{Affine, BezPath, Point as KPoint, Stroke};
use vello::peniko::{Brush, Color as VColor, Fill};

use crate::atlas::GlyphAtlas;
use crate::display_list::{
    DisplayCommand, DisplayList, FontBlob, GradientKind, GradientStop, ImageId, PathOp,
};
use crate::resources::ResourceLoader;

const SDF_SHADER_WGSL: &str = r#"
struct Uniforms {
    viewport: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(1) @binding(0) var tex_sampler: sampler;
@group(1) @binding(1) var sprite_tex: texture_2d<f32>;

struct InstanceIn {
    @location(0) rect: vec4<f32>,
    @location(1) radii: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) color2: vec4<f32>,
    @location(4) uv_rect: vec4<f32>,
    @location(5) params: vec4<f32>,
    @location(6) params2: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) radii: vec4<f32>,
    @location(3) color: vec4<f32>,
    @location(4) color2: vec4<f32>,
    @location(5) uv: vec2<f32>,
    @location(6) params: vec4<f32>,
    @location(7) params2: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vid: u32, inst: InstanceIn) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    let c = corners[vid];
    let expand = select(0.0, inst.params.y * 2.0, inst.params.x == 2.0);
    let origin = inst.rect.xy - vec2<f32>(expand, expand);
    let quad_size = inst.rect.zw + vec2<f32>(expand * 2.0, expand * 2.0);
    let px = origin + c * quad_size;

    let ndc_x = (px.x / max(u.viewport.x, 1.0)) * 2.0 - 1.0;
    let ndc_y = 1.0 - (px.y / max(u.viewport.y, 1.0)) * 2.0;

    var out: VsOut;
    out.clip_pos = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.local_pos = px - inst.rect.xy;
    out.size = inst.rect.zw;
    out.radii = inst.radii;
    out.color = inst.color;
    out.color2 = inst.color2;
    out.uv = inst.uv_rect.xy + c * inst.uv_rect.zw;
    out.params = inst.params;
    out.params2 = inst.params2;
    return out;
}

fn sd_rounded_box(p: vec2<f32>, half_size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let r_right = select(radii.z, radii.y, p.y < 0.0);
    let r_left = select(radii.w, radii.x, p.y < 0.0);
    let r = select(r_left, r_right, p.x >= 0.0);
    let q = abs(p) - half_size + vec2<f32>(r, r);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0))) - r;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let mode = u32(in.params.x + 0.5);
    let opacity = in.params2.z;
    let half = in.size * 0.5;
    let centered = in.local_pos - half;
    let d = sd_rounded_box(centered, half, in.radii);

    if (mode == 0u) {
        let cov = smoothstep(0.65, -0.65, d);
        return vec4<f32>(in.color.rgb, in.color.a * cov * opacity);
    } else if (mode == 1u) {
        let w = max(in.params.y, 0.5);
        let ring = abs(d + w * 0.5) - w * 0.5;
        let cov = smoothstep(0.65, -0.65, ring);
        return vec4<f32>(in.color.rgb, in.color.a * cov * opacity);
    } else if (mode == 2u) {
        let blur = max(in.params.y, 1.0);
        let cov = 1.0 - smoothstep(-blur * 0.5, blur * 1.4, d);
        return vec4<f32>(in.color.rgb, in.color.a * cov * opacity);
    } else if (mode == 3u) {
        let cov = smoothstep(0.65, -0.65, d);
        let p0 = in.params.zw;
        let p1 = in.params2.xy;
        let delta = p1 - p0;
        let denom = max(dot(delta, delta), 0.0001);
        let t = clamp(dot(in.local_pos - p0, delta) / denom, 0.0, 1.0);
        let col = mix(in.color, in.color2, t);
        return vec4<f32>(col.rgb, col.a * cov * opacity);
    } else if (mode == 4u) {
        let cov = smoothstep(0.65, -0.65, d);
        let center = in.params.zw;
        let radius = max(in.params2.x, 1.0);
        let t = clamp(length(in.local_pos - center) / radius, 0.0, 1.0);
        let col = mix(in.color, in.color2, t);
        return vec4<f32>(col.rgb, col.a * cov * opacity);
    } else if (mode == 5u) {
        let sample = textureSample(sprite_tex, tex_sampler, in.uv);
        let is_color = in.params.y > 0.5;
        if (is_color) {
            return vec4<f32>(sample.rgb, sample.a * opacity);
        } else {
            return vec4<f32>(in.color.rgb, in.color.a * sample.a * opacity);
        }
    } else {
        let sample = textureSample(sprite_tex, tex_sampler, in.uv);
        return vec4<f32>(sample.rgb * in.color.rgb, sample.a * in.color.a * opacity);
    }
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct UniformData {
    viewport: [f32; 2],
    _pad: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SdfInstance {
    rect: [f32; 4],
    radii: [f32; 4],
    color: [f32; 4],
    color2: [f32; 4],
    uv_rect: [f32; 4],
    params: [f32; 4],
    params2: [f32; 4],
}

impl SdfInstance {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x4,
            1 => Float32x4,
            2 => Float32x4,
            3 => Float32x4,
            4 => Float32x4,
            5 => Float32x4,
            6 => Float32x4,
        ],
    };
}

/// Platform-appropriate wgpu backend priority list matching Section 6.
pub fn platform_backends() -> wgpu::Backends {
    #[cfg(target_os = "macos")]
    {
        wgpu::Backends::METAL
    }
    #[cfg(target_os = "windows")]
    {
        wgpu::Backends::DX12 | wgpu::Backends::VULKAN
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        wgpu::Backends::VULKAN | wgpu::Backends::GL
    }
}

/// Diagnostic information about available GPU adapters (`rustui doctor`).
#[derive(Debug, Clone)]
pub struct GpuDiagnostics {
    pub adapter_available: bool,
    pub adapter_name: String,
    pub backend: String,
    pub device_type: String,
    pub vello_supported: bool,
}

/// Probes available wgpu adapters on the current host.
pub fn probe_gpu_adapter() -> GpuDiagnostics {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: platform_backends(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        ..Default::default()
    })) {
        Ok(adapter) => {
            let info = adapter.get_info();
            let vello_supported = adapter.limits().max_compute_workgroups_per_dimension > 0;
            GpuDiagnostics {
                adapter_available: true,
                adapter_name: info.name,
                backend: format!("{:?}", info.backend),
                device_type: format!("{:?}", info.device_type),
                vello_supported,
            }
        }
        Err(_) => GpuDiagnostics {
            adapter_available: false,
            adapter_name: "Software Rasterizer Fallback".to_owned(),
            backend: "CPU".to_owned(),
            device_type: "Cpu".to_owned(),
            vello_supported: false,
        },
    }
}

struct GpuContext {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline_rgba: wgpu::RenderPipeline,
    pipeline_bgra: Option<wgpu::RenderPipeline>,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    tex_bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    white_tex_bind_group: wgpu::BindGroup,
    atlas_texture: wgpu::Texture,
    atlas_size: (u32, u32),
    atlas_bind_group: wgpu::BindGroup,
    image_textures: HashMap<ImageId, (wgpu::Texture, wgpu::BindGroup)>,
    vello_renderer: Option<vello::Renderer>,
}

/// Main renderer driving both live GPU window presentation and headless offscreen snapshots.
pub struct WgpuRenderer {
    instance: wgpu::Instance,
    gpu: Option<GpuContext>,
}

impl std::fmt::Debug for WgpuRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WgpuRenderer")
            .field("gpu_initialized", &self.gpu.is_some())
            .finish()
    }
}

impl Default for WgpuRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl WgpuRenderer {
    pub fn new() -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: platform_backends(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        Self {
            instance,
            gpu: None,
        }
    }

    pub fn wgpu_instance(&self) -> &wgpu::Instance {
        &self.instance
    }

    pub fn has_gpu(&self) -> bool {
        self.gpu.is_some()
    }

    /// Initializes the wgpu device/queue/pipelines, optionally matching a live window surface.
    pub fn ensure_gpu(&mut self, compatible_surface: Option<&wgpu::Surface<'_>>) -> bool {
        if self.gpu.is_some() {
            return true;
        }
        let Ok(adapter) =
            pollster::block_on(self.instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface,
                force_fallback_adapter: false,
                ..Default::default()
            }))
        else {
            tracing::debug!("wgpu adapter unavailable; using software fallback");
            return false;
        };

        let Ok((device, queue)) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("rustui-device"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            }))
        else {
            return false;
        };

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rustui-sdf-shader"),
            source: wgpu::ShaderSource::Wgsl(SDF_SHADER_WGSL.into()),
        });

        let uniform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("rustui-uniform-bgl"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(
                            std::mem::size_of::<UniformData>() as u64,
                        ),
                    },
                    count: None,
                }],
            });

        let tex_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("rustui-tex-bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
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
                ],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rustui-pipeline-layout"),
            bind_group_layouts: &[
                Some(&uniform_bind_group_layout),
                Some(&tex_bind_group_layout),
            ],
            immediate_size: 0,
        });

        let create_pipeline = |format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("rustui-sdf-pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[Some(SdfInstance::LAYOUT)],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                multiview_mask: None,
                cache: None,
            })
        };

        let pipeline_rgba = create_pipeline(wgpu::TextureFormat::Rgba8Unorm);
        let pipeline_bgra = Some(create_pipeline(wgpu::TextureFormat::Bgra8UnormSrgb));

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rustui-uniforms"),
            size: std::mem::size_of::<UniformData>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rustui-uniform-bg"),
            layout: &uniform_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("rustui-linear-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let (white_tex, white_tex_bind_group) = create_rgba_texture(
            &device,
            &queue,
            &tex_bind_group_layout,
            &sampler,
            1,
            1,
            &[255, 255, 255, 255],
            "rustui-white-tex",
        );
        let _ = white_tex;

        let (atlas_texture, atlas_bind_group) = create_rgba_texture(
            &device,
            &queue,
            &tex_bind_group_layout,
            &sampler,
            64,
            64,
            &vec![0u8; 64 * 64 * 4],
            "rustui-atlas-tex",
        );

        let vello_renderer = vello::Renderer::new(
            &device,
            vello::RendererOptions {
                use_cpu: false,
                antialiasing_support: vello::AaSupport::area_only(),
                num_init_threads: std::num::NonZeroUsize::new(1),
                pipeline_cache: None,
            },
        )
        .ok();

        self.gpu = Some(GpuContext {
            device,
            queue,
            pipeline_rgba,
            pipeline_bgra,
            uniform_buffer,
            uniform_bind_group,
            tex_bind_group_layout,
            sampler,
            white_tex_bind_group,
            atlas_texture,
            atlas_size: (64, 64),
            atlas_bind_group,
            image_textures: HashMap::new(),
            vello_renderer,
        });
        true
    }

    pub fn device_and_queue(&self) -> Option<(&wgpu::Device, &wgpu::Queue)> {
        self.gpu.as_ref().map(|g| (&g.device, &g.queue))
    }

    /// Renders `list` to a live window surface texture view.
    #[allow(clippy::too_many_arguments)]
    pub fn render_to_texture_view(
        &mut self,
        view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        list: &DisplayList,
        atlas: &mut GlyphAtlas,
        resources: &ResourceLoader,
        clear_color: Color,
    ) -> Result<(), String> {
        if !self.ensure_gpu(None) {
            return Err("GPU adapter unavailable".to_owned());
        }
        let gpu = self.gpu.as_mut().expect("gpu initialized");
        render_with_gpu(
            gpu,
            view,
            format,
            width.max(1),
            height.max(1),
            list,
            atlas,
            resources,
            clear_color,
        );
        Ok(())
    }

    /// Renders `list` into an offscreen RGBA8 buffer `[width * height * 4]`.
    ///
    /// Uses `wgpu` GPU rendering when available, falling back automatically to the deterministic
    /// CPU software rasterizer on headless VMs without a GPU driver.
    pub fn render_offscreen(
        &mut self,
        list: &DisplayList,
        atlas: &mut GlyphAtlas,
        resources: &ResourceLoader,
        width: u32,
        height: u32,
        clear_color: Color,
    ) -> Vec<u8> {
        let w = width.max(1);
        let h = height.max(1);
        let force_sw = std::env::var("RUSTUI_SOFTWARE_RENDER").is_ok_and(|v| v == "1");
        if !force_sw
            && self.ensure_gpu(None)
            && let Some(bytes) =
                self.render_offscreen_gpu(list, atlas, resources, w, h, clear_color)
        {
            return bytes;
        }
        SoftwareRasterizer::render(list, atlas, resources, w, h, clear_color)
    }

    fn render_offscreen_gpu(
        &mut self,
        list: &DisplayList,
        atlas: &mut GlyphAtlas,
        resources: &ResourceLoader,
        width: u32,
        height: u32,
        clear_color: Color,
    ) -> Option<Vec<u8>> {
        let gpu = self.gpu.as_mut()?;
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("rustui-offscreen-target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        render_with_gpu(
            gpu,
            &view,
            wgpu::TextureFormat::Rgba8Unorm,
            width,
            height,
            list,
            atlas,
            resources,
            clear_color,
        );

        let bytes_per_row = align_to(width * 4, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rustui-readback"),
            size: u64::from(bytes_per_row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("rustui-readback-encoder"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit(Some(encoder.finish()));

        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |res| {
            let _ = tx.send(res);
        });
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().ok()?.ok()?;

        let mapped = slice.get_mapped_range().ok()?;
        let mut out = vec![0u8; (width * height * 4) as usize];
        for row in 0..height {
            let src = &mapped[(row * bytes_per_row) as usize..][..(width * 4) as usize];
            let dst = &mut out[(row * width * 4) as usize..][..(width * 4) as usize];
            dst.copy_from_slice(src);
        }
        drop(mapped);
        staging.unmap();
        Some(out)
    }
}

fn align_to(val: u32, align: u32) -> u32 {
    val.div_ceil(align) * align
}

#[allow(clippy::too_many_arguments)]
fn create_rgba_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    width: u32,
    height: u32,
    rgba: &[u8],
    label: &str,
) -> (wgpu::Texture, wgpu::BindGroup) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
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
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&view),
            },
        ],
    });
    (tex, bg)
}

enum DrawBatchKind {
    White,
    Atlas,
    Image(ImageId),
}

struct DrawBatch {
    kind: DrawBatchKind,
    scissor: [u32; 4],
    start: u32,
    count: u32,
}

#[allow(clippy::too_many_arguments)]
fn render_with_gpu(
    gpu: &mut GpuContext,
    target_view: &wgpu::TextureView,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    list: &DisplayList,
    atlas: &mut GlyphAtlas,
    resources: &ResourceLoader,
    clear_color: Color,
) {
    let scale = list.scale_factor.max(1.0);
    let uniforms = UniformData {
        viewport: [width as f32, height as f32],
        _pad: [0.0, 0.0],
    };
    gpu.queue
        .write_buffer(&gpu.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

    // Pre-rasterize any glyphs in the display list before uploading atlas.
    for cmd in &list.commands {
        if let DisplayCommand::DrawGlyphRun { glyphs, .. } = cmd {
            for g in glyphs {
                let blob = list.fonts.get(&g.key.font_id);
                atlas.get_or_rasterize(g.key, blob);
            }
        }
    }

    let (aw, ah) = atlas.dimensions();
    if gpu.atlas_size != (aw, ah) {
        let (tex, bg) = create_rgba_texture(
            &gpu.device,
            &gpu.queue,
            &gpu.tex_bind_group_layout,
            &gpu.sampler,
            aw,
            ah,
            atlas.pixels(),
            "rustui-atlas-tex",
        );
        gpu.atlas_texture = tex;
        gpu.atlas_bind_group = bg;
        gpu.atlas_size = (aw, ah);
        atlas.dirty = false;
    } else if atlas.dirty {
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &gpu.atlas_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            atlas.pixels(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(aw * 4),
                rows_per_image: Some(ah),
            },
            wgpu::Extent3d {
                width: aw,
                height: ah,
                depth_or_array_layers: 1,
            },
        );
        atlas.dirty = false;
    }

    // Build batched instances + optional Vello path scene.
    let mut instances: Vec<SdfInstance> = Vec::new();
    let mut batches: Vec<DrawBatch> = Vec::new();
    let mut clip_stack: Vec<Rect> = vec![Rect::new(0.0, 0.0, width as f32, height as f32)];
    let mut opacity_stack: Vec<f32> = vec![1.0];
    let mut vello_scene = vello::Scene::new();
    let mut has_vello_paths = false;

    let push_inst = |instances: &mut Vec<SdfInstance>,
                     batches: &mut Vec<DrawBatch>,
                     kind: DrawBatchKind,
                     scissor: [u32; 4],
                     inst: SdfInstance| {
        let idx = instances.len() as u32;
        instances.push(inst);
        if let Some(last) = batches.last_mut() {
            let same_kind = match (&last.kind, &kind) {
                (DrawBatchKind::White, DrawBatchKind::White) => true,
                (DrawBatchKind::Atlas, DrawBatchKind::Atlas) => true,
                (DrawBatchKind::Image(a), DrawBatchKind::Image(b)) => a == b,
                _ => false,
            };
            if same_kind && last.scissor == scissor {
                last.count += 1;
                return;
            }
        }
        batches.push(DrawBatch {
            kind,
            scissor,
            start: idx,
            count: 1,
        });
    };

    for cmd in &list.commands {
        let current_opacity = *opacity_stack.last().unwrap_or(&1.0);
        let cur_clip =
            *clip_stack
                .last()
                .unwrap_or(&Rect::new(0.0, 0.0, width as f32, height as f32));
        let scissor = rect_to_scissor(cur_clip, width, height);

        match cmd {
            DisplayCommand::PushClipRect(r)
            | DisplayCommand::PushClipRoundedRect { rect: r, .. } => {
                let scaled = Rect::new(r.x * scale, r.y * scale, r.width * scale, r.height * scale);
                let next = cur_clip.intersection(&scaled).unwrap_or(Rect::ZERO);
                clip_stack.push(next);
            }
            DisplayCommand::PopClip => {
                if clip_stack.len() > 1 {
                    clip_stack.pop();
                }
            }
            DisplayCommand::PushLayer { opacity, .. } => {
                opacity_stack.push(current_opacity * opacity.clamp(0.0, 1.0));
            }
            DisplayCommand::PopLayer => {
                if opacity_stack.len() > 1 {
                    opacity_stack.pop();
                }
            }
            DisplayCommand::FillRoundedRect { rect, radii, color } => {
                push_inst(
                    &mut instances,
                    &mut batches,
                    DrawBatchKind::White,
                    scissor,
                    SdfInstance {
                        rect: [
                            rect.x * scale,
                            rect.y * scale,
                            rect.width * scale,
                            rect.height * scale,
                        ],
                        radii: [
                            radii.top_left * scale,
                            radii.top_right * scale,
                            radii.bottom_right * scale,
                            radii.bottom_left * scale,
                        ],
                        color: [color.r, color.g, color.b, color.a],
                        color2: [0.0; 4],
                        uv_rect: [0.0, 0.0, 1.0, 1.0],
                        params: [0.0, 0.0, 0.0, 0.0],
                        params2: [0.0, 0.0, current_opacity, 0.0],
                    },
                );
            }
            DisplayCommand::StrokeRoundedRect {
                rect,
                radii,
                width: sw,
                color,
            } => {
                push_inst(
                    &mut instances,
                    &mut batches,
                    DrawBatchKind::White,
                    scissor,
                    SdfInstance {
                        rect: [
                            rect.x * scale,
                            rect.y * scale,
                            rect.width * scale,
                            rect.height * scale,
                        ],
                        radii: [
                            radii.top_left * scale,
                            radii.top_right * scale,
                            radii.bottom_right * scale,
                            radii.bottom_left * scale,
                        ],
                        color: [color.r, color.g, color.b, color.a],
                        color2: [0.0; 4],
                        uv_rect: [0.0, 0.0, 1.0, 1.0],
                        params: [1.0, sw * scale, 0.0, 0.0],
                        params2: [0.0, 0.0, current_opacity, 0.0],
                    },
                );
            }
            DisplayCommand::DrawBoxShadow {
                rect,
                corner_radius,
                offset,
                blur_radius,
                color,
            } => {
                let r = *corner_radius * scale;
                push_inst(
                    &mut instances,
                    &mut batches,
                    DrawBatchKind::White,
                    scissor,
                    SdfInstance {
                        rect: [
                            (rect.x + offset.x) * scale,
                            (rect.y + offset.y) * scale,
                            rect.width * scale,
                            rect.height * scale,
                        ],
                        radii: [r, r, r, r],
                        color: [color.r, color.g, color.b, color.a],
                        color2: [0.0; 4],
                        uv_rect: [0.0, 0.0, 1.0, 1.0],
                        params: [2.0, (*blur_radius).max(1.0) * scale, 0.0, 0.0],
                        params2: [0.0, 0.0, current_opacity, 0.0],
                    },
                );
            }
            DisplayCommand::DrawBlurRect {
                rect,
                corner_radius,
                tint,
                ..
            } => {
                let r = *corner_radius * scale;
                push_inst(
                    &mut instances,
                    &mut batches,
                    DrawBatchKind::White,
                    scissor,
                    SdfInstance {
                        rect: [
                            rect.x * scale,
                            rect.y * scale,
                            rect.width * scale,
                            rect.height * scale,
                        ],
                        radii: [r, r, r, r],
                        color: [tint.r, tint.g, tint.b, tint.a],
                        color2: [0.0; 4],
                        uv_rect: [0.0, 0.0, 1.0, 1.0],
                        params: [0.0, 0.0, 0.0, 0.0],
                        params2: [0.0, 0.0, current_opacity, 0.0],
                    },
                );
            }
            DisplayCommand::DrawGradient {
                rect,
                radii,
                gradient,
            } => {
                let (mode, c0, c1, p0, p1) = match gradient {
                    GradientKind::Linear { start, end, stops } => {
                        let a = stops.first().map_or(Color::TRANSPARENT, |s| s.color);
                        let b = stops.last().map_or(a, |s| s.color);
                        (
                            3.0,
                            [a.r, a.g, a.b, a.a],
                            [b.r, b.g, b.b, b.a],
                            [start.x * scale, start.y * scale],
                            [end.x * scale, end.y * scale],
                        )
                    }
                    GradientKind::Radial {
                        center,
                        radius,
                        stops,
                    } => {
                        let a = stops.first().map_or(Color::TRANSPARENT, |s| s.color);
                        let b = stops.last().map_or(a, |s| s.color);
                        (
                            4.0,
                            [a.r, a.g, a.b, a.a],
                            [b.r, b.g, b.b, b.a],
                            [center.x * scale, center.y * scale],
                            [*radius * scale, 0.0],
                        )
                    }
                };
                push_inst(
                    &mut instances,
                    &mut batches,
                    DrawBatchKind::White,
                    scissor,
                    SdfInstance {
                        rect: [
                            rect.x * scale,
                            rect.y * scale,
                            rect.width * scale,
                            rect.height * scale,
                        ],
                        radii: [
                            radii.top_left * scale,
                            radii.top_right * scale,
                            radii.bottom_right * scale,
                            radii.bottom_left * scale,
                        ],
                        color: c0,
                        color2: c1,
                        uv_rect: [0.0, 0.0, 1.0, 1.0],
                        params: [mode, 0.0, p0[0], p0[1]],
                        params2: [p1[0], p1[1], current_opacity, 0.0],
                    },
                );
            }
            DisplayCommand::DrawGlyphRun { glyphs, color } => {
                for g in glyphs {
                    let Some(entry) = atlas.get(&g.key).copied() else {
                        continue;
                    };
                    if entry.width == 0 || entry.height == 0 {
                        continue;
                    }
                    let x = g.position.x * scale + entry.bearing_x as f32;
                    let y = g.position.y * scale - entry.bearing_y as f32;
                    let gc = if g.color.a > 0.0 { g.color } else { *color };
                    push_inst(
                        &mut instances,
                        &mut batches,
                        DrawBatchKind::Atlas,
                        scissor,
                        SdfInstance {
                            rect: [x, y, entry.width as f32, entry.height as f32],
                            radii: [0.0; 4],
                            color: [gc.r, gc.g, gc.b, gc.a],
                            color2: [0.0; 4],
                            uv_rect: [entry.uv.x, entry.uv.y, entry.uv.width, entry.uv.height],
                            params: [5.0, if entry.is_color { 1.0 } else { 0.0 }, 0.0, 0.0],
                            params2: [0.0, 0.0, current_opacity, 0.0],
                        },
                    );
                }
            }
            DisplayCommand::DrawImage {
                rect,
                image_id,
                uv,
                tint,
            } => {
                if let Some(img) = resources.get(*image_id) {
                    if !gpu.image_textures.contains_key(image_id) {
                        let pair = create_rgba_texture(
                            &gpu.device,
                            &gpu.queue,
                            &gpu.tex_bind_group_layout,
                            &gpu.sampler,
                            img.width,
                            img.height,
                            &img.rgba,
                            "rustui-image-tex",
                        );
                        gpu.image_textures.insert(*image_id, pair);
                    }
                    push_inst(
                        &mut instances,
                        &mut batches,
                        DrawBatchKind::Image(*image_id),
                        scissor,
                        SdfInstance {
                            rect: [
                                rect.x * scale,
                                rect.y * scale,
                                rect.width * scale,
                                rect.height * scale,
                            ],
                            radii: [0.0; 4],
                            color: [tint.r, tint.g, tint.b, tint.a],
                            color2: [0.0; 4],
                            uv_rect: [uv.x, uv.y, uv.width, uv.height],
                            params: [6.0, 0.0, 0.0, 0.0],
                            params2: [0.0, 0.0, current_opacity, 0.0],
                        },
                    );
                }
            }
            DisplayCommand::DrawNineSlice {
                rect,
                image_id,
                slice,
                tint,
            } => {
                if let Some(img) = resources.get(*image_id) {
                    if !gpu.image_textures.contains_key(image_id) {
                        let pair = create_rgba_texture(
                            &gpu.device,
                            &gpu.queue,
                            &gpu.tex_bind_group_layout,
                            &gpu.sampler,
                            img.width,
                            img.height,
                            &img.rgba,
                            "rustui-image-tex",
                        );
                        gpu.image_textures.insert(*image_id, pair);
                    }
                    for (patch_rect, patch_uv) in compute_nine_slice_patches(
                        *rect,
                        img.width as f32,
                        img.height as f32,
                        *slice,
                    ) {
                        push_inst(
                            &mut instances,
                            &mut batches,
                            DrawBatchKind::Image(*image_id),
                            scissor,
                            SdfInstance {
                                rect: [
                                    patch_rect.x * scale,
                                    patch_rect.y * scale,
                                    patch_rect.width * scale,
                                    patch_rect.height * scale,
                                ],
                                radii: [0.0; 4],
                                color: [tint.r, tint.g, tint.b, tint.a],
                                color2: [0.0; 4],
                                uv_rect: [patch_uv.x, patch_uv.y, patch_uv.width, patch_uv.height],
                                params: [6.0, 0.0, 0.0, 0.0],
                                params2: [0.0, 0.0, current_opacity, 0.0],
                            },
                        );
                    }
                }
            }
            DisplayCommand::DrawPath { ops, fill, stroke } => {
                has_vello_paths = true;
                let path = path_ops_to_bezpath(ops, scale);
                if let Some(fc) = fill {
                    let c = VColor::new([fc.r, fc.g, fc.b, fc.a * current_opacity]);
                    vello_scene.fill(
                        Fill::NonZero,
                        Affine::IDENTITY,
                        &Brush::Solid(c),
                        None,
                        &path,
                    );
                }
                if let Some((sw, sc)) = stroke {
                    let c = VColor::new([sc.r, sc.g, sc.b, sc.a * current_opacity]);
                    vello_scene.stroke(
                        &Stroke::new(f64::from(*sw * scale)),
                        Affine::IDENTITY,
                        &Brush::Solid(c),
                        None,
                        &path,
                    );
                }
            }
            DisplayCommand::DrawCustomShader { rect, .. } => {
                // Custom shader rectangle placeholder draw if no external WGSL pipeline supplied.
                push_inst(
                    &mut instances,
                    &mut batches,
                    DrawBatchKind::White,
                    scissor,
                    SdfInstance {
                        rect: [
                            rect.x * scale,
                            rect.y * scale,
                            rect.width * scale,
                            rect.height * scale,
                        ],
                        radii: [0.0; 4],
                        color: [0.12, 0.14, 0.18, 1.0],
                        color2: [0.0; 4],
                        uv_rect: [0.0, 0.0, 1.0, 1.0],
                        params: [0.0, 0.0, 0.0, 0.0],
                        params2: [0.0, 0.0, current_opacity, 0.0],
                    },
                );
            }
        }
    }

    // If Vello paths exist and Vello GPU renderer is ready, rasterize them into a cached texture and blit first.
    if has_vello_paths && let Some(vello) = &mut gpu.vello_renderer {
        let vello_tex = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("rustui-vello-path-tex"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let vello_view = vello_tex.create_view(&wgpu::TextureViewDescriptor::default());
        if vello
            .render_to_texture(
                &gpu.device,
                &gpu.queue,
                &vello_scene,
                &vello_view,
                &vello::RenderParams {
                    base_color: VColor::TRANSPARENT,
                    width,
                    height,
                    antialiasing_method: vello::AaConfig::Area,
                },
            )
            .is_ok()
        {
            let vello_bg = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("rustui-vello-bg"),
                layout: &gpu.tex_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Sampler(&gpu.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&vello_view),
                    },
                ],
            });
            let composite_id = u64::MAX - 1;
            gpu.image_textures
                .insert(composite_id, (vello_tex, vello_bg));
            let idx = instances.len() as u32;
            instances.push(SdfInstance {
                rect: [0.0, 0.0, width as f32, height as f32],
                radii: [0.0; 4],
                color: [1.0, 1.0, 1.0, 1.0],
                color2: [0.0; 4],
                uv_rect: [0.0, 0.0, 1.0, 1.0],
                params: [6.0, 0.0, 0.0, 0.0],
                params2: [0.0, 0.0, 1.0, 0.0],
            });
            batches.push(DrawBatch {
                kind: DrawBatchKind::Image(composite_id),
                scissor: [0, 0, width, height],
                start: idx,
                count: 1,
            });
        }
    }

    let instance_buf = if !instances.is_empty() {
        let byte_size = (instances.len() * std::mem::size_of::<SdfInstance>()) as u64;
        let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rustui-instances"),
            size: byte_size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        gpu.queue
            .write_buffer(&buf, 0, bytemuck::cast_slice(&instances));
        Some(buf)
    } else {
        None
    };

    let pipeline = if format == wgpu::TextureFormat::Bgra8UnormSrgb {
        gpu.pipeline_bgra.as_ref().unwrap_or(&gpu.pipeline_rgba)
    } else {
        &gpu.pipeline_rgba
    };

    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("rustui-render-encoder"),
        });

    {
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("rustui-main-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::from(clear_color.r),
                        g: f64::from(clear_color.g),
                        b: f64::from(clear_color.b),
                        a: f64::from(clear_color.a),
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        if let Some(buf) = &instance_buf {
            rpass.set_pipeline(pipeline);
            rpass.set_bind_group(0, &gpu.uniform_bind_group, &[]);
            rpass.set_vertex_buffer(0, buf.slice(..));

            for batch in &batches {
                if batch.scissor[2] == 0 || batch.scissor[3] == 0 {
                    continue;
                }
                rpass.set_scissor_rect(
                    batch.scissor[0],
                    batch.scissor[1],
                    batch.scissor[2],
                    batch.scissor[3],
                );
                let bg = match batch.kind {
                    DrawBatchKind::White => &gpu.white_tex_bind_group,
                    DrawBatchKind::Atlas => &gpu.atlas_bind_group,
                    DrawBatchKind::Image(id) => gpu
                        .image_textures
                        .get(&id)
                        .map(|(_, b)| b)
                        .unwrap_or(&gpu.white_tex_bind_group),
                };
                rpass.set_bind_group(1, bg, &[]);
                rpass.draw(0..6, batch.start..(batch.start + batch.count));
            }
        }
    }

    gpu.queue.submit(Some(encoder.finish()));
}

fn rect_to_scissor(r: Rect, max_w: u32, max_h: u32) -> [u32; 4] {
    let x0 = (r.x.max(0.0).floor() as u32).min(max_w);
    let y0 = (r.y.max(0.0).floor() as u32).min(max_h);
    let x1 = (r.max_x().max(0.0).ceil() as u32).min(max_w);
    let y1 = (r.max_y().max(0.0).ceil() as u32).min(max_h);
    [x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0)]
}

fn path_ops_to_bezpath(ops: &[PathOp], scale: f32) -> BezPath {
    let mut path = BezPath::new();
    let s = f64::from(scale);
    for op in ops {
        match *op {
            PathOp::MoveTo(p) => path.move_to(KPoint::new(f64::from(p.x) * s, f64::from(p.y) * s)),
            PathOp::LineTo(p) => path.line_to(KPoint::new(f64::from(p.x) * s, f64::from(p.y) * s)),
            PathOp::QuadTo { ctrl, to } => path.quad_to(
                KPoint::new(f64::from(ctrl.x) * s, f64::from(ctrl.y) * s),
                KPoint::new(f64::from(to.x) * s, f64::from(to.y) * s),
            ),
            PathOp::CubicTo { ctrl1, ctrl2, to } => path.curve_to(
                KPoint::new(f64::from(ctrl1.x) * s, f64::from(ctrl1.y) * s),
                KPoint::new(f64::from(ctrl2.x) * s, f64::from(ctrl2.y) * s),
                KPoint::new(f64::from(to.x) * s, f64::from(to.y) * s),
            ),
            PathOp::Close => path.close_path(),
        }
    }
    path
}

fn compute_nine_slice_patches(
    dst: Rect,
    img_w: f32,
    img_h: f32,
    slice: rustui_core::Edges,
) -> [(Rect, Rect); 9] {
    let iw = img_w.max(1.0);
    let ih = img_h.max(1.0);
    let l = slice.left.min(dst.width * 0.5).min(iw * 0.5);
    let r = slice.right.min(dst.width * 0.5).min(iw * 0.5);
    let t = slice.top.min(dst.height * 0.5).min(ih * 0.5);
    let b = slice.bottom.min(dst.height * 0.5).min(ih * 0.5);

    let xs = [dst.x, dst.x + l, dst.max_x() - r, dst.max_x()];
    let ys = [dst.y, dst.y + t, dst.max_y() - b, dst.max_y()];
    let us = [0.0, l / iw, 1.0 - r / iw, 1.0];
    let vs = [0.0, t / ih, 1.0 - b / ih, 1.0];

    let mut out = [(Rect::ZERO, Rect::ZERO); 9];
    let mut k = 0;
    for row in 0..3 {
        for col in 0..3 {
            out[k] = (
                Rect::new(
                    xs[col],
                    ys[row],
                    xs[col + 1] - xs[col],
                    ys[row + 1] - ys[row],
                ),
                Rect::new(
                    us[col],
                    vs[row],
                    us[col + 1] - us[col],
                    vs[row + 1] - vs[row],
                ),
            );
            k += 1;
        }
    }
    out
}

/// Deterministic CPU software rasterizer used as automatic fallback when a GPU adapter is absent.
#[derive(Debug, Default)]
pub struct SoftwareRasterizer;

impl SoftwareRasterizer {
    pub fn render(
        list: &DisplayList,
        atlas: &mut GlyphAtlas,
        resources: &ResourceLoader,
        width: u32,
        height: u32,
        clear_color: Color,
    ) -> Vec<u8> {
        let w = width.max(1);
        let h = height.max(1);
        let scale = list.scale_factor.max(1.0);
        let clear = clear_color.to_rgba8();
        let mut buf = vec![0u8; (w * h * 4) as usize];
        for px in buf.as_chunks_mut::<4>().0 {
            px.copy_from_slice(&clear);
        }

        let mut clip_stack: Vec<Rect> = vec![Rect::new(0.0, 0.0, w as f32, h as f32)];
        let mut opacity_stack: Vec<f32> = vec![1.0];

        for cmd in &list.commands {
            let opacity = *opacity_stack.last().unwrap_or(&1.0);
            let clip = *clip_stack
                .last()
                .unwrap_or(&Rect::new(0.0, 0.0, w as f32, h as f32));

            match cmd {
                DisplayCommand::PushClipRect(r)
                | DisplayCommand::PushClipRoundedRect { rect: r, .. } => {
                    let sr = scale_rect(*r, scale);
                    clip_stack.push(clip.intersection(&sr).unwrap_or(Rect::ZERO));
                }
                DisplayCommand::PopClip => {
                    if clip_stack.len() > 1 {
                        clip_stack.pop();
                    }
                }
                DisplayCommand::PushLayer {
                    opacity: layer_op, ..
                } => {
                    opacity_stack.push(opacity * layer_op.clamp(0.0, 1.0));
                }
                DisplayCommand::PopLayer => {
                    if opacity_stack.len() > 1 {
                        opacity_stack.pop();
                    }
                }
                DisplayCommand::FillRoundedRect { rect, radii, color } => {
                    let sr = scale_rect(*rect, scale);
                    let sradii = scale_radii(*radii, scale);
                    raster_sdf_rect(&mut buf, w, h, clip, sr, sradii, |d| {
                        let cov = smoothstep(0.65, -0.65, d);
                        Color::rgba(color.r, color.g, color.b, color.a * cov * opacity)
                    });
                }
                DisplayCommand::StrokeRoundedRect {
                    rect,
                    radii,
                    width: sw,
                    color,
                } => {
                    let sr = scale_rect(*rect, scale);
                    let sradii = scale_radii(*radii, scale);
                    let sw_px = (*sw * scale).max(0.5);
                    raster_sdf_rect(&mut buf, w, h, clip, sr, sradii, |d| {
                        let ring = (d + sw_px * 0.5).abs() - sw_px * 0.5;
                        let cov = smoothstep(0.65, -0.65, ring);
                        Color::rgba(color.r, color.g, color.b, color.a * cov * opacity)
                    });
                }
                DisplayCommand::DrawBoxShadow {
                    rect,
                    corner_radius,
                    offset,
                    blur_radius,
                    color,
                } => {
                    let blur = (*blur_radius * scale).max(1.0);
                    let sr = Rect::new(
                        (rect.x + offset.x) * scale,
                        (rect.y + offset.y) * scale,
                        rect.width * scale,
                        rect.height * scale,
                    );
                    let expanded = sr.expand(blur * 1.5);
                    let sradii = CornerRadii::all(*corner_radius * scale);
                    raster_sdf_rect_in_bounds(&mut buf, w, h, clip, expanded, sr, sradii, |d| {
                        let cov = 1.0 - smoothstep(-blur * 0.5, blur * 1.4, d);
                        Color::rgba(color.r, color.g, color.b, color.a * cov * opacity)
                    });
                }
                DisplayCommand::DrawBlurRect {
                    rect,
                    corner_radius,
                    tint,
                    ..
                } => {
                    let sr = scale_rect(*rect, scale);
                    let sradii = CornerRadii::all(*corner_radius * scale);
                    raster_sdf_rect(&mut buf, w, h, clip, sr, sradii, |d| {
                        let cov = smoothstep(0.65, -0.65, d);
                        Color::rgba(tint.r, tint.g, tint.b, tint.a * cov * opacity)
                    });
                }
                DisplayCommand::DrawGradient {
                    rect,
                    radii,
                    gradient,
                } => {
                    let sr = scale_rect(*rect, scale);
                    let sradii = scale_radii(*radii, scale);
                    raster_sdf_rect_xy(&mut buf, w, h, clip, sr, sradii, |px, py, d| {
                        let cov = smoothstep(0.65, -0.65, d);
                        let lx = px - sr.x;
                        let ly = py - sr.y;
                        let c = eval_gradient(gradient, lx / scale, ly / scale);
                        Color::rgba(c.r, c.g, c.b, c.a * cov * opacity)
                    });
                }
                DisplayCommand::DrawImage {
                    rect,
                    image_id,
                    uv,
                    tint,
                } => {
                    if let Some(img) = resources.get(*image_id) {
                        let sr = scale_rect(*rect, scale);
                        blit_image(
                            &mut buf, w, h, clip, sr, *uv, img.width, img.height, &img.rgba, *tint,
                            opacity,
                        );
                    }
                }
                DisplayCommand::DrawNineSlice {
                    rect,
                    image_id,
                    slice,
                    tint,
                } => {
                    if let Some(img) = resources.get(*image_id) {
                        for (pr, puv) in compute_nine_slice_patches(
                            *rect,
                            img.width as f32,
                            img.height as f32,
                            *slice,
                        ) {
                            let sr = scale_rect(pr, scale);
                            blit_image(
                                &mut buf, w, h, clip, sr, puv, img.width, img.height, &img.rgba,
                                *tint, opacity,
                            );
                        }
                    }
                }
                DisplayCommand::DrawGlyphRun { glyphs, color } => {
                    for g in glyphs {
                        let blob = list.fonts.get(&g.key.font_id);
                        let Some(entry) = atlas.get_or_rasterize(g.key, blob) else {
                            continue;
                        };
                        if entry.width == 0 || entry.height == 0 {
                            continue;
                        }
                        let dst_x = (g.position.x * scale + entry.bearing_x as f32).round() as i32;
                        let dst_y = (g.position.y * scale - entry.bearing_y as f32).round() as i32;
                        let gc = if g.color.a > 0.0 { g.color } else { *color };
                        let (aw, _ah) = atlas.dimensions();
                        let atlas_px = atlas.pixels();
                        for row in 0..entry.height {
                            let py = dst_y + row as i32;
                            if py < 0 || py >= h as i32 {
                                continue;
                            }
                            if (py as f32) < clip.y || (py as f32) >= clip.max_y() {
                                continue;
                            }
                            for col in 0..entry.width {
                                let px = dst_x + col as i32;
                                if px < 0 || px >= w as i32 {
                                    continue;
                                }
                                if (px as f32) < clip.x || (px as f32) >= clip.max_x() {
                                    continue;
                                }
                                let src_i = (((entry.y + row) * aw + (entry.x + col)) * 4) as usize;
                                let src = if entry.is_color {
                                    Color::from_rgba8(
                                        atlas_px[src_i],
                                        atlas_px[src_i + 1],
                                        atlas_px[src_i + 2],
                                        atlas_px[src_i + 3],
                                    )
                                } else {
                                    let a = f32::from(atlas_px[src_i + 3]) / 255.0;
                                    Color::rgba(gc.r, gc.g, gc.b, gc.a * a)
                                };
                                blend_pixel(
                                    &mut buf,
                                    w,
                                    px as u32,
                                    py as u32,
                                    Color::rgba(src.r, src.g, src.b, src.a * opacity),
                                );
                            }
                        }
                    }
                }
                DisplayCommand::DrawPath { ops, fill, stroke } => {
                    let pts = flatten_path(ops, scale);
                    if let Some(fc) = fill {
                        fill_polygon(
                            &mut buf,
                            w,
                            h,
                            clip,
                            &pts,
                            Color::rgba(fc.r, fc.g, fc.b, fc.a * opacity),
                        );
                    }
                    if let Some((sw, sc)) = stroke {
                        stroke_polyline(
                            &mut buf,
                            w,
                            h,
                            clip,
                            &pts,
                            (*sw * scale).max(1.0),
                            Color::rgba(sc.r, sc.g, sc.b, sc.a * opacity),
                        );
                    }
                }
                DisplayCommand::DrawCustomShader { rect, .. } => {
                    let sr = scale_rect(*rect, scale);
                    raster_sdf_rect(&mut buf, w, h, clip, sr, CornerRadii::ZERO, |_| {
                        Color::rgba(0.12, 0.14, 0.18, opacity)
                    });
                }
            }
        }
        buf
    }
}

fn scale_rect(r: Rect, scale: f32) -> Rect {
    Rect::new(r.x * scale, r.y * scale, r.width * scale, r.height * scale)
}

fn scale_radii(r: CornerRadii, scale: f32) -> CornerRadii {
    CornerRadii {
        top_left: r.top_left * scale,
        top_right: r.top_right * scale,
        bottom_right: r.bottom_right * scale,
        bottom_left: r.bottom_left * scale,
    }
}

fn sd_rounded_box(px: f32, py: f32, hx: f32, hy: f32, radii: CornerRadii) -> f32 {
    let r = if px >= 0.0 {
        if py < 0.0 {
            radii.top_right
        } else {
            radii.bottom_right
        }
    } else if py < 0.0 {
        radii.top_left
    } else {
        radii.bottom_left
    };
    let qx = px.abs() - hx + r;
    let qy = py.abs() - hy + r;
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    qx.max(qy).min(0.0) + outside - r
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn raster_sdf_rect(
    buf: &mut [u8],
    w: u32,
    h: u32,
    clip: Rect,
    rect: Rect,
    radii: CornerRadii,
    eval: impl Fn(f32) -> Color,
) {
    raster_sdf_rect_in_bounds(buf, w, h, clip, rect, rect, radii, eval);
}

#[allow(clippy::too_many_arguments)]
fn raster_sdf_rect_in_bounds(
    buf: &mut [u8],
    w: u32,
    h: u32,
    clip: Rect,
    iter_rect: Rect,
    sdf_rect: Rect,
    radii: CornerRadii,
    eval: impl Fn(f32) -> Color,
) {
    raster_sdf_rect_xy_bounds(buf, w, h, clip, iter_rect, sdf_rect, radii, |_, _, d| {
        eval(d)
    });
}

fn raster_sdf_rect_xy(
    buf: &mut [u8],
    w: u32,
    h: u32,
    clip: Rect,
    rect: Rect,
    radii: CornerRadii,
    eval: impl Fn(f32, f32, f32) -> Color,
) {
    raster_sdf_rect_xy_bounds(buf, w, h, clip, rect, rect, radii, eval);
}

#[allow(clippy::too_many_arguments)]
fn raster_sdf_rect_xy_bounds(
    buf: &mut [u8],
    w: u32,
    h: u32,
    clip: Rect,
    iter_rect: Rect,
    sdf_rect: Rect,
    radii: CornerRadii,
    eval: impl Fn(f32, f32, f32) -> Color,
) {
    let Some(area) = iter_rect.intersection(&clip) else {
        return;
    };
    let x0 = (area.x.floor().max(0.0) as u32).min(w);
    let y0 = (area.y.floor().max(0.0) as u32).min(h);
    let x1 = (area.max_x().ceil().max(0.0) as u32).min(w);
    let y1 = (area.max_y().ceil().max(0.0) as u32).min(h);
    let hx = sdf_rect.width * 0.5;
    let hy = sdf_rect.height * 0.5;
    let cx = sdf_rect.x + hx;
    let cy = sdf_rect.y + hy;

    for py in y0..y1 {
        let fy = py as f32 + 0.5;
        for px in x0..x1 {
            let fx = px as f32 + 0.5;
            let d = sd_rounded_box(fx - cx, fy - cy, hx, hy, radii);
            let c = eval(fx, fy, d);
            if c.a > 0.001 {
                blend_pixel(buf, w, px, py, c);
            }
        }
    }
}

fn eval_gradient(gradient: &GradientKind, x: f32, y: f32) -> Color {
    let sample_stops = |stops: &[GradientStop], t: f32| -> Color {
        if stops.is_empty() {
            return Color::TRANSPARENT;
        }
        if stops.len() == 1 || t <= stops[0].offset {
            return stops[0].color;
        }
        for win in stops.windows(2) {
            if t <= win[1].offset {
                let span = (win[1].offset - win[0].offset).max(0.0001);
                return win[0].color.lerp(win[1].color, (t - win[0].offset) / span);
            }
        }
        stops.last().map_or(Color::TRANSPARENT, |s| s.color)
    };

    match gradient {
        GradientKind::Linear { start, end, stops } => {
            let dx = end.x - start.x;
            let dy = end.y - start.y;
            let denom = (dx * dx + dy * dy).max(0.0001);
            let t = (((x - start.x) * dx + (y - start.y) * dy) / denom).clamp(0.0, 1.0);
            sample_stops(stops, t)
        }
        GradientKind::Radial {
            center,
            radius,
            stops,
        } => {
            let d = ((x - center.x).powi(2) + (y - center.y).powi(2)).sqrt();
            let t = (d / radius.max(1.0)).clamp(0.0, 1.0);
            sample_stops(stops, t)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn blit_image(
    buf: &mut [u8],
    w: u32,
    h: u32,
    clip: Rect,
    dst: Rect,
    uv: Rect,
    img_w: u32,
    img_h: u32,
    rgba: &[u8],
    tint: Color,
    opacity: f32,
) {
    let Some(area) = dst.intersection(&clip) else {
        return;
    };
    if dst.width <= 0.0 || dst.height <= 0.0 || img_w == 0 || img_h == 0 {
        return;
    }
    let x0 = (area.x.floor().max(0.0) as u32).min(w);
    let y0 = (area.y.floor().max(0.0) as u32).min(h);
    let x1 = (area.max_x().ceil().max(0.0) as u32).min(w);
    let y1 = (area.max_y().ceil().max(0.0) as u32).min(h);

    for py in y0..y1 {
        let v_norm = ((py as f32 + 0.5) - dst.y) / dst.height;
        let v = (uv.y + v_norm * uv.height).clamp(0.0, 0.9999);
        let sy = (v * img_h as f32) as u32;
        for px in x0..x1 {
            let u_norm = ((px as f32 + 0.5) - dst.x) / dst.width;
            let u = (uv.x + u_norm * uv.width).clamp(0.0, 0.9999);
            let sx = (u * img_w as f32) as u32;
            let idx = ((sy * img_w + sx) * 4) as usize;
            let src = Color::rgba(
                (f32::from(rgba[idx]) / 255.0) * tint.r,
                (f32::from(rgba[idx + 1]) / 255.0) * tint.g,
                (f32::from(rgba[idx + 2]) / 255.0) * tint.b,
                (f32::from(rgba[idx + 3]) / 255.0) * tint.a * opacity,
            );
            blend_pixel(buf, w, px, py, src);
        }
    }
}

fn flatten_path(ops: &[PathOp], scale: f32) -> Vec<Point> {
    let mut pts = Vec::new();
    let mut cur = Point::ZERO;
    let mut start = Point::ZERO;
    for op in ops {
        match *op {
            PathOp::MoveTo(p) => {
                cur = Point::new(p.x * scale, p.y * scale);
                start = cur;
                pts.push(cur);
            }
            PathOp::LineTo(p) => {
                cur = Point::new(p.x * scale, p.y * scale);
                pts.push(cur);
            }
            PathOp::QuadTo { ctrl, to } => {
                let c = Point::new(ctrl.x * scale, ctrl.y * scale);
                let t = Point::new(to.x * scale, to.y * scale);
                for step in 1..=12 {
                    let s = step as f32 / 12.0;
                    let inv = 1.0 - s;
                    pts.push(Point::new(
                        inv * inv * cur.x + 2.0 * inv * s * c.x + s * s * t.x,
                        inv * inv * cur.y + 2.0 * inv * s * c.y + s * s * t.y,
                    ));
                }
                cur = t;
            }
            PathOp::CubicTo { ctrl1, ctrl2, to } => {
                let c1 = Point::new(ctrl1.x * scale, ctrl1.y * scale);
                let c2 = Point::new(ctrl2.x * scale, ctrl2.y * scale);
                let t = Point::new(to.x * scale, to.y * scale);
                for step in 1..=16 {
                    let s = step as f32 / 16.0;
                    let inv = 1.0 - s;
                    pts.push(Point::new(
                        inv.powi(3) * cur.x
                            + 3.0 * inv.powi(2) * s * c1.x
                            + 3.0 * inv * s.powi(2) * c2.x
                            + s.powi(3) * t.x,
                        inv.powi(3) * cur.y
                            + 3.0 * inv.powi(2) * s * c1.y
                            + 3.0 * inv * s.powi(2) * c2.y
                            + s.powi(3) * t.y,
                    ));
                }
                cur = t;
            }
            PathOp::Close => {
                pts.push(start);
                cur = start;
            }
        }
    }
    pts
}

fn fill_polygon(buf: &mut [u8], w: u32, h: u32, clip: Rect, pts: &[Point], color: Color) {
    if pts.len() < 3 || color.a <= 0.0 {
        return;
    }
    let min_y = pts
        .iter()
        .map(|p| p.y)
        .fold(f32::INFINITY, f32::min)
        .max(clip.y)
        .max(0.0);
    let max_y = pts
        .iter()
        .map(|p| p.y)
        .fold(f32::NEG_INFINITY, f32::max)
        .min(clip.max_y())
        .min(h as f32);
    let min_x = clip.x.max(0.0) as u32;
    let max_x = clip.max_x().min(w as f32) as u32;

    for y in (min_y.floor() as u32)..(max_y.ceil() as u32).min(h) {
        let fy = y as f32 + 0.5;
        let mut xs = Vec::new();
        for i in 0..pts.len() {
            let a = pts[i];
            let b = pts[(i + 1) % pts.len()];
            if (a.y <= fy && b.y > fy) || (b.y <= fy && a.y > fy) {
                let t = (fy - a.y) / (b.y - a.y);
                xs.push(a.x + t * (b.x - a.x));
            }
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        for pair in xs.as_chunks::<2>().0 {
            let sx0 = (pair[0].floor() as i32).max(min_x as i32) as u32;
            let sx1 = (pair[1].ceil() as i32).min(max_x as i32).max(0) as u32;
            for x in sx0..sx1 {
                blend_pixel(buf, w, x, y, color);
            }
        }
    }
}

fn stroke_polyline(
    buf: &mut [u8],
    w: u32,
    h: u32,
    clip: Rect,
    pts: &[Point],
    width: f32,
    color: Color,
) {
    if pts.len() < 2 || color.a <= 0.0 {
        return;
    }
    let radius = (width * 0.5).max(0.5);
    for seg in pts.windows(2) {
        let a = seg[0];
        let b = seg[1];
        let x0 = (a.x.min(b.x) - radius - 1.0).max(clip.x).max(0.0) as u32;
        let y0 = (a.y.min(b.y) - radius - 1.0).max(clip.y).max(0.0) as u32;
        let x1 = ((a.x.max(b.x) + radius + 1.0)
            .min(clip.max_x())
            .min(w as f32)) as u32;
        let y1 = ((a.y.max(b.y) + radius + 1.0)
            .min(clip.max_y())
            .min(h as f32)) as u32;
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let len_sq = (dx * dx + dy * dy).max(0.0001);
        for py in y0..y1 {
            let fy = py as f32 + 0.5;
            for px in x0..x1 {
                let fx = px as f32 + 0.5;
                let t = (((fx - a.x) * dx + (fy - a.y) * dy) / len_sq).clamp(0.0, 1.0);
                let proj_x = a.x + t * dx;
                let proj_y = a.y + t * dy;
                let dist = ((fx - proj_x).powi(2) + (fy - proj_y).powi(2)).sqrt() - radius;
                let cov = smoothstep(0.65, -0.65, dist);
                if cov > 0.001 {
                    blend_pixel(
                        buf,
                        w,
                        px,
                        py,
                        Color::rgba(color.r, color.g, color.b, color.a * cov),
                    );
                }
            }
        }
    }
}

#[inline]
fn blend_pixel(buf: &mut [u8], w: u32, x: u32, y: u32, src: Color) {
    let idx = ((y * w + x) * 4) as usize;
    let sa = src.a.clamp(0.0, 1.0);
    if sa >= 0.999 {
        let [r, g, b, a] = src.to_rgba8();
        buf[idx] = r;
        buf[idx + 1] = g;
        buf[idx + 2] = b;
        buf[idx + 3] = a;
        return;
    }
    let inv = 1.0 - sa;
    let dr = f32::from(buf[idx]) / 255.0;
    let dg = f32::from(buf[idx + 1]) / 255.0;
    let db = f32::from(buf[idx + 2]) / 255.0;
    let da = f32::from(buf[idx + 3]) / 255.0;
    buf[idx] = ((src.r * sa + dr * inv).clamp(0.0, 1.0) * 255.0).round() as u8;
    buf[idx + 1] = ((src.g * sa + dg * inv).clamp(0.0, 1.0) * 255.0).round() as u8;
    buf[idx + 2] = ((src.b * sa + db * inv).clamp(0.0, 1.0) * 255.0).round() as u8;
    buf[idx + 3] = ((sa + da * inv).clamp(0.0, 1.0) * 255.0).round() as u8;
}

/// Encodes RGBA8 pixel data into a PNG byte buffer.
pub fn encode_rgba_to_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let img = image::RgbaImage::from_raw(width, height, rgba.to_vec())
        .ok_or_else(|| "invalid RGBA dimensions".to_owned())?;
    let mut out = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut out);
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut cursor, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

pub fn register_font_if_missing(list: &mut DisplayList, blob: FontBlob) {
    list.register_font(blob);
}
