//! Live multi-scale SDF sculpting & digging in `plateau-play` (POC_PLAN.md, step 8a).
//!
//! Toggle build/sculpt mode with `B`:
//! - **Left-click**: add material (raise a hill, place a block/wall, build a room)
//! - **Right-click** or **`X`**: carve material (dig a cave/tunnel, hollow a room)
//! - **Mouse wheel** or **`[` / `]`**: resize brush relative to current player size
//! - **`Tab`**: cycle brush preset (smooth hill/cave sphere, flat-floor mound/cave cylinder, sharp block box, smooth box)
//! - **`R`**: cycle material
//! - **`Z` / `Y`**: undo / redo with live chunk re-invalidation
//! - **`F5`** (or **`Ctrl+S`**) / **`F9`**: save / reload edits from disk (`.plateau` file)

use std::path::PathBuf;
use std::sync::Arc;

use bevy::prelude::*;
use plateau_engine::compose::PlacedElement;
use plateau_engine::edit::{Edit, EditLog, Op, Shape, WorldSave};
use plateau_engine::picture::colour;
use plateau_engine::{Material, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrushPreset {
    HillSphere,
    MoundCylinder,
    BlockBox,
    SmoothBox,
}

impl BrushPreset {
    pub const ALL: [BrushPreset; 4] = [
        BrushPreset::HillSphere,
        BrushPreset::MoundCylinder,
        BrushPreset::BlockBox,
        BrushPreset::SmoothBox,
    ];

    pub fn name(self) -> &'static str {
        match self {
            BrushPreset::HillSphere => "sphere (smooth)",
            BrushPreset::MoundCylinder => "cylinder (smooth)",
            BrushPreset::BlockBox => "block (sharp)",
            BrushPreset::SmoothBox => "box (smooth)",
        }
    }

    pub fn next(self) -> BrushPreset {
        let i = BrushPreset::ALL.iter().position(|&p| p == self).unwrap_or(0);
        BrushPreset::ALL[(i + 1) % BrushPreset::ALL.len()]
    }

    pub fn shape_and_blend(self, r: f32) -> (Shape, f32) {
        match self {
            BrushPreset::HillSphere => (Shape::Sphere { radius: r }, r * 0.35),
            BrushPreset::MoundCylinder => (Shape::Cylinder { radius: r, half_height: r * 0.65, round: r * 0.18 }, r * 0.25),
            BrushPreset::BlockBox => (Shape::Box { half: [r * 0.85, r * 0.85, r * 0.85], round: r * 0.06 }, 0.0),
            BrushPreset::SmoothBox => (Shape::Box { half: [r, r * 0.7, r], round: r * 0.15 }, r * 0.22),
        }
    }

    /// World-space center given a surface hit and outward normal.
    /// Sharp blocks sit on top of the aimed surface (overlapping slightly to stay solid);
    /// organic spheres/cylinders and carves center right on the hit point.
    pub fn placement_center(self, op: Op, hit: [f64; 3], normal: [f32; 3], r: f32) -> [f32; 3] {
        let lift = match (op, self) {
            (Op::Add, BrushPreset::BlockBox) => r * 0.65,
            (Op::Add, BrushPreset::SmoothBox) => r * 0.45,
            (Op::Add, BrushPreset::MoundCylinder) => r * 0.2,
            _ => 0.0,
        };
        [0, 1, 2].map(|a| (hit[a] + (normal[a] * lift) as f64) as f32)
    }
}

#[derive(Resource)]
pub struct Sculpt {
    pub active: bool,
    pub preset: BrushPreset,
    pub material_idx: usize,
    /// Brush radius as a multiple of `player.body.size`.
    pub radius_mult: f32,
    /// Current immutable edit log snapshot shared with `ComposedLog`.
    pub edits: Arc<EditLog>,
    pub save_path: PathBuf,
    pub area_name: String,
    pub seed: u64,
    pub dirty: bool,
    /// Latest aimed surface `(point, normal)` while sculpt mode is active.
    pub aim: Option<([f64; 3], [f32; 3])>,
    /// Cooldown timer for continuous hold-to-sculpt/dig strokes.
    pub hold_timer: f32,
    /// Center of the last stamped edit, to space out continuous strokes cleanly.
    pub last_stamp: Option<[f32; 3]>,
}

impl Sculpt {
    pub fn new(edits: Arc<EditLog>, save_path: PathBuf, area_name: String, seed: u64) -> Sculpt {
        Sculpt {
            active: false,
            preset: BrushPreset::HillSphere,
            material_idx: 0, // Soil
            radius_mult: 0.75,
            edits,
            save_path,
            area_name,
            seed,
            dirty: false,
            aim: None,
            hold_timer: 0.0,
            last_stamp: None,
        }
    }

    pub fn material(&self) -> Material {
        Material::ALL[self.material_idx % Material::ALL.len()]
    }

    pub fn next_material(&mut self) {
        self.material_idx = (self.material_idx + 1) % Material::ALL.len();
    }

    pub fn adjust_radius(&mut self, factor: f32) {
        self.radius_mult = (self.radius_mult * factor).clamp(0.2, 2.5);
    }

