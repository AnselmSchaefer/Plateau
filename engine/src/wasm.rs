//! An area's module in wasmtime, with the limits the engine keeps (POC_PLAN.md, "Running areas"):
//! fuel for each batch, a memory limit, no SIMD and no threads, NaNs left as the machine makes them
//! (the law `laws::nans_unseen` stands in for making them one). The engine writes the points and
//! reads the answers and the description as the canonical ABI lays out `plateau:region@0.1.0`'s
//! types, checking every pointer, length and number the area gives back.

use wasmtime::error::Context;
use wasmtime::{Caller, Config, Engine, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc, format_err};

use crate::check::{self, HOST, MOST_MEMORY};
use crate::{Answer, Borders, Command, Description, Edge, Instance, LARGEST, Light, Material, MeshData, Place, PlayerState, Region, Result, SMALLEST, Scene, Size, Vertex};

/// How a module is run.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Fuel for each point of a batch (a batch is given this times its points, and `FUEL_PER_CALL`),
    /// or none counted.
    pub fuel_per_sample: Option<u64>,
    pub memory: usize,
    /// Every NaN made the one NaN, so no machine's own NaNs can be seen. Off in play: it makes an
    /// area over three times slower (spike/RESULTS.md). The law `nans_unseen` stands in for it.
    pub canonical_nans: bool,
}

/// Fuel for a batch besides its points', for `describe` / `scene`, and per runtime `tick` / `interact`.
pub const FUEL_PER_CALL: u64 = 1_000_000;
pub const FUEL_TO_DESCRIBE: u64 = 100_000_000;
pub const FUEL_PER_TICK: u64 = 2_000_000;
/// Security caps for procedural scenes & runtime commands emitted by an untrusted `.wasm`.
pub const MOST_MESHES: usize = 256;
pub const MOST_VERTICES: usize = 200_000;
pub const MOST_INDICES: usize = 600_000;
pub const MOST_INSTANCES: usize = 16_384;
pub const MOST_LIGHTS: usize = 64;
pub const MOST_COMMANDS: usize = 256;
/// Maximum clamped magnitude for an `ApplyImpulse` verb (m/s).
pub const MAX_IMPULSE: f32 = 25.0;
/// The most points in a batch: what a chunk's corner asks at once is well within it.
pub const MOST_POINTS: usize = 1 << 16;
/// The most places an area may name, and the longest name.
pub const MOST_PLACES: usize = 64;
pub const LONGEST_NAME: usize = 200;
/// How many heights an edge has: one a metre, both corners included.
pub const EDGE_HEIGHTS: usize = 65;

impl Default for Limits {
    fn default() -> Self {
        // The walled garden takes about 10 000 a point for the tiny (spike/RESULTS.md): room for
        // twenty times that, until the law "within its budget" (step 3) sets it.
        Limits { fuel_per_sample: Some(200_000), memory: MOST_MEMORY, canonical_nans: false }
    }
}

/// An area's module checked and compiled, ready to be instantiated: once for each thread that
/// samples it.
pub struct Compiled {
    engine: Engine,
    module: Module,
    limits: Limits,
}

impl Compiled {
    /// Checks `bytes` (`check::check`, refused with its reasons) and compiles them.
    pub fn new(bytes: &[u8], limits: Limits) -> Result<Compiled> {
        if let Err(why) = check::check(bytes) {
            return Err(format_err!("the area's module is refused: {}", why.join("; ")));
        }
        let mut config = Config::new();
        config.consume_fuel(limits.fuel_per_sample.is_some());
        // The same results on every machine: nothing whose results differ from one processor to the
        // next. Threads are refused by building wasmtime without them. NaNs are left as the
        // machine makes them, unless asked: `nans_unseen` checks that the area never looks.
        config.cranelift_nan_canonicalization(limits.canonical_nans);
        config.wasm_relaxed_simd(false);
        config.wasm_simd(false);
        let engine = Engine::new(&config)?;
        let module = Module::new(&engine, bytes)?;
        Ok(Compiled { engine, module, limits })
    }

