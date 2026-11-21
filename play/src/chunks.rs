//! The area's surface about the player, made off the frame (POC_PLAN.md, step 2): the chunks the
//! player sees (`rings::wanted`), made by a thread for each core but two, each with its own instance
//! of the area's module, finest and nearest first; shown as they come back, drawn relative to the
//! player's eye and scaled to their size (`Chunks::place`).
//!
//! Chunks no longer wanted stay shown until every wanted chunk is made, so moving or changing size
//! never opens a hole; then they are hidden, and kept a while in case they are wanted again.

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

/// The most chunks kept, shown or not; beyond it, those wanted longest ago are let go.
const MOST_KEPT: usize = 6000;
/// The most chunks brought to the screen in a frame, so a frame is never held up by many at once.
const MOST_A_FRAME: usize = 48;

/// What is known of a chunk.
enum Known {
    /// Made, with nothing in it.
    Empty,
    Shown { entity: Entity, triangles: usize, wanted: u64 },
}

struct Jobs {
    /// Chunks to make, the most wanted last.
    todo: Vec<Key>,
    /// Being made.
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

/// How making chunks is going, for the HUD and the tour.
#[derive(Clone, Copy, Debug, Default)]
pub struct Counts {
    /// Chunks wanted and shown, wanted and empty, wanted and not yet made.
    pub shown: usize,
    pub empty: usize,
    pub waiting: usize,
    pub triangles: usize,
    /// Of the chunks made since the counts were last cleared: how many, the slowest, all together.
    pub made: usize,
    pub slowest: Duration,
    pub total: Duration,
}

#[derive(Resource)]
pub struct Chunks {
    queue: Arc<Queue>,
    done: Mutex<Receiver<Done>>,
    known: HashMap<Key, Known>,
    wanted: HashSet<Key>,
    frame: u64,
    material: Handle<StandardMaterial>,
    pub counts: Counts,
}

impl Chunks {
    pub fn new(compiled: Arc<Compiled>, seed: u64, material: Handle<StandardMaterial>) -> Chunks {
        let queue = Arc::new(Queue { jobs: Mutex::new(Jobs { todo: Vec::new(), making: HashSet::new(), stop: false }), more: Condvar::new() });
        let (tell, done) = channel();
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).saturating_sub(2).max(1);
        for k in 0..threads {
            let (queue, tell, compiled) = (queue.clone(), tell.clone(), compiled.clone());
            std::thread::Builder::new().name(format!("chunks {k}")).spawn(move || work(&queue, &tell, &compiled, seed)).expect("a thread to make chunks");
        }
        Chunks { queue, done: Mutex::new(done), known: HashMap::new(), wanted: HashSet::new(), frame: 0, material, counts: Counts::default() }
    }

    /// Asks for the chunks a viewer at `eye`, `size` tall, sees, and brings to the screen those
    /// made since the last frame.
    pub fn update(&mut self, eye: [f64; 3], size: f32, commands: &mut Commands, meshes: &mut Assets<Mesh>) {
        self.frame += 1;
        let mut wanted = rings::wanted(eye, size);
        // Finest first, and on each level the nearest first.
        let far = |k: &Key| {
            let m = k.middle();
            ((m[0] - eye[0]).powi(2) + (m[1] - eye[1]).powi(2) + (m[2] - eye[2]).powi(2)).sqrt() / k.side()
        };
        wanted.sort_by(|a, b| a.level.cmp(&b.level).then(far(a).total_cmp(&far(b))));
        self.wanted = wanted.iter().copied().collect();
        {
            let mut jobs = self.queue.jobs.lock().unwrap();
            let making = &jobs.making;
            let todo: Vec<Key> = wanted.iter().rev().filter(|k| !self.known.contains_key(k) && !making.contains(k)).copied().collect();
            jobs.todo = todo;
            if !jobs.todo.is_empty() {
                self.queue.more.notify_all();
            }
        }
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
                        .spawn((Mesh3d(meshes.add(to_bevy(mesh))), MeshMaterial3d(self.material.clone()), Transform::default(), Visibility::Hidden))
                        .id();
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
        // Let go of the chunks wanted longest ago, if too many are kept.
        if self.known.len() > MOST_KEPT {
            let mut old: Vec<(u64, Key)> = self
                .known
                .iter()
                .filter(|(k, _)| !self.wanted.contains(k))
                .map(|(k, known)| (if let Known::Shown { wanted, .. } = known { *wanted } else { 0 }, *k))
                .collect();
            old.sort_unstable();
            for (_, key) in old.into_iter().take(self.known.len() - MOST_KEPT / 2) {
                if let Some(Known::Shown { entity, .. }) = self.known.remove(&key) {
                    commands.entity(entity).despawn();
                }
            }
        }
        let mut counts = Counts { made: self.counts.made, slowest: self.counts.slowest, total: self.counts.total, ..Counts::default() };
        for key in &self.wanted {
            match self.known.get_mut(key) {
                Some(Known::Shown { triangles, wanted, .. }) => {
                    counts.shown += 1;
                    counts.triangles += *triangles;
                    *wanted = self.frame;
                }
                Some(Known::Empty) => counts.empty += 1,
                None => counts.waiting += 1,
            }
        }
        self.counts = counts;
    }

    /// Clears what is counted of the chunks made.
    pub fn clear_made(&mut self) {
        (self.counts.made, self.counts.slowest, self.counts.total) = (0, Duration::ZERO, Duration::ZERO);
    }

    /// Every chunk shown drawn where it is from the eye, `scale` render units to a metre, and
    /// those not wanted hidden once nothing wanted is still being made.
    pub fn place<F: bevy::ecs::query::QueryFilter>(&self, eye: [f64; 3], scale: f64, transforms: &mut Query<(&mut Transform, &mut Visibility), F>) {
        let settled = self.counts.waiting == 0;
        for (key, known) in &self.known {
            let Known::Shown { entity, .. } = known else { continue };
            let Ok((mut transform, mut visibility)) = transforms.get_mut(*entity) else { continue };
            let show = self.wanted.contains(key) || !settled;
            *visibility = if show { Visibility::Visible } else { Visibility::Hidden };
            if show {
                let o = key.origin();
                transform.translation = Vec3::new(((o[0] - eye[0]) * scale) as f32, ((o[1] - eye[1]) * scale) as f32, ((o[2] - eye[2]) * scale) as f32);
                transform.scale = Vec3::splat(scale as f32);
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

/// A thread making chunks, the most wanted first, with its own instance of the area.
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
        // After an error the instance is not to be asked again: a new one for the next chunk.
        if made.is_err() {
            area = None;
        }
        if tell.send(Done { key, made: made.map_err(|e| format!("{e:#}")), took: started.elapsed() }).is_err() {
            return;
        }
    }
}

/// A chunk's surface as Bevy draws it: each vertex coloured by its material, darkened in hollows.
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
    let mut out = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    out.insert_attribute(Mesh::ATTRIBUTE_POSITION, mesh.positions);
    out.insert_attribute(Mesh::ATTRIBUTE_NORMAL, mesh.normals);
    out.insert_attribute(Mesh::ATTRIBUTE_COLOR, colours);
    out.insert_indices(Indices::U32(mesh.indices));
    out
}
