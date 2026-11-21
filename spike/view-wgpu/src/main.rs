//! The garden with wgpu and winit by hand (POC_PLAN.md, step 0): the same surface, sun and
//! camera as `view-bevy`, in one file with its own shader, to see what Bevy saves and costs.
//! W A S D to move, Space and Shift up and down, the mouse held to look. The frames a second go
//! to the terminal every second; `PLATEAU_NO_VSYNC=1` to see how many it could.

use glam::{Mat4, Vec3};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;
use wgpu::util::DeviceExt;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

const SHADER: &str = r#"
struct Uniforms { view_proj: mat4x4<f32>, eye: vec4<f32>, sun: vec4<f32> };
@group(0) @binding(0) var<uniform> u: Uniforms;

struct In { @location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) colour: vec3<f32> };
struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) colour: vec3<f32>,
    @location(2) world: vec3<f32>,
};

@vertex fn vs(v: In) -> Out {
    return Out(u.view_proj * vec4(v.pos, 1.0), v.normal, v.colour, v.pos);
}

@fragment fn fs(o: Out) -> @location(0) vec4<f32> {
    let n = normalize(o.normal);
    let sun = max(dot(n, u.sun.xyz), 0.0) * 1.6;
    let sky = 0.35 + 0.15 * n.y;
    let lit = o.colour * (sun + sky);
    let fog = 1.0 - exp(-distance(o.world, u.eye.xyz) * 0.004);
    return vec4(mix(lit, vec3(0.30, 0.45, 0.75), fog), 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    pos: [f32; 3],
    normal: [f32; 3],
    colour: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    sun: [f32; 4],
}

struct Gpu {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    depth: wgpu::TextureView,
}

struct Camera {
    eye: Vec3,
    yaw: f32,
    pitch: f32,
}

impl Camera {
    fn forward(&self) -> Vec3 {
        Vec3::new(-self.yaw.sin() * self.pitch.cos(), self.pitch.sin(), -self.yaw.cos() * self.pitch.cos())
    }
}

struct App {
    gpu: Option<Gpu>,
    camera: Camera,
    keys: HashSet<KeyCode>,
    looking: bool,
    last: Instant,
    frames: u32,
    counted: Instant,
}

const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

fn depth_view(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d { width: config.width, height: config.height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

impl Gpu {
    fn new(window: Arc<Window>) -> Gpu {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let surface = instance.create_surface(window.clone()).expect("a surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .expect("an adapter");
        eprintln!("adapter: {:?}", adapter.get_info());
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).expect("a device");
        let size = window.inner_size();
        let mut config = surface.get_default_config(&adapter, size.width.max(1), size.height.max(1)).expect("a configuration");
        if std::env::var_os("PLATEAU_NO_VSYNC").is_some() {
            config.present_mode = wgpu::PresentMode::AutoNoVsync;
        }
        surface.configure(&device, &config);

        let garden = spike_host::garden_mesh().expect("the garden's module");
        let vertices: Vec<Vertex> = (0..garden.positions.len())
            .map(|i| Vertex { pos: garden.positions[i], normal: garden.normals[i], colour: spike_host::colour(garden.materials[i]) })
            .collect();
        let buffer = |label, contents: &[u8], usage| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents, usage })
        };
        let vertex_buffer = buffer("vertices", bytemuck::cast_slice(&vertices), wgpu::BufferUsages::VERTEX);
        let index_buffer = buffer("indices", bytemuck::cast_slice(&garden.indices), wgpu::BufferUsages::INDEX);
        let uniforms = buffer("uniforms", bytemuck::bytes_of(&Uniforms::zeroed()), wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("garden"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("garden"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            primitive: wgpu::PrimitiveState { cull_mode: Some(wgpu::Face::Back), ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(config.format.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let depth = depth_view(&device, &config);
        Gpu {
            window,
            surface,
            device,
            queue,
            config,
            pipeline,
            vertices: vertex_buffer,
            indices: index_buffer,
            count: garden.indices.len() as u32,
            uniforms,
            bind_group,
            depth,
        }
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.device, &self.config);
        self.depth = depth_view(&self.device, &self.config);
    }

    fn draw(&mut self, camera: &Camera) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            _ => return,
        };
        let aspect = self.config.width as f32 / self.config.height as f32;
        let proj = Mat4::perspective_rh(50f32.to_radians(), aspect, 0.05, 2000.0);
        let view = Mat4::look_to_rh(camera.eye, camera.forward(), Vec3::Y);
        let sun = (Vec3::new(10.0, 30.0, 50.0) - Vec3::new(32.0, 0.0, 32.0)).normalize();
        let uniforms = Uniforms { view_proj: (proj * view).to_cols_array_2d(), eye: camera.eye.extend(1.0).into(), sun: sun.extend(0.0).into() };
        self.queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.30, g: 0.45, b: 0.75, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertices.slice(..));
            pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.count, 0, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        self.window.pre_present_notify();
        self.queue.present(frame);
    }
}

use bytemuck::Zeroable;

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_none() {
            let attributes = Window::default_attributes().with_title("Plateau spike: wgpu");
            let window = Arc::new(event_loop.create_window(attributes).expect("a window"));
            self.gpu = Some(Gpu::new(window));
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.resize(size.width, size.height);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    if event.state == ElementState::Pressed { self.keys.insert(code) } else { self.keys.remove(&code) };
                }
            }
            WindowEvent::MouseInput { button: MouseButton::Left, state, .. } => self.looking = state == ElementState::Pressed,
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = (now - self.last).as_secs_f32();
                self.last = now;
                self.step(dt);
                if let Some(gpu) = &mut self.gpu {
                    gpu.draw(&self.camera);
                    gpu.window.request_redraw();
                }
                self.frames += 1;
                if self.counted.elapsed().as_secs_f32() >= 1.0 {
                    eprintln!("fps: {:.1}", self.frames as f32 / self.counted.elapsed().as_secs_f32());
                    self.frames = 0;
                    self.counted = Instant::now();
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: winit::event::DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event
            && self.looking
        {
            self.camera.yaw -= delta.0 as f32 * 0.003;
            self.camera.pitch = (self.camera.pitch - delta.1 as f32 * 0.003).clamp(-1.5, 1.5);
        }
    }
}

impl App {
    fn step(&mut self, dt: f32) {
        let forward = self.camera.forward();
        let right = forward.cross(Vec3::Y).normalize();
        let mut go = Vec3::ZERO;
        for (key, way) in [(KeyCode::KeyW, forward), (KeyCode::KeyS, -forward), (KeyCode::KeyD, right), (KeyCode::KeyA, -right),
            (KeyCode::Space, Vec3::Y), (KeyCode::ShiftLeft, -Vec3::Y)]
        {
            if self.keys.contains(&key) {
                go += way;
            }
        }
        self.camera.eye += go.normalize_or_zero() * 8.0 * dt;
    }
}

fn main() {
    let event_loop = EventLoop::new().expect("an event loop");
    let mut app = App {
        gpu: None,
        camera: Camera { eye: Vec3::new(56.0, 9.0, 60.0), yaw: 0.7, pitch: -0.3 },
        keys: HashSet::new(),
        looking: false,
        last: Instant::now(),
        frames: 0,
        counted: Instant::now(),
    };
    event_loop.run_app(&mut app).expect("the event loop");
}
