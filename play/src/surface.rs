//! Chunk and prop surface material (POC_PLAN.md, step 7): extends Bevy's `StandardMaterial` with
//! a 4×4 Bayer dithered LOD cross-fade and world-locked procedural micro-grain + glossy water.

use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use plateau_engine::Material as AreaMaterial;

pub const FADE_STEPS: usize = 8;
pub const FADE_SECONDS: f32 = 0.22;
const GRAIN_PERIOD: f64 = 16.0;

pub type SurfaceMaterial = ExtendedMaterial<StandardMaterial, SurfaceExt>;

#[derive(Clone, Copy, Debug, Default, Reflect, ShaderType)]
pub struct SurfaceParams {
    pub fade_mode: Vec4,
    pub eye_mod: Vec4,
}

#[derive(Asset, AsBindGroup, Reflect, Clone, Debug, Default)]
pub struct SurfaceExt {
    #[uniform(100)]
    pub params: SurfaceParams,
}

impl MaterialExtension for SurfaceExt {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path("embedded://plateau_play/surface.wesl".into())
    }
}

#[derive(Clone)]
pub struct Materials {
    pub opaque: Handle<SurfaceMaterial>,
    pub fade_in: [Handle<SurfaceMaterial>; FADE_STEPS],
    pub fade_out: [Handle<SurfaceMaterial>; FADE_STEPS],
}

impl Materials {
    pub fn new(assets: &mut Assets<SurfaceMaterial>) -> Materials {
        let base = StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.92,
            reflectance: 0.3,
            cull_mode: None,
            ..default()
        };
        let mut make = |threshold: f32, mode: f32| {
            assets.add(ExtendedMaterial {
                base: base.clone(),
                extension: SurfaceExt {
                    params: SurfaceParams {
                        fade_mode: Vec4::new(threshold, mode, 1.0, 0.0),
                        eye_mod: Vec4::new(0.0, 0.0, 0.0, GRAIN_PERIOD as f32),
                    },
                },
            })
        };
        let opaque = make(1.0, 0.0);
        let fade_in = std::array::from_fn(|k| make((k + 1) as f32 / (FADE_STEPS + 1) as f32, 1.0));
        let fade_out = std::array::from_fn(|k| make((k + 1) as f32 / (FADE_STEPS + 1) as f32, -1.0));
        Materials { opaque, fade_in, fade_out }
    }

    pub fn sync_eye(&self, assets: &mut Assets<SurfaceMaterial>, eye: [f64; 3], scale: f64) {
        let m = |c: f64| (c * scale).rem_euclid(GRAIN_PERIOD) as f32;
        let eye_mod = Vec4::new(m(eye[0]), m(eye[1]), m(eye[2]), GRAIN_PERIOD as f32);
        for handle in std::iter::once(&self.opaque).chain(&self.fade_in).chain(&self.fade_out) {
            if let Some(mut mat) = assets.get_mut(handle) {
                mat.extension.params.eye_mod = eye_mod;
            }
        }
    }

    pub fn incoming(&self, t: f32) -> &Handle<SurfaceMaterial> {
        let idx = ((t * FADE_STEPS as f32) as usize).min(FADE_STEPS - 1);
        &self.fade_in[idx]
    }

    pub fn outgoing(&self, t: f32) -> &Handle<SurfaceMaterial> {
        let idx = ((t * FADE_STEPS as f32) as usize).min(FADE_STEPS - 1);
        &self.fade_out[idx]
    }
}

/// Per-material `(perceptual_roughness, micro_grain_amplitude)` packed into `ATTRIBUTE_UV_0`.
pub fn properties(m: AreaMaterial) -> [f32; 2] {
    match m {
        AreaMaterial::Water => [0.08, 0.0],
        AreaMaterial::Stone => [0.76, 0.09],
        AreaMaterial::Wood => [0.80, 0.09],
        AreaMaterial::Sand => [0.88, 0.08],
        AreaMaterial::Soil => [0.84, 0.08],
        AreaMaterial::Mud => [0.48, 0.06],
        AreaMaterial::Grass => [0.92, 0.07],
        AreaMaterial::Leaf => [0.88, 0.07],
        AreaMaterial::Moss => [0.94, 0.08],
    }
}
