use anyhow::Result;
use log::{error, info};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::thread;
use tunnelclient::config::ClientConfig;
use tunnelclient::draw::Draw;
use tunnelclient::render::Vertex;
use tunnels_lib::RunFlag;
use tunnels_lib::Snapshot;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Fullscreen, Window, WindowAttributes, WindowId};
use zero_configure::pub_sub::Receiver as ZmqReceiver;
use zmq::Context;

pub type SnapshotManagerHandle = Arc<Mutex<Option<SnapshotHandle>>>;
pub type SnapshotHandle = Arc<Snapshot>;

const SHADER_SOURCE: &str = "
struct Uniforms {
    resolution: vec2<f32>,
};
@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(@location(0) pos: vec2<f32>, @location(1) color: vec4<f32>) -> VertexOutput {
    var out: VertexOutput;
    let ndc = vec2<f32>(
        pos.x / uniforms.resolution.x * 2.0 - 1.0,
        -(pos.y / uniforms.resolution.y * 2.0 - 1.0),
    );
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.color = color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
";

/// wgpu-based renderer. Initializes the GPU pipeline: instance -> adapter -> device -> surface
/// -> render pipeline -> MSAA texture. Requests all backends (Metal, GL, Vulkan, DX12) for
/// cross-platform support. Uses 4x MSAA and AutoVsync present mode.
struct GpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    pipeline: wgpu::RenderPipeline,
    #[allow(dead_code)] // Kept alive to back the uniform bind group
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    msaa_texture: wgpu::TextureView,
    surface_config: wgpu::SurfaceConfiguration,
    // Persistent GPU buffers, reused across frames (grow-only).
    vertex_buffer: wgpu::Buffer,
    vertex_buffer_size: u64,
    index_buffer: wgpu::Buffer,
    index_buffer_size: u64,
}

impl GpuRenderer {
    fn new(window: Arc<Window>, width: u32, height: u32) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::GL | wgpu::Backends::METAL,
            ..Default::default()
        });

        let surface = instance.create_surface(window).unwrap();

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .expect("Failed to find GPU adapter");

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("tunnelclient device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
            },
            None,
        ))
        .expect("Failed to create device");

        let surface_config = surface
            .get_default_config(&adapter, width, height)
            .expect("Surface not supported");
        let mut surface_config = surface_config;
        // Use non-sRGB format to avoid double gamma correction. Our HSV→RGB
        // conversion already produces sRGB values; an sRGB surface format would
        // apply gamma correction a second time, washing out colors.
        surface_config.format = surface_config.format.remove_srgb_suffix();
        surface_config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&device, &surface_config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniform buffer"),
            size: 8,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bind group layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("render pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                        wgpu::VertexAttribute {
                            offset: 8,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x4,
                        },
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 4,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        let msaa_texture = Self::create_msaa_texture(&device, &surface_config);

        queue.write_buffer(
            &uniform_buffer,
            0,
            bytemuck::cast_slice(&[width as f32, height as f32]),
        );

        // Start with empty buffers; they'll grow on the first frame.
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vertex buffer"),
            size: 0,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("index buffer"),
            size: 0,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            device,
            queue,
            surface,
            pipeline,
            uniform_buffer,
            uniform_bind_group,
            msaa_texture,
            surface_config,
            vertex_buffer,
            vertex_buffer_size: 0,
            index_buffer,
            index_buffer_size: 0,
        }
    }

    fn create_msaa_texture(
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
    ) -> wgpu::TextureView {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("msaa texture"),
            size: wgpu::Extent3d {
                width: config.width,
                height: config.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 4,
            dimension: wgpu::TextureDimension::D2,
            format: config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        texture.create_view(&Default::default())
    }

    fn render(&mut self, vertices: &[Vertex], indices: &[u32]) {
        let frame = match self.surface.get_current_texture() {
            Ok(f) => f,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&self.device, &self.surface_config);
                self.msaa_texture = Self::create_msaa_texture(&self.device, &self.surface_config);
                error!("Surface lost or outdated, reconfigured");
                return;
            }
            Err(e) => {
                error!("Failed to get surface texture: {e}");
                return;
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("render encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.msaa_texture,
                    resolve_target: Some(&view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });

            if !vertices.is_empty() && !indices.is_empty() {
                let vertex_bytes = bytemuck::cast_slice(vertices);
                let index_bytes = bytemuck::cast_slice(indices);

                // Grow buffers if needed, otherwise reuse.
                if vertex_bytes.len() as u64 > self.vertex_buffer_size {
                    self.vertex_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("vertex buffer"),
                        size: vertex_bytes.len() as u64,
                        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                    self.vertex_buffer_size = vertex_bytes.len() as u64;
                }
                if index_bytes.len() as u64 > self.index_buffer_size {
                    self.index_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("index buffer"),
                        size: index_bytes.len() as u64,
                        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                    self.index_buffer_size = index_bytes.len() as u64;
                }

                self.queue
                    .write_buffer(&self.vertex_buffer, 0, vertex_bytes);
                self.queue.write_buffer(&self.index_buffer, 0, index_bytes);

                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..vertex_bytes.len() as u64));
                pass.set_index_buffer(
                    self.index_buffer.slice(..index_bytes.len() as u64),
                    wgpu::IndexFormat::Uint32,
                );
                pass.draw_indexed(0..indices.len() as u32, 0, 0..1);
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();
    }
}