    /// The area made, with its seed: `describe` and `sample` may be called from now.
    pub fn instantiate(&self, seed: u64) -> Result<Wasm> {
        let ctx = Ctx { seed, limits: StoreLimitsBuilder::new().memory_size(self.limits.memory).instances(1).build() };
        let mut store = Store::new(&self.engine, ctx);
        store.limiter(|ctx| &mut ctx.limits);
        if self.limits.fuel_per_sample.is_some() {
            store.set_fuel(FUEL_PER_CALL)?;
        }
        let mut linker = Linker::new(&self.engine);
        linker.func_wrap(HOST, "seed", |caller: Caller<'_, Ctx>| caller.data().seed as i64)?;
        let instance = linker.instantiate(&mut store, &self.module)?;
        let memory = instance.get_memory(&mut store, "memory").ok_or_else(|| format_err!("the area has no memory"))?;
        let scene = match (instance.get_typed_func(&mut store, "scene"), instance.get_typed_func(&mut store, "cabi_post_scene")) {
            (Ok(f), Ok(p)) => Some((f, p)),
            _ => None,
        };
        let tick = match (instance.get_typed_func(&mut store, "tick"), instance.get_typed_func(&mut store, "cabi_post_tick")) {
            (Ok(f), Ok(p)) => Some((f, p)),
            _ => None,
        };
        let interact = match (instance.get_typed_func(&mut store, "interact"), instance.get_typed_func(&mut store, "cabi_post_interact")) {
            (Ok(f), Ok(p)) => Some((f, p)),
            _ => None,
        };
        Ok(Wasm {
            realloc: instance.get_typed_func(&mut store, "cabi_realloc").context("the area's cabi_realloc")?,
            sample: instance.get_typed_func(&mut store, "sample").context("the area's sample")?,
            post_sample: instance.get_typed_func(&mut store, "cabi_post_sample").context("the area's cabi_post_sample")?,
            describe: instance.get_typed_func(&mut store, "describe").context("the area's describe")?,
            post_describe: instance.get_typed_func(&mut store, "cabi_post_describe").context("the area's cabi_post_describe")?,
            scene,
            tick,
            interact,
            store,
            memory,
            fuel_per_sample: self.limits.fuel_per_sample,
        })
    }
}

struct Ctx {
    seed: u64,
    limits: StoreLimits,
}

/// An area's module, made and running. After an error (a trap, fuel run out, an answer the engine
/// cannot read) it is not to be asked again: its memory may be anything.
pub struct Wasm {
    store: Store<Ctx>,
    memory: Memory,
    realloc: TypedFunc<(u32, u32, u32, u32), u32>,
    sample: TypedFunc<(u32, u32, f32), u32>,
    post_sample: TypedFunc<u32, ()>,
    describe: TypedFunc<(), u32>,
    post_describe: TypedFunc<u32, ()>,
    scene: Option<(TypedFunc<(), u32>, TypedFunc<u32, ()>)>,
    tick: Option<(TypedFunc<u32, u32>, TypedFunc<u32, ()>)>,
    interact: Option<(TypedFunc<u32, u32>, TypedFunc<u32, ()>)>,
    fuel_per_sample: Option<u64>,
}

impl Wasm {
    /// Checks, compiles and makes a module in one: for when one instance is enough.
    pub fn new(bytes: &[u8], seed: u64, limits: Limits) -> Result<Wasm> {
        Compiled::new(bytes, limits)?.instantiate(seed)
    }

    /// The fuel left after the last call, if counted.
    pub fn fuel_left(&self) -> Option<u64> {
        self.fuel_per_sample.and_then(|_| self.store.get_fuel().ok())
    }

