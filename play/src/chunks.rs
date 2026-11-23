//! The area's surface about the player, made off the frame (POC_PLAN.md, steps 2 and 7): view-
//! directed `6×4×6` rings shifted forward along the look direction, frustum-prioritized chunk
//! streaming, and complementary 4×4 Bayer dithered LOD cross-fades.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use plateau_engine::chunk::{self, Key, Made};
use plateau_engine::picture::colour;
use plateau_engine::{Compiled, rings};

use crate::surface::{FADE_SECONDS, Materials, SurfaceMaterial, properties};

const MOST_KEPT: usize = 12_000;
const MOST_A_FRAME: usize = 64;

enum Known {
    Empty,
    Shown { entity: Entity, triangles: usize, wanted: u64 },
}

struct Jobs {
    todo: Vec<Key>,
    making: HashSet<Key>,
    stop: bool,
}

struct Queue {
    jobs: Mutex<Jobs>,
    more: Condvar,
}

struct Done {
    key: Key,
    made: Result<Made, String>,
    took: Duration,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Counts {
    pub shown: usize,
    pub empty: usize,
    pub waiting: usize,
    pub holes: usize,
    pub ahead: usize,
    pub triangles: usize,
    pub longest: Duration,
    pub made: usize,
    pub slowest: Duration,
    pub total: Duration,
}

#[derive(Resource)]
pub struct Chunks {
    queue: Arc<Queue>,
    done: Mutex<Receiver<Done>>,
    known: HashMap<Key, Known>,
    rings: rings::Rings,
    wanted: HashSet<Key>,
    shown: HashSet<Key>,
    fading_in: HashMap<Key, f32>,
    fading_out: HashMap<Key, f32>,
    frame: u64,
    pub materials: Materials,
    pub counts: Counts,
}

impl Chunks {
    pub fn new(compiled: Arc<Compiled>, seed: u64, materials: Materials) -> Chunks {
        let queue = Arc::new(Queue { jobs: Mutex::new(Jobs { todo: Vec::new(), making: HashSet::new(), stop: false }), more: Condvar::new() });
        let (tell, done) = channel();
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).saturating_sub(2).max(1);
        for k in 0..threads {
            let (queue, tell, compiled) = (queue.clone(), tell.clone(), compiled.clone());
            std::thread::Builder::new().name(format!("chunks {k}")).spawn(move || work(&queue, &tell, &compiled, seed)).expect("a thread to make chunks");
        }
        Chunks {
            queue,
            done: Mutex::new(done),
            known: HashMap::new(),
            rings: rings::Rings::default(),
            wanted: HashSet::new(),
            shown: HashSet::new(),
            fading_in: HashMap::new(),
            fading_out: HashMap::new(),
            frame: 0,
            materials,
            counts: Counts::default(),
        }
    }

