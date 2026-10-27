// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! bevy_gl_rec: what Slint's Bevy demo (examples/bevy/bevy-hosts-slint) asks
//! of an OpenGL ES 3.0 driver, measured on the host without rendering.
//!
//! Bevy runs headless with its wgpu renderer on wgpu's GLES backend, whose GL
//! is `fakegl`: it reports what YakoGL reports on the Pi 4 ("OpenGL ES 3.0",
//! GLSL ES 3.00, YakoGL's limits), accepts every shader and renders nothing.
//! So naga emits the GLSL ES 3.00 that YakoGL would receive, and the draws,
//! textures and uploads of each frame are counted.
//!
//! The scene is the demo's (a cube with a lit StandardMaterial, a quad with an
//! unlit, alpha-blended 600x600 RGBA texture re-uploaded every frame in place
//! of Slint's software-rendered UI, the demo's glTF cow, a point light with
//! shadow maps, a Camera3d and Bevy UI text), rendered to a 1024x600 image
//! instead of a window. One difference is forced: YakoGL reports
//! GL_MAX_SAMPLES 1, so the camera is single-sampled (Bevy's default is 4x).
//!
//! Usage: bevy_gl_rec <demo assets dir> <output dir> [frames]
//! Writes <output dir>/NNN.vert and NNN.frag per linked program and prints
//! one line per frame.
//!
//! Two things a native OpenGL ES 3.0 target needs that stock Bevy 0.19 and
//! YakoGL do not give it, both measured with this tool:
//!
//! - GL_MAX_UNIFORM_BLOCK_SIZE: YakoGL reports 6144 bytes (ES 3.0's minimum is
//!   16384), and wgpu refuses Bevy's view and skinned-mesh bind groups (16,320
//!   and 16,384 bytes), so no frame is drawn. `BEVY_GL_REC_UBO_BYTES=16384`
//!   answers the ES 3.0 minimum instead, to measure the frame past it.
//! - Bevy 0.19 applies its GLES 3.0 profile (no array or cube-array textures,
//!   16-byte uniform alignment, the matching bind-group layouts) only on
//!   wasm32. `bevy-0.19.1-gles3-native.patch` makes it follow the `webgl`
//!   feature instead (65 lines in bevy_render, bevy_pbr, bevy_core_pipeline and
//!   bevy_post_process). Without it the PBR shaders do not compose ("Capability
//!   CUBE_ARRAY_TEXTURES is required"). Apply it to copies of the four crates
//!   and build with
//!   `--config 'patch.crates-io.bevy_render.path="<dir>/bevy_render"'` (and the
//!   same for the other three).

mod fakegl;

use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::renderer::{
    RenderAdapter, RenderAdapterInfo, RenderDevice, RenderInstance, RenderQueue, WgpuWrapper,
};
use bevy::render::settings::RenderCreation;
use bevy::render::RenderPlugin;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::{ExitCondition, WindowPlugin};
use bevy::world_serialization::WorldAssetRoot;

#[derive(Component)]
struct SlintTexture(Handle<Image>);

fn render_creation() -> RenderCreation {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::GL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    // SAFETY: fakegl has no context to be current; every entry point is ours.
    let exposed = unsafe {
        wgpu::hal::gles::Adapter::new_external(fakegl::get_proc, wgpu::GlBackendOptions::default())
    }
    .expect("wgpu refused the fake GL as an adapter");
    // SAFETY: the adapter came from this backend.
    let adapter = unsafe { instance.create_adapter_from_hal(exposed) };
    let info = adapter.get_info();
    println!("adapter: {} / {} ({:?})", info.name, info.driver_info, info.backend);
    println!("downlevel: {:?}", adapter.get_downlevel_capabilities().flags);
    let limits = adapter.limits();
    println!(
        "limits: max_uniform_buffer_binding_size {}, max_storage_buffers_per_shader_stage {}, \
         max_samplers_per_shader_stage {}, max_sampled_textures_per_shader_stage {}",
        limits.max_uniform_buffer_binding_size,
        limits.max_storage_buffers_per_shader_stage,
        limits.max_samplers_per_shader_stage,
        limits.max_sampled_textures_per_shader_stage
    );
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("bevy_gl_rec"),
        required_features: wgpu::Features::empty(),
        required_limits: limits,
        ..Default::default()
    }))
    .expect("request_device");
    RenderCreation::manual(
        RenderDevice::from(device),
        RenderQueue(Arc::new(WgpuWrapper::new(queue))),
        RenderAdapterInfo(WgpuWrapper::new(info)),
        RenderAdapter(Arc::new(WgpuWrapper::new(adapter))),
        RenderInstance(Arc::new(WgpuWrapper::new(instance))),
    )
}