    fn fuel(&mut self, fuel: impl FnOnce(u64) -> u64) -> Result<()> {
        if let Some(per) = self.fuel_per_sample {
            self.store.set_fuel(fuel(per))?;
        }
        Ok(())
    }
}

impl Region for Wasm {
    fn sample(&mut self, points: &[[f32; 3]], size: f32, out: &mut Vec<Answer>) -> Result<()> {
        let count = points.len();
        if count > MOST_POINTS {
            return Err(format_err!("{count} points asked at once, more than the {MOST_POINTS} a batch may have"));
        }
        if !(SMALLEST..=LARGEST).contains(&size) {
            return Err(format_err!("an area is asked for viewers {SMALLEST} to {LARGEST} m tall, not {size}"));
        }
        self.fuel(|per| FUEL_PER_CALL + per * count as u64)?;
        // `list<point>`: 12 bytes a point, aligned to 4, in room the area gives; the area owns it
        // from the call on.
        let at = self.realloc.call(&mut self.store, (0, 0, 4, (count * 12) as u32))?;
        let data = self.memory.data_mut(&mut self.store);
        let room = data.get_mut(at as usize..at as usize + count * 12).ok_or_else(|| format_err!("the area gave room outside its memory"))?;
        for (p, bytes) in points.iter().zip(room.as_chunks_mut::<12>().0) {
            for (c, b) in p.iter().zip(bytes.as_chunks_mut::<4>().0) {
                *b = c.to_le_bytes();
            }
        }
        let ret = self.sample.call(&mut self.store, (at, count as u32, size))?;
        // `list<answer>`: a pointer and a length at `ret`; 8 bytes an answer, its distance and its
        // material's number.
        let data = Bytes(self.memory.data(&self.store));
        let (list, len) = (data.u32(ret)?, data.u32(ret + 4)? as usize);
        if len != count {
            return Err(format_err!("the area gave {len} answers for {count} points"));
        }
        let answers = data.slice(list, len, 8)?;
        out.clear();
        out.reserve(len);
        for b in answers.as_chunks::<8>().0 {
            let distance = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            let material = Material::from_number(b[4]).ok_or_else(|| format_err!("the area answered material {}, which is not one of the engine's", b[4]))?;
            out.push(Answer { distance, material });
        }
        self.post_sample.call(&mut self.store, ret)?;
        Ok(())
    }

    fn describe(&mut self) -> Result<Description> {
        self.fuel(|_| FUEL_TO_DESCRIBE)?;
        let ret = self.describe.call(&mut self.store, ())?;
        let d = Bytes(self.memory.data(&self.store));
        // `description`: its name (8 bytes), places (8), borders (four edges of 16).
        let name = d.name(ret)?;
        let (list, len) = (d.u32(ret + 8)?, d.u32(ret + 12)? as usize);
        if len > MOST_PLACES {
            return Err(format_err!("the area names {len} places, more than the {MOST_PLACES} it may"));
        }
        let places = d.slice(list, len, 24)?;
        let mut read = Vec::with_capacity(len);
        for k in 0..len as u32 {
            // `place`: its name (8 bytes), its size (1, then 3 unused), where it is (12).
            let at = list + k * 24;
            let size = match places[k as usize * 24 + 8] {
                0 => Size::Tiny,
                1 => Size::Person,
                2 => Size::Giant,
                n => return Err(format_err!("the area's place is for size {n}, which is not one of the engine's")),
            };
            read.push(Place { name: d.name(at)?, size, at: [d.f32(at + 12)?, d.f32(at + 16)?, d.f32(at + 20)?] });
        }
        let edge = |at: u32| -> Result<Edge> {
            // `edge`: its heights (8 bytes), then whether there is water (1, 3 unused) and its level (4).
            let (list, len) = (d.u32(at)?, d.u32(at + 4)? as usize);
            if len != EDGE_HEIGHTS {
                return Err(format_err!("the area's edge has {len} heights, not {EDGE_HEIGHTS}"));
            }
            let ground = d.slice(list, len, 4)?.as_chunks::<4>().0.iter().map(|&b| f32::from_le_bytes(b)).collect();
            let water = match d.slice(at + 8, 1, 1)?[0] {
                0 => None,
                1 => Some(d.f32(at + 12)?),
                n => return Err(format_err!("the area's edge says {n} for whether there is water")),
            };
            Ok(Edge { ground, water })
        };
        let borders = Borders { north: edge(ret + 16)?, east: edge(ret + 32)?, south: edge(ret + 48)?, west: edge(ret + 64)? };
        let description = Description { name, places: read, borders };
        self.post_describe.call(&mut self.store, ret)?;
        Ok(description)
    }