/// Accumulates tessellated geometry during a frame. The GPU renderer draws the accumulated
/// geometry in a single render pass. `clear()` ignores the color parameter because the actual
/// screen clear happens in the wgpu render pass's `LoadOp::Clear`.
struct FrameBuilder {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
}

impl FrameBuilder {
    fn new() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
        }
    }

    fn clear(&mut self) {
        self.vertices.clear();
        self.indices.clear();
    }
}

impl tunnelclient::render::RenderTarget for FrameBuilder {
    fn clear(&mut self, _color: [f32; 4]) {
        self.clear();
    }

    fn draw_triangles(&mut self, vertices: &[Vertex], indices: &[u32]) {
        let base = self.vertices.len() as u32;
        self.vertices.extend_from_slice(vertices);
        self.indices.extend(indices.iter().map(|i| i + base));
    }
}

pub enum AppEvent {
    NewConfig(ClientConfig, RunFlag),
}

struct ShowState {
    window: Arc<Window>,
    renderer: GpuRenderer,
    snapshot_manager: SnapshotManagerHandle,
    frame_builder: FrameBuilder,
    cfg: ClientConfig,
    run_flag: RunFlag,
}

struct ShowApp {
    ctx: Context,
    state: Option<ShowState>,
    pending_config: Option<(ClientConfig, RunFlag)>,
    config_rx: Option<Receiver<(ClientConfig, RunFlag)>>,
    event_proxy: Option<EventLoopProxy<AppEvent>>,
}

impl ShowApp {
    fn new_standalone(cfg: ClientConfig, ctx: Context, run_flag: RunFlag) -> Self {
        Self {
            ctx,
            state: None,
            pending_config: Some((cfg, run_flag)),
            config_rx: None,
            event_proxy: None,
        }
    }

    fn new_remote(
        ctx: Context,
        first_config: ClientConfig,
        first_flag: RunFlag,
        config_rx: Receiver<(ClientConfig, RunFlag)>,
    ) -> Self {
        Self {
            ctx,
            state: None,
            pending_config: Some((first_config, first_flag)),
            config_rx: Some(config_rx),
            event_proxy: None,
        }
    }

    fn setup_show(&mut self, event_loop: &ActiveEventLoop, cfg: ClientConfig, run_flag: RunFlag) {
        info!("Running on video channel {}.", cfg.video_channel);

        let snapshot_manager: SnapshotManagerHandle = Arc::new(Mutex::new(None));

        if let Err(e) =
            receive_snapshots(&self.ctx, &cfg, snapshot_manager.clone(), run_flag.clone())
        {
            error!("Failed to set up snapshot receiver: {e}");
            return;
        }

        let mut attrs = WindowAttributes::default()
            .with_title(format!("tunnelclient: channel {}", cfg.video_channel))
            .with_inner_size(LogicalSize::new(cfg.x_resolution, cfg.y_resolution));

        if cfg.fullscreen {
            attrs = attrs.with_fullscreen(Some(Fullscreen::Borderless(None)));
        }

        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .expect("Failed to create window"),
        );