    pub fn update(
        &mut self,
        eye: [f64; 3],
        look: [f64; 3],
        size: f32,
        open: i32,
        ahead: &[([f64; 3], f32, i32)],
        dt: f32,
        commands: &mut Commands,
        meshes: &mut Assets<Mesh>,
    ) {
        self.frame += 1;
        let order = |keys: &mut Vec<Key>, eye: [f64; 3]| {
            let score = |k: &Key| -> (u8, i32, f64) {
                let m = k.middle();
                let (dx, dy, dz) = (m[0] - eye[0], m[1] - eye[1], m[2] - eye[2]);
                let len = (dx * dx + dy * dy + dz * dz).sqrt();
                let dist = len / k.side();
                let cos = if len > 1e-6 { (dx * look[0] + dy * look[1] + dz * look[2]) / len } else { 1.0 };
                let bucket = if dist < 1.3 || cos > 0.25 {
                    0
                } else if cos > -0.2 {
                    1
                } else {
                    2
                };
                (bucket, k.level, dist)
            };
            keys.sort_by(|a, b| {
                let (sa, sb) = (score(a), score(b));
                sa.0.cmp(&sb.0).then(sa.1.cmp(&sb.1)).then(sa.2.total_cmp(&sb.2))
            });
        };
        let look2 = [look[0], look[2]];
        let mut wanted = self.rings.wanted_looking(eye, look2, size, open);
        order(&mut wanted, eye);
        self.wanted = wanted.iter().copied().collect();
        let mut later = Vec::new();
        for &(e, s, o) in ahead {
            let mut keys: Vec<Key> = self.rings.clone().wanted_looking(e, look2, s, o).into_iter().filter(|k| !self.wanted.contains(k)).collect();
            order(&mut keys, e);
            later.extend(keys);
        }
        {
            let mut jobs = self.queue.jobs.lock().unwrap();
            let making = &jobs.making;
            let mut seen = HashSet::new();
            let todo: Vec<Key> =
                wanted.iter().chain(&later).filter(|k| !self.known.contains_key(k) && !making.contains(k) && seen.insert(**k)).copied().collect();
            jobs.todo = todo.into_iter().rev().collect();
            if !jobs.todo.is_empty() {
                self.queue.more.notify_all();
            }
        }
        let mut fresh = HashSet::new();
        let made: Vec<Done> = self.done.lock().unwrap().try_iter().take(MOST_A_FRAME).collect();
        for done in made {
            self.queue.jobs.lock().unwrap().making.remove(&done.key);
            self.counts.made += 1;
            self.counts.slowest = self.counts.slowest.max(done.took);
            self.counts.total += done.took;
            let known = match done.made {
                Ok(Made { mesh: Some(mesh), .. }) => {
                    let triangles = mesh.indices.len() / 3;
                    let entity = commands
                        .spawn((
                            Mesh3d(meshes.add(to_bevy(mesh))),
                            MeshMaterial3d(self.materials.opaque.clone()),
                            Transform::default(),
                            Visibility::Hidden,
                        ))
                        .id();
                    fresh.insert(done.key);
                    Known::Shown { entity, triangles, wanted: self.frame }
                }
                Ok(Made { mesh: None, .. }) => Known::Empty,
                Err(why) => {
                    error!("chunk {:?} could not be made: {why}", done.key);
                    Known::Empty
                }
            };
            self.known.insert(done.key, known);
        }
        let prev_shown = std::mem::take(&mut self.shown);
        let (shown, holes) = rings::shown(&wanted, &|k| self.known.contains_key(&k) && !fresh.contains(&k));
        self.shown = shown.into_iter().collect();

        let step = if FADE_SECONDS > 0.0 { dt / FADE_SECONDS } else { 1.0 };
        self.fading_in.retain(|k, t| {
            *t += step;
            *t < 1.0 && self.shown.contains(k)
        });
        self.fading_out.retain(|k, t| {
            *t += step;
            *t < 1.0 && !self.shown.contains(k)
        });
        if !prev_shown.is_empty() {
            let mut any_replaced = false;
            for &k in &prev_shown {
                if !self.shown.contains(&k) {
                    any_replaced = true;
                    if matches!(self.known.get(&k), Some(Known::Shown { .. })) {
                        self.fading_out.insert(k, 0.0);
                    }
                }
            }
            if any_replaced {
                for &k in &self.shown {
                    if !prev_shown.contains(&k) && matches!(self.known.get(&k), Some(Known::Shown { .. })) {
                        self.fading_in.insert(k, 0.0);
                    }
                }
            }
        }

        let later: HashSet<Key> = later.into_iter().collect();
        if self.known.len() > MOST_KEPT {
            let mut old: Vec<(u64, Key)> = self
                .known
                .iter()
                .filter(|(k, _)| !self.wanted.contains(k) && !self.shown.contains(k) && !self.fading_out.contains_key(k) && !later.contains(k))
                .map(|(k, known)| (if let Known::Shown { wanted, .. } = known { *wanted } else { 0 }, *k))
                .collect();
            old.sort_unstable();
            for (_, key) in old.into_iter().take(self.known.len() - MOST_KEPT / 2) {
                if let Some(Known::Shown { entity, .. }) = self.known.remove(&key) {
                    commands.entity(entity).despawn();
                }
            }
        }
        let c = self.counts;
        let mut counts = Counts { made: c.made, slowest: c.slowest, total: c.total, longest: c.longest, holes, ..Counts::default() };
        for key in &self.wanted {
            match self.known.get(key) {
                Some(Known::Shown { .. }) => counts.shown += 1,
                Some(Known::Empty) => counts.empty += 1,
                None => counts.waiting += 1,
            }
        }
        counts.ahead = later.iter().filter(|k| !self.known.contains_key(k)).count();
        for key in self.shown.iter().chain(&later) {
            if let Some(Known::Shown { triangles, wanted, .. }) = self.known.get_mut(key) {
                *wanted = self.frame;
                if self.shown.contains(key) {
                    counts.triangles += *triangles;
                }
            }
        }
        self.counts = counts;
    }