    fn scene(&mut self) -> Result<Scene> {
        let Some((call_scene, post_scene)) = self.scene.clone() else {
            return Ok(Scene::default());
        };
        self.fuel(|_| FUEL_TO_DESCRIBE)?;
        let ret = call_scene.call(&mut self.store, ())?;
        let d = Bytes(self.memory.data(&self.store));
        let (meshes_ptr, meshes_len) = (d.u32(ret)?, d.u32(ret + 4)? as usize);
        let (inst_ptr, inst_len) = (d.u32(ret + 8)?, d.u32(ret + 12)? as usize);
        let (lights_ptr, lights_len) = (d.u32(ret + 16)?, d.u32(ret + 20)? as usize);
        if meshes_len > MOST_MESHES {
            return Err(format_err!("the area gave {meshes_len} meshes, more than the {MOST_MESHES} it may"));
        }
        if inst_len > MOST_INSTANCES {
            return Err(format_err!("the area gave {inst_len} instances, more than the {MOST_INSTANCES} it may"));
        }
        if lights_len > MOST_LIGHTS {
            return Err(format_err!("the area gave {lights_len} lights, more than the {MOST_LIGHTS} it may"));
        }

        let mut total_verts = 0usize;
        let mut total_indices = 0usize;
        let mut meshes = Vec::with_capacity(meshes_len);
        for m in 0..meshes_len as u32 {
            let base = meshes_ptr + m * 16;
            let (vptr, vlen) = (d.u32(base)?, d.u32(base + 4)? as usize);
            let (iptr, ilen) = (d.u32(base + 8)?, d.u32(base + 12)? as usize);
            total_verts = total_verts.saturating_add(vlen);
            total_indices = total_indices.saturating_add(ilen);
            if total_verts > MOST_VERTICES {
                return Err(format_err!("the area's meshes exceed {MOST_VERTICES} total vertices"));
            }
            if total_indices > MOST_INDICES || !ilen.is_multiple_of(3) {
                return Err(format_err!("the area's mesh indices are invalid or exceed {MOST_INDICES}"));
            }
            let raw_v = d.slice(vptr, vlen, 28)?;
            let mut vertices = Vec::with_capacity(vlen);
            for b in raw_v.as_chunks::<28>().0 {
                let f = |o: usize| f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
                let pos = [f(0), f(4), f(8)];
                let normal = [f(12), f(16), f(20)];
                let material = Material::from_number(b[24]).ok_or_else(|| format_err!("vertex material {} is invalid", b[24]))?;
                if !pos.iter().chain(&normal).all(|v| v.is_finite() && v.abs() <= 256.0) {
                    return Err(format_err!("a mesh vertex has a non-finite or out-of-range coordinate"));
                }
                vertices.push(Vertex { pos, normal, material, tint: [b[25], b[26], b[27]] });
            }
            let raw_i = d.slice(iptr, ilen, 4)?;
            let mut indices = Vec::with_capacity(ilen);
            for b in raw_i.as_chunks::<4>().0 {
                let idx = u32::from_le_bytes(*b);
                if idx as usize >= vlen {
                    return Err(format_err!("mesh {m} has out-of-bounds vertex index {idx} (vertices: {vlen})"));
                }
                indices.push(idx);
            }
            meshes.push(MeshData { vertices, indices });
        }

        let raw_inst = d.slice(inst_ptr, inst_len, 52)?;
        let mut instances = Vec::with_capacity(inst_len);
        for b in raw_inst.as_chunks::<52>().0 {
            let inst = read_instance(b, meshes_len)?;
            instances.push(inst);
        }

        let raw_lights = d.slice(lights_ptr, lights_len, 36)?;
        let mut lights = Vec::with_capacity(lights_len);
        for b in raw_lights.as_chunks::<36>().0 {
            lights.push(read_light(b)?);
        }

        post_scene.call(&mut self.store, ret)?;
        Ok(Scene { meshes, instances, lights })
    }