fn setup(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // the Slint UI's texture (600x600 RGBA8, re-uploaded every frame)
    let ui = Image::new_fill(
        Extent3d { width: 600, height: 600, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &[40, 120, 200, 255],
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::default(),
    );
    let ui_handle = images.add(ui);
    let ui_material = materials.add(StandardMaterial {
        base_color_texture: Some(ui_handle.clone()),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        ..default()
    });
    commands.spawn(SlintTexture(ui_handle));
    let cube_material =
        materials.add(StandardMaterial { base_color: Color::srgb(0.2, 0.7, 0.9), unlit: false, ..default() });
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let quad = meshes.add(Rectangle::new(1.0, 1.0));
    commands
        .spawn((
            Mesh3d(cube),
            MeshMaterial3d(cube_material),
            Transform::from_xyz(0.0, 0.0, -0.5).with_rotation(Quat::from_rotation_y(0.5)).with_scale(Vec3::splat(2.0)),
        ))
        .with_children(|p| {
            p.spawn((Mesh3d(quad), MeshMaterial3d(ui_material), Transform::from_xyz(0.0, 0.0, 0.5001)));
        });
    commands.spawn((
        WorldAssetRoot(assets.load("cow.gltf#Scene0")),
        Transform::from_scale(Vec3::splat(4.0)).with_translation(Vec3::new(-2.0, 2.0, -30.0)),
    ));
    commands.spawn((
        PointLight { intensity: 2_000_000.0, range: 100.0, shadow_maps_enabled: true, ..default() },
        Transform::from_xyz(8.0, 16.0, 8.0),
    ));
    let target = images.add(Image::new_target_texture(1024, 600, TextureFormat::Rgba8UnormSrgb, None));
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 0.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
        Camera { clear_color: ClearColorConfig::Custom(Color::srgb(0.1, 0.1, 0.1)), ..default() },
        RenderTarget::Image(target.into()),
        Msaa::Off,
    ));
    commands
        .spawn((
            Node { position_type: PositionType::Absolute, left: Val::Px(10.0), top: Val::Px(10.0), ..default() },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
        ))
        .with_children(|p| {
            p.spawn(Text::new("Slint + Bevy Integration Demo"));
        });
}

/// What the demo's render_slint does every frame: new pixels in the UI texture.
fn touch_ui(q: Query<&SlintTexture>, mut images: ResMut<Assets<Image>>, mut n: Local<u8>) {
    *n = n.wrapping_add(1);
    for t in &q {
        if let Some(mut img) = images.get_mut(&t.0) {
            if let Some(d) = img.data.as_mut() {
                d[0] = *n;
            }
        }
    }
}

/// Rotate the cube, as the demo's keyboard does, so frames differ.
fn spin(mut q: Query<&mut Transform, With<Mesh3d>>, time: Res<Time>) {
    for mut t in &mut q {
        t.rotate_y(0.3 * time.delta_secs());
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let assets = args.get(1).expect("usage: bevy_gl_rec <demo assets dir> <output dir> [frames]").clone();
    let out = std::path::PathBuf::from(args.get(2).expect("output dir"));
    let frames: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(120);
    std::fs::create_dir_all(&out).unwrap();

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(RenderPlugin {
                render_creation: render_creation(),
                synchronous_pipeline_compilation: true,
                ..default()
            })
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
            .set(AssetPlugin { file_path: assets, ..default() }),
    )
    .add_systems(Startup, setup)
    .add_systems(Update, (touch_ui, spin));
    app.finish();
    app.cleanup();

    let mut nprog = 0usize;
    for f in 0..frames {
        app.update();
        let fr = fakegl::take_frame();
        for p in fakegl::take_programs() {
            std::fs::write(out.join(format!("{nprog:03}.vert")), &p.vs).unwrap();
            std::fs::write(out.join(format!("{nprog:03}.frag")), &p.fs).unwrap();
            nprog += 1;
        }
        println!(
            "frame {f}: {} draws ({} instanced, max {} instances, {} vertices), {} programs, depth {} stencil {} blend {}, \
             fbo binds {}, ubo binds {}, textures by target {:?}, tex uploads {} ({} B), buffer uploads {} ({} B), new textures {:?}",
            fr.draws, fr.instanced, fr.max_instances, fr.vertices, fr.programs_used.len(), fr.depth_draws, fr.stencil_draws,
            fr.blend_draws, fr.fbo_binds, fr.ubo_binds, fr.tex_targets, fr.tex_uploads, fr.tex_bytes, fr.buf_uploads,
            fr.buf_bytes, fr.new_textures
        );
    }
    println!("programs linked: {nprog}");
}
