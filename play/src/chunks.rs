//! The area's surface about the player, made off the frame (POC_PLAN.md, steps 2, 7, and 8):
//! view-directed `6×4×6` rings shifted forward along the look direction, frustum-prioritized chunk
//! streaming, complementary 4×4 Bayer dithered LOD cross-fades, and live targeted chunk
//! invalidation for scale-stamped SDF edits & placed SDF `.wasm` elements with zero-flicker
//! in-place mesh swaps.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use plateau_engine::atlas::{Atlas, CompiledAtlas};
use plateau_engine::chunk::{self, Key, Made};
use plateau_engine::compose::{Composed, ComposedLog};
use plateau_engine::live::Invalidation;
use plateau_engine::picture::colour;
use plateau_engine::{Wasm, rings};

use crate::surface::{FADE_SECONDS, Materials, SurfaceMaterial, properties};

const MOST_KEPT: usize = 12_000;
const MOST_A_FRAME: usize = 64;

enum Known {
    Empty { revision: u64 },
    Shown { entity: Entity, triangles: usize, wanted: u64, revision: u64 },
}

impl Known {
    fn revision(&self) -> u64 {
        match *self {
            Known::Empty { revision } | Known::Shown { revision, .. } => revision,
        }
    }
}

struct Jobs {
    todo: Vec<Key>,
    making: HashMap<Key, u64>,
    composed: Arc<ComposedLog>,
    stop: bool,
}

struct Queue {
    jobs: Mutex<Jobs>,
    more: Condvar,
}