    fn tick(&mut self, dt: f32, player: &PlayerState) -> Result<Vec<Command>> {
        let Some((call_tick, post_tick)) = self.tick.clone() else {
            return Ok(Vec::new());
        };
        self.fuel(|_| FUEL_PER_TICK)?;
        let arg = self.realloc.call(&mut self.store, (0, 0, 4, 80))?;
        {
            let data = self.memory.data_mut(&mut self.store);
            let room = data.get_mut(arg as usize..arg as usize + 80).ok_or_else(|| format_err!("the area gave room outside its memory"))?;
            room[0..4].copy_from_slice(&dt.to_le_bytes());
            write_player_state(&mut room[4..80], player);
        }
        let ret = call_tick.call(&mut self.store, arg)?;
        let cmds = read_commands(Bytes(self.memory.data(&self.store)), ret)?;
        post_tick.call(&mut self.store, ret)?;
        Ok(cmds)
    }

    fn interact(&mut self, player: &PlayerState) -> Result<Vec<Command>> {
        let Some((call_interact, post_interact)) = self.interact.clone() else {
            return Ok(Vec::new());
        };
        self.fuel(|_| FUEL_PER_TICK)?;
        let arg = self.realloc.call(&mut self.store, (0, 0, 4, 76))?;
        {
            let data = self.memory.data_mut(&mut self.store);
            let room = data.get_mut(arg as usize..arg as usize + 76).ok_or_else(|| format_err!("the area gave room outside its memory"))?;
            write_player_state(room, player);
        }
        let ret = call_interact.call(&mut self.store, arg)?;
        let cmds = read_commands(Bytes(self.memory.data(&self.store)), ret)?;
        post_interact.call(&mut self.store, ret)?;
        Ok(cmds)
    }
}

fn read_instance(b: &[u8; 52], meshes_len: usize) -> Result<Instance> {
    let u = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let f = |o: usize| f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let id = u(0);
    let mesh = u(4);
    if meshes_len > 0 && mesh as usize >= meshes_len {
        return Err(format_err!("instance references mesh {mesh}, only {meshes_len} exist"));
    }
    let pos = [f(8), f(12), f(16)];
    let mut rot = [f(20), f(24), f(28), f(32)];
    let scale = [f(36), f(40), f(44)];
    let solid = match b[48] {
        0 => false,
        1 => true,
        n => return Err(format_err!("instance solid bool was {n}")),
    };
    if !pos.iter().all(|v| v.is_finite() && (-128.0..=192.0).contains(v))
        || !rot.iter().all(|v| v.is_finite())
        || !scale.iter().all(|v| v.is_finite() && v.abs() <= 64.0)
    {
        return Err(format_err!("an instance has an out-of-range or non-finite transform"));
    }
    let rlen = (rot[0] * rot[0] + rot[1] * rot[1] + rot[2] * rot[2] + rot[3] * rot[3]).sqrt();
    if rlen > 1e-5 {
        for c in &mut rot {
            *c /= rlen;
        }
    } else {
        rot = [0.0, 0.0, 0.0, 1.0];
    }
    Ok(Instance { id, mesh, pos, rot, scale, solid })
}

fn read_light(b: &[u8; 36]) -> Result<Light> {
    let u = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let f = |o: usize| f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let id = u(0);
    let pos = [f(4), f(8), f(12)];
    let color = [f(16).clamp(0.0, 1.0), f(20).clamp(0.0, 1.0), f(24).clamp(0.0, 1.0)];
    let intensity = f(28).clamp(0.0, 100_000.0);
    let radius = f(32).clamp(0.01, 64.0);
    if !pos.iter().all(|v| v.is_finite() && (-128.0..=192.0).contains(v)) || !f(28).is_finite() || !f(32).is_finite() {
        return Err(format_err!("a light has a non-finite or out-of-range value"));
    }
    Ok(Light { id, pos, color, intensity, radius })
}

fn write_player_state(buf: &mut [u8], p: &PlayerState) {
    let put_f = |buf: &mut [u8], o: usize, v: f32| buf[o..o + 4].copy_from_slice(&v.to_le_bytes());
    put_f(buf, 0, p.feet[0]);
    put_f(buf, 4, p.feet[1]);
    put_f(buf, 8, p.feet[2]);
    put_f(buf, 12, p.eye[0]);
    put_f(buf, 16, p.eye[1]);
    put_f(buf, 20, p.eye[2]);
    put_f(buf, 24, p.look[0]);
    put_f(buf, 28, p.look[1]);
    put_f(buf, 32, p.look[2]);
    put_f(buf, 36, p.velocity[0]);
    put_f(buf, 40, p.velocity[1]);
    put_f(buf, 44, p.velocity[2]);
    put_f(buf, 48, p.size);
    buf[52] = p.grounded as u8;
    buf[53] = p.holding_use as u8;
    buf[54] = 0;
    buf[55] = 0;
    buf[56..60].copy_from_slice(&p.target.to_le_bytes());
    put_f(buf, 60, p.aim[0]);
    put_f(buf, 64, p.aim[1]);
    put_f(buf, 68, p.aim[2]);
    put_f(buf, 72, p.time);
}

fn read_commands(d: Bytes<'_>, ret: u32) -> Result<Vec<Command>> {
    let (ptr, len) = (d.u32(ret)?, d.u32(ret + 4)? as usize);
    if len > MOST_COMMANDS {
        return Err(format_err!("the area emitted {len} commands in one step, more than the {MOST_COMMANDS} allowed"));
    }
    let raw = d.slice(ptr, len, 56)?;
    let mut out = Vec::with_capacity(len);
    for (idx, b) in raw.as_chunks::<56>().0.iter().enumerate() {
        let base = ptr + idx as u32 * 56;
        let cmd = match b[0] {
            0 => {
                let slice: &[u8; 52] = b[4..56].try_into().unwrap();
                Command::SetTransform(read_instance(slice, 0)?)
            }
            1 => {
                let id = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
                let vis = match b[8] {
                    0 => false,
                    1 => true,
                    n => return Err(format_err!("set-visible bool was {n}")),
                };
                Command::SetVisible(id, vis)
            }
            2 => {
                let id = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
                let sol = match b[8] {
                    0 => false,
                    1 => true,
                    n => return Err(format_err!("set-solid bool was {n}")),
                };
                Command::SetSolid(id, sol)
            }
            3 => {
                let f = |o: usize| f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
                let mut dv = [f(4), f(8), f(12)];
                if !dv.iter().all(|v| v.is_finite()) {
                    return Err(format_err!("apply-impulse had a non-finite value"));
                }
                let mag = (dv[0] * dv[0] + dv[1] * dv[1] + dv[2] * dv[2]).sqrt();
                if mag > MAX_IMPULSE {
                    let s = MAX_IMPULSE / mag;
                    dv = [dv[0] * s, dv[1] * s, dv[2] * s];
                }
                Command::ApplyImpulse(dv)
            }
            4 => {
                let slice: &[u8; 36] = b[4..40].try_into().unwrap();
                Command::SetLight(read_light(slice)?)
            }
            5 => Command::Say(d.name(base + 4)?),
            tag => return Err(format_err!("unknown command tag {tag}")),
        };
        out.push(cmd);
    }
    Ok(out)
}

/// An area's memory, read with every pointer and length checked.
struct Bytes<'a>(&'a [u8]);

