//! An area of `areas/` played natively, as a `Region`: what its module stands for in the tests
//! (POC_PLAN.md, "Tests"), its answers made the engine's own.

use plateau_engine::{
    Answer, Borders, Command, Description, Edge, Instance, Light, Material, MeshData, Place, PlayerState, Region, Result, Scene, Size, Vertex,
};
use plateau_sdk::native::Played;

pub struct Native(pub Box<dyn Played>);

/// The area of `areas/<name>/` played natively, with its seed.
pub fn area(name: &str, seed: u64) -> Native {
    let (_, played) = areas_played::AREAS.iter().find(|(n, _)| *n == name).expect("an area of areas/");
    Native(played(seed))
}

fn to_sdk_ps(p: &PlayerState) -> plateau_sdk::PlayerState {
    let pt = |v: [f32; 3]| plateau_sdk::point(v[0], v[1], v[2]);
    plateau_sdk::PlayerState {
        feet: pt(p.feet),
        eye: pt(p.eye),
        look: pt(p.look),
        velocity: pt(p.velocity),
        size: p.size,
        grounded: p.grounded,
        holding_use: p.holding_use,
        target: p.target,
        aim: pt(p.aim),
        time: p.time,
    }
}

fn from_sdk_inst(i: plateau_sdk::Instance) -> Instance {
    Instance {
        id: i.id,
        mesh: i.mesh,
        pos: [i.pos.x, i.pos.y, i.pos.z],
        rot: [i.rot_x, i.rot_y, i.rot_z, i.rot_w],
        scale: [i.scale.x, i.scale.y, i.scale.z],
        solid: i.solid,
    }
}

fn from_sdk_light(l: plateau_sdk::Light) -> Light {
    Light { id: l.id, pos: [l.pos.x, l.pos.y, l.pos.z], color: [l.r, l.g, l.b], intensity: l.intensity, radius: l.radius }
}

fn from_sdk_cmds(cmds: Vec<plateau_sdk::Command>) -> Vec<Command> {
    cmds.into_iter()
        .map(|c| match c {
            plateau_sdk::Command::SetTransform(i) => Command::SetTransform(from_sdk_inst(i)),
            plateau_sdk::Command::SetVisible((id, v)) => Command::SetVisible(id, v),
            plateau_sdk::Command::SetSolid((id, s)) => Command::SetSolid(id, s),
            plateau_sdk::Command::ApplyImpulse(p) => Command::ApplyImpulse([p.x, p.y, p.z]),
            plateau_sdk::Command::SetLight(l) => Command::SetLight(from_sdk_light(l)),
            plateau_sdk::Command::Say(s) => Command::Say(s),
        })
        .collect()
}

impl Region for Native {
    fn sample(&mut self, points: &[[f32; 3]], size: f32, out: &mut Vec<Answer>) -> Result<()> {
        let points: Vec<_> = points.iter().map(|&[x, y, z]| plateau_sdk::point(x, y, z)).collect();
        let mut answers = Vec::new();
        self.0.sample(&points, size, &mut answers);
        out.clear();
        out.extend(answers.iter().map(|a| Answer { distance: a.distance, material: Material::from_number(a.material as u8).unwrap() }));
        Ok(())
    }

    fn describe(&mut self) -> Result<Description> {
        let d = self.0.describe();
        let edge = |e: &plateau_sdk::Edge| Edge { ground: e.ground.clone(), water: e.water };
        let b = &d.borders;
        Ok(Description {
            name: d.name.clone(),
            places: d
                .places
                .iter()
                .map(|p| Place { name: p.name.clone(), size: Size::ALL[p.size as usize], at: [p.at.x, p.at.y, p.at.z] })
                .collect(),
            borders: Borders { north: edge(&b.north), east: edge(&b.east), south: edge(&b.south), west: edge(&b.west) },
        })
    }

    fn scene(&mut self) -> Result<Scene> {
        let s = self.0.scene();
        Ok(Scene {
            meshes: s
                .meshes
                .into_iter()
                .map(|m| MeshData {
                    vertices: m
                        .vertices
                        .into_iter()
                        .map(|v| Vertex {
                            pos: [v.pos.x, v.pos.y, v.pos.z],
                            normal: [v.normal.x, v.normal.y, v.normal.z],
                            material: Material::from_number(v.material as u8).unwrap(),
                            tint: [v.tint_r, v.tint_g, v.tint_b],
                        })
                        .collect(),
                    indices: m.indices,
                })
                .collect(),
            instances: s.instances.into_iter().map(from_sdk_inst).collect(),
            lights: s.lights.into_iter().map(from_sdk_light).collect(),
        })
    }

    fn tick(&mut self, dt: f32, player: &PlayerState) -> Result<Vec<Command>> {
        Ok(from_sdk_cmds(self.0.tick(dt, &to_sdk_ps(player))))
    }

    fn interact(&mut self, player: &PlayerState) -> Result<Vec<Command>> {
        Ok(from_sdk_cmds(self.0.interact(&to_sdk_ps(player))))
    }
}