    pub fn radius(&self, player_size: f32) -> f32 {
        (self.radius_mult * player_size).max(0.003)
    }

    pub fn make_edit(&self, op: Op, player_size: f32, hit: [f64; 3], normal: [f32; 3]) -> Edit {
        let r = self.radius(player_size);
        let (shape, blend) = self.preset.shape_and_blend(r);
        let center = self.preset.placement_center(op, hit, normal, r);
        match op {
            Op::Add => Edit::add(shape, center, blend, self.material(), player_size),
            Op::Carve => Edit::carve(shape, center, blend, None, player_size),
        }
    }

    pub fn push_edit(&mut self, edit: Edit) -> Result<()> {
        let mut next = (*self.edits).clone();
        next.push(edit)?;
        self.edits = Arc::new(next);
        self.dirty = true;
        Ok(())
    }

    pub fn pop_edit(&mut self) -> Option<Edit> {
        let mut next = (*self.edits).clone();
        let removed = next.pop()?;
        self.edits = Arc::new(next);
        self.dirty = true;
        Some(removed)
    }

    pub fn replace_edits(&mut self, edits: Vec<Edit>) -> Result<()> {
        let next_rev = self.edits.revision() + 1;
        self.edits = Arc::new(EditLog::from_edits_with_revision(edits, next_rev)?);
        self.dirty = false;
        Ok(())
    }

    pub fn save_world(&mut self, elements: &[PlacedElement]) -> Result<()> {
        let world = WorldSave::with_elements(self.area_name.clone(), self.seed, self.edits.edits().to_vec(), elements.to_vec());
        world.save(&self.save_path)?;
        self.dirty = false;
        Ok(())
    }

    pub fn load_world(&self) -> Result<WorldSave> {
        WorldSave::load(&self.save_path)
    }
}

/// Marker component for the translucent 3D sculpt brush preview ghost.
#[derive(Component)]
pub struct BrushGhost;

#[derive(Resource)]
pub struct GhostAssets {
    sphere: Handle<Mesh>,
    cube: Handle<Mesh>,
    cylinder: Handle<Mesh>,
    material: Handle<StandardMaterial>,
}

pub fn spawn_ghost(commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) {
    let sphere = meshes.add(Sphere::new(1.0).mesh().ico(3).unwrap());
    let cube = meshes.add(Cuboid::new(2.0, 2.0, 2.0));
    let cylinder = meshes.add(Cylinder::new(1.0, 2.0));
    let material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.55, 0.85, 0.95, 0.28),
        emissive: LinearRgba::rgb(0.15, 0.35, 0.45),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    commands.spawn((
        Mesh3d(sphere.clone()),
        MeshMaterial3d(material.clone()),
        Transform::default(),
        Visibility::Hidden,
        BrushGhost,
    ));
    commands.insert_resource(GhostAssets { sphere, cube, cylinder, material });
}

/// Positions and sizes the translucent brush preview ghost at the player's current aim point.
pub fn place_ghost(
    sculpt: Res<Sculpt>,
    player: Res<crate::Player>,
    assets: Res<GhostAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut ghost_q: Query<(&mut Transform, &mut Visibility, &mut Mesh3d), With<BrushGhost>>,
) {
    let Ok((mut tf, mut vis, mut mesh3d)) = ghost_q.single_mut() else { return };
    if !sculpt.active {
        vis.set_if_neq(Visibility::Hidden);
        return;
    }
    let Some((hit, normal)) = sculpt.aim else {
        vis.set_if_neq(Visibility::Hidden);
        return;
    };
    let player_size = player.body.size;
    let eye = player.eye.unwrap_or(player.body.eye());
    let scale = (crate::DRAWN_AS / player_size) as f64;
    let r = sculpt.radius(player_size);
    let (shape, _) = sculpt.preset.shape_and_blend(r);
    let center = sculpt.preset.placement_center(Op::Add, hit, normal, r);
    let (target_mesh, ext) = match shape {
        Shape::Sphere { radius } => (&assets.sphere, [radius, radius, radius]),
        Shape::Box { half, .. } => (&assets.cube, half),
        Shape::Cylinder { radius, half_height, .. } => (&assets.cylinder, [radius, half_height, radius]),
    };
    if mesh3d.0 != *target_mesh {
        mesh3d.0 = target_mesh.clone();
    }
    if let Some(mut mat) = materials.get_mut(&assets.material) {
        let [cr, cg, cb] = colour(sculpt.material());
        mat.base_color = Color::srgba(cr, cg, cb, 0.32);
        mat.emissive = LinearRgba::rgb(cr * 0.35, cg * 0.35, cb * 0.35);
    }
    tf.translation = Vec3::new(
        ((center[0] as f64 - eye[0]) * scale) as f32,
        ((center[1] as f64 - eye[1]) * scale) as f32,
        ((center[2] as f64 - eye[2]) * scale) as f32,
    );
    tf.rotation = Quat::IDENTITY;
    tf.scale = Vec3::new(ext[0] * scale as f32, ext[1] * scale as f32, ext[2] * scale as f32);
    vis.set_if_neq(Visibility::Visible);
}