        if cfg.capture_mouse {
            let _ = window.set_cursor_grab(winit::window::CursorGrabMode::Confined);
            window.set_cursor_visible(false);
        }

        let renderer = GpuRenderer::new(window.clone(), cfg.x_resolution, cfg.y_resolution);

        self.state = Some(ShowState {
            window,
            renderer,
            snapshot_manager,
            frame_builder: FrameBuilder::new(),
            cfg,
            run_flag,
        });
    }

    fn teardown_show(&mut self) {
        if let Some(mut state) = self.state.take() {
            state.run_flag.stop();
        }
    }
}

impl ApplicationHandler<AppEvent> for ShowApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Some((cfg, run_flag)) = self.pending_config.take() {
            self.setup_show(event_loop, cfg, run_flag);
        }

        // For remote mode, spawn a thread that forwards config updates to the event loop proxy
        if let (Some(rx), Some(proxy)) = (self.config_rx.take(), self.event_proxy.clone()) {
            thread::Builder::new()
                .name("config_forwarder".to_string())
                .spawn(move || {
                    while let Ok((cfg, flag)) = rx.recv() {
                        if proxy.send_event(AppEvent::NewConfig(cfg, flag)).is_err() {
                            break;
                        }
                    }
                })
                .expect("Failed to spawn config forwarder");
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::NewConfig(cfg, run_flag) => {
                info!("Received new config, tearing down current show.");
                self.teardown_show();
                self.setup_show(event_loop, cfg, run_flag);
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                self.teardown_show();
                event_loop.exit();
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key: Key::Named(NamedKey::Escape),
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => {
                self.teardown_show();
                event_loop.exit();
            }
            WindowEvent::RedrawRequested => {
                if let Some(ref mut state) = self.state {
                    if !state.run_flag.should_run() {
                        info!("Quit flag tripped, ending show.");
                        self.teardown_show();
                        event_loop.exit();
                        return;
                    }

                    let snapshot = state.snapshot_manager.lock().unwrap().clone();

                    state.frame_builder.clear();
                    if let Some(snapshot) = snapshot {
                        snapshot.draw(&mut state.frame_builder, &state.cfg);
                    }

                    state
                        .renderer
                        .render(&state.frame_builder.vertices, &state.frame_builder.indices);
                    state.window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

pub struct Show {
    cfg: ClientConfig,
    run_flag: RunFlag,
    ctx: Context,
}

impl Show {
    pub fn new(cfg: ClientConfig, ctx: Context, run_flag: RunFlag) -> Result<Self> {
        Ok(Show { cfg, run_flag, ctx })
    }

    pub fn run(self) {
        let event_loop = EventLoop::<AppEvent>::with_user_event()
            .build()
            .expect("Failed to create event loop");

        let mut app = ShowApp::new_standalone(self.cfg, self.ctx, self.run_flag);
        event_loop.run_app(&mut app).expect("Event loop error");
    }
}

pub fn run_remote_show(
    ctx: Context,
    first_config: ClientConfig,
    first_flag: RunFlag,
    config_rx: Receiver<(ClientConfig, RunFlag)>,
) {
    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .build()
        .expect("Failed to create event loop");

    let proxy = event_loop.create_proxy();
    let mut app = ShowApp::new_remote(ctx, first_config, first_flag, config_rx);
    app.event_proxy = Some(proxy);

    event_loop.run_app(&mut app).expect("Event loop error");
}

fn receive_snapshots(
    ctx: &Context,
    cfg: &ClientConfig,
    snapshot_manager: SnapshotManagerHandle,
    run_flag: RunFlag,
) -> Result<()> {
    let mut receiver: ZmqReceiver<Snapshot> = ZmqReceiver::new(
        ctx,
        &cfg.server_hostname,
        6000,
        Some(&[cfg.video_channel as u8]),
    )?;
    thread::Builder::new()
        .name("snapshot_receiver".to_string())
        .spawn(move || loop {
            if !run_flag.should_run() {
                info!("Snapshot receiver shutting down.");
                break;
            }
            match receiver.receive_msg(true) {
                Ok(Some(msg)) => {
                    *snapshot_manager.lock().unwrap() = Some(Arc::new(msg));
                }
                Ok(None) => continue,
                Err(e) => error!("receive error: {e}"),
            }
        })?;
    Ok(())
}