struct Done {
    key: Key,
    revision: u64,
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
    /// Cached chunks invalidated by SDF edits/elements that need rebuilding to at least `min_revision`.
    /// Their old mesh stays visible until the rebuilt mesh arrives, preventing flicker.
    stale: HashMap<Key, u64>,
    /// Completed rebuilds for invalidated chunks held until every visible chunk of the same
    /// edit/undo is ready, so `Shown -> Empty` despawns and `Empty -> Shown` spawns commit
    /// atomically in the same frame (preventing 1-frame rectangular sky holes).
    pending_stale: HashMap<Key, Done>,
    stale_wait_frames: u32,
    composed_rev: u64,
    /// Newly promoted solid Scene layers held back from hiding (`skip_solid: true`) until the
    /// rebuilt SDF chunks carrying their `MeshSdf` are applied in the same frame.
    deferred_hide: HashMap<u32, (u64, plateau_engine::Scene)>,
    tiles: Vec<[i32; 2]>,
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
    pub fn new(atlas: Arc<CompiledAtlas>, composed: Arc<ComposedLog>, materials: Materials) -> Chunks {
        let queue = Arc::new(Queue {
            jobs: Mutex::new(Jobs { todo: Vec::new(), making: HashMap::new(), composed, stop: false }),
            more: Condvar::new(),
        });
        let (tell, done) = channel();
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).saturating_sub(2).max(1);
        for k in 0..threads {
            let (queue, tell, atlas) = (queue.clone(), tell.clone(), atlas.clone());
            std::thread::Builder::new().name(format!("chunks {k}")).spawn(move || work(&queue, &tell, &atlas)).expect("a thread to make chunks");
        }
        let tiles = atlas.tile_coords();
        Chunks {
            queue,
            done: Mutex::new(done),
            known: HashMap::new(),
            stale: HashMap::new(),
            pending_stale: HashMap::new(),
            stale_wait_frames: 0,
            composed_rev: 0,
            deferred_hide: HashMap::new(),
            rings: rings::Rings::with_tiles(tiles.clone()),
            tiles,
            wanted: HashSet::new(),
            shown: HashSet::new(),
            fading_in: HashMap::new(),
            fading_out: HashMap::new(),
            frame: 0,
            materials,
            counts: Counts::default(),
        }
    }

    /// Publishes an updated `ComposedLog` snapshot to worker threads and invalidates affected chunks
    /// (both cached in `known` and currently in-flight in `jobs.making` so stale builds never
    /// overwrite or leave permanent holes).
    pub fn set_composed(&mut self, composed: Arc<ComposedLog>, inv: &Invalidation) {
        let rev = composed.revision();
        self.composed_rev = rev;
        let in_flight: Vec<Key> = {
            let mut jobs = self.queue.jobs.lock().unwrap();
            jobs.composed = composed;
            jobs.making
                .iter()
                .filter(|&(&k, &r)| r < rev && inv.affects(k))
                .map(|(&k, _)| k)
                .collect()
        };
        for (&key, known) in &self.known {
            if known.revision() < rev && inv.affects(key) {
                self.stale.insert(key, rev);
            }
        }
        for key in in_flight {
            self.stale.insert(key, rev);
        }
        self.pending_stale.retain(|&key, done| {
            if done.revision < rev && inv.affects(key) {
                self.stale.insert(key, rev);
                false
            } else {
                true
            }
        });
        self.stale_wait_frames = 0;
        self.queue.more.notify_all();
    }

    pub fn composed_revision(&self) -> u64 {
        self.composed_rev
    }

    /// Whether any visible chunk still awaits an SDF rebuild up to `rev`.
    pub fn has_wanted_stale_for(&self, rev: u64) -> bool {
        self.wanted
            .iter()
            .any(|k| self.stale.get(k).is_some_and(|&r| r <= rev) || self.pending_stale.contains_key(k))
    }

    /// Holds a `skip_solid: true` layer replacement until the SDF chunks for `rev` have been
    /// applied, so promoted wood/stone never disappears while background chunk workers mesh it.
    pub fn defer_hide_solid(&mut self, owner: u32, rev: u64, scene: plateau_engine::Scene) {
        self.deferred_hide.insert(owner, (rev, scene));
    }

    pub fn cancel_deferred_hide(&mut self, owner: u32) {
        self.deferred_hide.remove(&owner);
    }

    pub fn clear_deferred_hides(&mut self) {
        self.deferred_hide.clear();
    }

    /// Drains deferred `skip_solid: true` layers whose SDF chunks are now committed and visible.
    pub fn take_ready_deferred_hides(&mut self) -> Vec<(u32, plateau_engine::Scene)> {
        let ready: Vec<u32> = self
            .deferred_hide
            .iter()
            .filter(|&(_, &(rev, _))| !self.has_wanted_stale_for(rev))
            .map(|(&owner, _)| owner)
            .collect();
        ready
            .into_iter()
            .filter_map(|owner| self.deferred_hide.remove(&owner).map(|(_, sc)| (owner, sc)))
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        eye: [f64; 3],
        look: [f64; 3],
        size: f32,
        scale: f64,
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
            let needs = |k: &Key| -> bool {
                if let Some(&need_rev) = self.stale.get(k) {
                    making.get(k).is_none_or(|&r| r < need_rev)
                } else if self.pending_stale.contains_key(k) {
                    false
                } else {
                    !self.known.contains_key(k) && !making.contains_key(k)
                }
            };
            // Prioritize invalidated wanted chunks first so live sculpting & element placement respond immediately.
            let mut todo: Vec<Key> = Vec::new();
            for k in &wanted {
                if self.stale.contains_key(k) && needs(k) && seen.insert(*k) {
                    todo.push(*k);
                }
            }
            for k in wanted.iter().chain(&later) {
                if needs(k) && seen.insert(*k) {
                    todo.push(*k);
                }
            }
            jobs.todo = todo.into_iter().rev().collect();
            if !jobs.todo.is_empty() {
                self.queue.more.notify_all();
            }
        }
        let mut fresh = HashSet::new();
        let mut spawned_this_frame: Vec<(Key, Entity)> = Vec::new();
        let made: Vec<Done> = self.done.lock().unwrap().try_iter().take(MOST_A_FRAME).collect();
        let mut ready_to_apply: Vec<Done> = Vec::with_capacity(made.len());
        for done in made {
            {
                let mut jobs = self.queue.jobs.lock().unwrap();
                if jobs.making.get(&done.key).is_some_and(|&r| r <= done.revision) {
                    jobs.making.remove(&done.key);
                }
            }
            self.counts.made += 1;
            self.counts.slowest = self.counts.slowest.max(done.took);
            self.counts.total += done.took;

            if self.known.get(&done.key).is_some_and(|k| k.revision() > done.revision) {
                continue;
            }
            let was_stale = if let Some(&need) = self.stale.get(&done.key) {
                if done.revision >= need {
                    self.stale.remove(&done.key);
                    true
                } else if self.known.contains_key(&done.key) {
                    // Discard superseded in-flight build rather than flashing a stale empty/old mesh.
                    continue;
                } else {
                    false
                }
            } else {
                false
            };

            if was_stale && self.known.contains_key(&done.key) {
                self.pending_stale.insert(done.key, done);
            } else {
                ready_to_apply.push(done);
            }
        }

        // Commit all buffered edit/undo rebuilds atomically once every visible stale chunk is done
        // (or after a short frame cap for huge reloads) so `Shown -> Empty` never despawns before
        // its neighbor's `Empty -> Shown` mesh is ready!
        if !self.pending_stale.is_empty() {
            let any_wanted_stale = self.wanted.iter().any(|k| self.stale.contains_key(k));
            if !any_wanted_stale || self.stale_wait_frames >= 8 {
                ready_to_apply.extend(self.pending_stale.drain().map(|(_, d)| d));
                self.stale_wait_frames = 0;
            } else {
                self.stale_wait_frames += 1;
            }
        }

        for done in ready_to_apply {
            let was_known = self.known.contains_key(&done.key);
            // When updating an already-shown chunk with a new mesh, swap `Mesh3d` in place on the
            // existing entity instead of despawning + respawning `Visibility::Hidden`.
            match done.made {
                Ok(Made { mesh: Some(mesh), .. }) => {
                    let triangles = mesh.indices.len() / 3;
                    let bevy_mesh = meshes.add(to_bevy(mesh));
                    if let Some(Known::Shown { entity, triangles: cur_tris, wanted, revision }) = self.known.get_mut(&done.key) {
                        commands.entity(*entity).insert(Mesh3d(bevy_mesh));
                        *cur_tris = triangles;
                        *wanted = self.frame;
                        *revision = done.revision;
                    } else {
                        let o = done.key.origin();
                        let tf = Transform {
                            translation: Vec3::new(
                                ((o[0] - eye[0]) * scale) as f32,
                                ((o[1] - eye[1]) * scale) as f32,
                                ((o[2] - eye[2]) * scale) as f32,
                            ),
                            scale: Vec3::splat(scale as f32),
                            ..default()
                        };
                        let entity = commands
                            .spawn((
                                Mesh3d(bevy_mesh),
                                MeshMaterial3d(self.materials.opaque.clone()),
                                tf,
                                Visibility::Hidden,
                            ))
                            .id();
                        if !was_known {
                            fresh.insert(done.key);
                        } else {
                            // Previously `Known::Empty` becoming `Known::Shown` during an atomic
                            // edit/undo swap: make it visible on THIS exact frame once `shown_over`
                            // confirms it is shown!
                            spawned_this_frame.push((done.key, entity));
                        }
                        self.known.insert(done.key, Known::Shown { entity, triangles, wanted: self.frame, revision: done.revision });
                    }
                }
                Ok(Made { mesh: None, .. }) => {
                    if let Some(Known::Shown { entity: old_entity, .. }) = self.known.insert(done.key, Known::Empty { revision: done.revision }) {
                        commands.entity(old_entity).despawn();
                    }
                }
                Err(why) => {
                    error!("chunk {:?} could not be made: {why}", done.key);
                    if let Some(Known::Shown { entity: old_entity, .. }) = self.known.insert(done.key, Known::Empty { revision: done.revision }) {
                        commands.entity(old_entity).despawn();
                    }
                }
            }
            if was_known {
                self.fading_in.remove(&done.key);
                self.fading_out.remove(&done.key);
            }
        }
        let prev_shown = std::mem::take(&mut self.shown);
        let (shown, holes) = rings::shown_over(&wanted, &self.tiles, &|k| self.known.contains_key(&k) && !fresh.contains(&k));
        self.shown = shown.into_iter().collect();
        for (key, entity) in spawned_this_frame {
            if self.shown.contains(&key) {
                commands.entity(entity).insert(Visibility::Visible);
            }
        }

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
                self.stale.remove(&key);
                self.pending_stale.remove(&key);
                if let Some(Known::Shown { entity, .. }) = self.known.remove(&key) {
                    commands.entity(entity).despawn();
                }
            }
        }
        let c = self.counts;
        let mut counts = Counts { made: c.made, slowest: c.slowest, total: c.total, longest: c.longest, holes, ..Counts::default() };
        for key in &self.wanted {
            match self.known.get(key) {
                Some(Known::Shown { .. }) if !self.stale.contains_key(key) => counts.shown += 1,
                Some(Known::Empty { .. }) if !self.stale.contains_key(key) => counts.empty += 1,
                _ => counts.waiting += 1,
            }
        }
        counts.ahead = later.iter().filter(|k| !self.known.contains_key(k) || self.stale.contains_key(k)).count();
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

fn work(queue: &Queue, tell: &Sender<Done>, atlas: &CompiledAtlas) {
    let mut composed: Option<Composed<Atlas<Wasm>, Arc<ComposedLog>>> = None;
    loop {
        let (key, log) = {
            let mut jobs = queue.jobs.lock().unwrap();
            loop {
                if jobs.stop {
                    return;
                }
                if let Some(key) = jobs.todo.pop() {
                    let log = jobs.composed.clone();
                    jobs.making.insert(key, log.revision());
                    break (key, log);
                }
                jobs = queue.more.wait(jobs).unwrap();
            }
        };
        let revision = log.revision();
        let started = Instant::now();
        let made = match composed.as_mut() {
            Some(c) => {
                if c.log.revision() != revision {
                    c.log = log;
                    c.prune_instances();
                }
                Ok(c)
            }
            None => atlas.instantiate().map(|a| composed.insert(Composed::new(a, log))),
        }
        .and_then(|c| chunk::make(c, key));
        if made.is_err() {
            composed = None;
        }
        if tell.send(Done { key, revision, made: made.map_err(|e| format!("{e:#}")), took: started.elapsed() }).is_err() {
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