impl Bytes<'_> {
    /// `n` things of `size` bytes from `at`, aligned to 4 if larger than a byte.
    fn slice(&self, at: u32, n: usize, size: usize) -> Result<&[u8]> {
        if size > 1 && !at.is_multiple_of(4) {
            return Err(format_err!("the area answered with a pointer out of line"));
        }
        let end = n.checked_mul(size).and_then(|len| (at as usize).checked_add(len));
        end.and_then(|end| self.0.get(at as usize..end)).ok_or_else(|| format_err!("the area answered outside its memory"))
    }

    fn u32(&self, at: u32) -> Result<u32> {
        let b = self.slice(at, 1, 4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn f32(&self, at: u32) -> Result<f32> {
        Ok(f32::from_bits(self.u32(at)?))
    }

    /// A `string` at `at`: its pointer and its length, UTF-8, not too long.
    fn name(&self, at: u32) -> Result<String> {
        let (ptr, len) = (self.u32(at)?, self.u32(at + 4)? as usize);
        if len > LONGEST_NAME {
            return Err(format_err!("the area gave a name {len} bytes long, more than the {LONGEST_NAME} a name may be"));
        }
        let bytes = self.slice(ptr, len, 1)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| format_err!("the area gave a name that is not UTF-8"))
    }
}