    pub fn clear_made(&mut self) {
        (self.counts.made, self.counts.slowest, self.counts.total, self.counts.longest) = (0, Duration::ZERO, Duration::ZERO, Duration::ZERO);
    }

    pub fn place<F: bevy::ecs::query::QueryFilter>(
        &self,
        eye: [f64; 3],
        scale: f64,
        transforms: &mut Query<(&mut Transform, &mut Visibility, Option<&mut MeshMaterial3d<SurfaceMaterial>>), F>,
    ) {
        for (key, known) in &self.known {
            let Known::Shown { entity, .. } = known else { continue };
            let Ok((mut transform, mut visibility, material)) = transforms.get_mut(*entity) else { continue };
            let in_t = self.fading_in.get(key).copied();
            let out_t = self.fading_out.get(key).copied();
            let show = self.shown.contains(key) || out_t.is_some();
            visibility.set_if_neq(if show { Visibility::Visible } else { Visibility::Hidden });
            if show {
                let o = key.origin();
                transform.translation = Vec3::new(((o[0] - eye[0]) * scale) as f32, ((o[1] - eye[1]) * scale) as f32, ((o[2] - eye[2]) * scale) as f32);
                transform.scale = Vec3::splat(scale as f32);
                if let Some(mut mat) = material {
                    let target = if let Some(t) = out_t {
                        self.materials.outgoing(t)
                    } else if let Some(t) = in_t {
                        self.materials.incoming(t)
                    } else {
                        &self.materials.opaque
                    };
                    if mat.0 != *target {
                        mat.0 = target.clone();
                    }
                }
            }
        }
    }
}

impl Drop for Chunks {
    fn drop(&mut self) {
        self.queue.jobs.lock().unwrap().stop = true;
        self.queue.more.notify_all();
    }
}

fn work(queue: &Queue, tell: &Sender<Done>, compiled: &Compiled, seed: u64) {
    let mut area = None;
    loop {
        let key = {
            let mut jobs = queue.jobs.lock().unwrap();
            loop {
                if jobs.stop {
                    return;
                }
                if let Some(key) = jobs.todo.pop() {
                    jobs.making.insert(key);
                    break key;
                }
                jobs = queue.more.wait(jobs).unwrap();
            }
        };
        let started = Instant::now();
        let made = match area.as_mut() {
            Some(a) => Ok(a),
            None => compiled.instantiate(seed).map(|a| area.insert(a)),
        }
        .and_then(|a| chunk::make(a, key));
        if made.is_err() {
            area = None;
        }
        if tell.send(Done { key, made: made.map_err(|e| format!("{e:#}")), took: started.elapsed() }).is_err() {
            return;
        }
    }
}

fn to_bevy(mesh: chunk::Mesh) -> Mesh {
    let colours: Vec<[f32; 4]> = mesh
        .materials
        .iter()
        .zip(&mesh.open)
        .map(|(&m, &open)| {
            let [r, g, b] = colour(m);
            [r * open, g * open, b * open, 1.0]
        })
        .collect();
    let uvs: Vec<[f32; 2]> = mesh.materials.iter().map(|&m| properties(m)).collect();
    let mut out = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    out.insert_attribute(Mesh::ATTRIBUTE_POSITION, mesh.positions);
    out.insert_attribute(Mesh::ATTRIBUTE_NORMAL, mesh.normals);
    out.insert_attribute(Mesh::ATTRIBUTE_COLOR, colours);
    out.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    out.insert_indices(Indices::U32(mesh.indices));
    out
}
