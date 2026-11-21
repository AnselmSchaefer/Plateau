//! The spike's host (POC_PLAN.md, step 0). An area answers, for a batch of points and a viewer's
//! size, a distance and a material at each; here it is answered either by the area's source built
//! into the host (`Native`) or by its module in wasmtime (`Wasm`), so the two can be compared and
//! timed. `mesh` makes a chunk's surface from those answers.

pub mod garden;
pub mod mesh;

pub use wasmtime::Result;
use wasmtime::error::Context;
use wasmtime::format_err as anyhow;
use wasmtime::{Caller, Config, Engine, Instance, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc};

/// What an area is, to the engine: an answer for each point, at a size.
pub trait Area {
    /// Fills `out` with a distance and a material for each of `points`, for a viewer `size` tall.
    fn sample(&mut self, points: &[[f32; 3]], size: f32, out: &mut Vec<(f32, u32)>) -> Result<()>;
}

/// The area's source, played by the host itself.
pub struct Native {
    pub seed: u64,
}

impl Area for Native {
    fn sample(&mut self, points: &[[f32; 3]], size: f32, out: &mut Vec<(f32, u32)>) -> Result<()> {
        out.clear();
        out.extend(points.iter().map(|&p| garden::sample(p, size, self.seed)));
        Ok(())
    }
}

/// How a module is run: whether instructions are counted, and how much memory it may have.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Fuel for each batch, or none counted.
    pub fuel: Option<u64>,
    pub memory: usize,
    /// Every NaN made the one NaN, so no machine's own NaNs can be seen. Off in play: it makes an
    /// area nearly three times slower (spike/RESULTS.md). The law `nans_unseen` stands in for it.
    pub canonical_nans: bool,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { fuel: Some(1 << 40), memory: 64 << 20, canonical_nans: false }
    }
}

struct Ctx {
    seed: u64,
    limits: StoreLimits,
}

/// An area's module, in wasmtime, with the limits the engine keeps.
pub struct Wasm {
    store: Store<Ctx>,
    memory: Memory,
    alloc: TypedFunc<u32, u32>,
    sample: TypedFunc<(u32, u32, f32, u32), ()>,
    fuel: Option<u64>,
    /// Room in the module for the points and the answers, and for how many points it is.
    points: u32,
    answers: u32,
    room: usize,
}

impl Wasm {
    pub fn new(bytes: &[u8], seed: u64, limits: Limits) -> Result<Wasm> {
        let mut config = Config::new();
        config.consume_fuel(limits.fuel.is_some());
        // The same results on every machine: nothing whose results differ from one processor to the
        // next. Threads are refused by building wasmtime without them. NaNs are left as the
        // machine makes them, unless asked: `nans_unseen` checks that the area never looks.
        config.cranelift_nan_canonicalization(limits.canonical_nans);
        config.wasm_relaxed_simd(false);
        config.wasm_simd(false);
        let engine = Engine::new(&config)?;
        let module = Module::new(&engine, bytes)?;
        let ctx = Ctx { seed, limits: StoreLimitsBuilder::new().memory_size(limits.memory).instances(1).build() };
        let mut store = Store::new(&engine, ctx);
        store.limiter(|ctx| &mut ctx.limits);
        if let Some(fuel) = limits.fuel {
            store.set_fuel(fuel)?;
        }
        let mut linker = Linker::new(&engine);
        linker.func_wrap("host", "seed", |caller: Caller<'_, Ctx>| caller.data().seed as i64)?;
        let instance: Instance = linker.instantiate(&mut store, &module)?;
        let memory = instance.get_memory(&mut store, "memory").ok_or_else(|| anyhow!("the area has no memory"))?;
        let alloc = instance.get_typed_func(&mut store, "alloc").context("the area's alloc")?;
        let sample = instance.get_typed_func(&mut store, "sample").context("the area's sample")?;
        Ok(Wasm { store, memory, alloc, sample, fuel: limits.fuel, points: 0, answers: 0, room: 0 })
    }

    /// The fuel left after the last batch, if counted: what it used is `Limits::fuel` less this.
    pub fn fuel_left(&self) -> Option<u64> {
        self.fuel.and_then(|_| self.store.get_fuel().ok())
    }
}

impl Area for Wasm {
    fn sample(&mut self, points: &[[f32; 3]], size: f32, out: &mut Vec<(f32, u32)>) -> Result<()> {
        let count = points.len();
        if count > self.room {
            // Room is never given back: the batches are about the same size every time.
            self.points = self.alloc.call(&mut self.store, (count * 12) as u32)?;
            self.answers = self.alloc.call(&mut self.store, (count * 8) as u32)?;
            self.room = count;
        }
        let at = self.points as usize;
        let data = self.memory.data_mut(&mut self.store);
        let room = data.get_mut(at..at + count * 12).ok_or_else(|| anyhow!("the area gave room outside its memory"))?;
        for (p, bytes) in points.iter().zip(room.chunks_exact_mut(12)) {
            bytes[0..4].copy_from_slice(&p[0].to_le_bytes());
            bytes[4..8].copy_from_slice(&p[1].to_le_bytes());
            bytes[8..12].copy_from_slice(&p[2].to_le_bytes());
        }
        if let Some(fuel) = self.fuel {
            self.store.set_fuel(fuel)?;
        }
        self.sample.call(&mut self.store, (self.points, count as u32, size, self.answers))?;
        let at = self.answers as usize;
        let data = self.memory.data(&self.store);
        let answers = data.get(at..at + count * 8).ok_or_else(|| anyhow!("the area answered outside its memory"))?;
        out.clear();
        out.extend(answers.chunks_exact(8).map(|b| {
            let d = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            let m = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
            (d, m)
        }));
        Ok(())
    }
}

/// The law that stands in for NaN canonicalization (spike/RESULTS.md): the area gives the same
/// answers, bit for bit, with every NaN made the one NaN and with the machine's own. A NaN's bits
/// differ from one processor to the next (on x86 the machine's NaN is not the one NaN, so the
/// difference shows here); an area whose answers depend on them would make a different world on
/// another machine. On failure, says the first point and size where the answers differ.
pub fn nans_unseen(module: &[u8], seed: u64, points: &[[f32; 3]]) -> Result<std::result::Result<(), String>> {
    let one = Limits { canonical_nans: true, ..Limits::default() };
    let mut one = Wasm::new(module, seed, one)?;
    let mut own = Wasm::new(module, seed, Limits::default())?;
    let (mut a, mut b) = (Vec::new(), Vec::new());
    for &(name, size) in &SIZES {
        for batch in points.chunks(4096) {
            one.sample(batch, size, &mut a)?;
            own.sample(batch, size, &mut b)?;
            if let Some(i) = (0..batch.len()).find(|&i| a[i].0.to_bits() != b[i].0.to_bits() || a[i].1 != b[i].1) {
                let [x, y, z] = batch[i];
                return Ok(Err(format!(
                    "the area looks at a NaN's bits: at ({x}, {y}, {z}) for a {name} ({size} m) it answers {:?} with every \
                     NaN made one and {:?} with this machine's own",
                    a[i], b[i]
                )));
            }
        }
    }
    Ok(Ok(()))
}

/// The three sizes, in metres (CONCEPT.md, "Three sizes").
pub const SIZES: [(&str, f32); 3] = [("tiny", 0.017), ("person", 1.7), ("giant", 17.0)];

/// The voxel a size sees near it: a twentieth of its height (POC_PLAN.md, "Detail per size").
pub fn voxel(size: f32) -> f32 {
    size / 20.0
}

/// FNV-1a over answers' bits: the same answers give the same hash on every machine.
pub fn hash(answers: &[(f32, u32)]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &(d, m) in answers {
        for b in d.to_bits().to_le_bytes().into_iter().chain(m.to_le_bytes()) {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

/// A fixed set of points for hashing and timing: a jittered grid over the garden, from below the
/// pond's floor to above the wall, `n` along each side.
pub fn points(n: usize) -> Vec<[f32; 3]> {
    let mut points = Vec::with_capacity(n * n * n);
    let mut state = 0x2545_f491u32;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state as f32 / u32::MAX as f32
    };
    for i in 0..n {
        for j in 0..n {
            for k in 0..n {
                let x = (i as f32 + next()) / n as f32 * garden::WIDTH;
                let y = (j as f32 + next()) / n as f32 * 8.0 - 3.0;
                let z = (k as f32 + next()) / n as f32 * garden::WIDTH;
                points.push([x, y, z]);
            }
        }
    }
    points
}

/// The module built from `spike/garden/area.rs` by `spike/build.sh`, at an optimization level.
pub fn module(level: &str) -> Result<Vec<u8>> {
    let path = format!("{}/../modules/garden-{level}.wasm", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).with_context(|| format!("{path}: run spike/build.sh first"))
}

/// The whole garden as one chunk at a person's size, 25 cm a cell, from its optimized module:
/// what the viewers show.
pub fn garden_mesh() -> Result<mesh::Mesh> {
    let mut area = Wasm::new(&module("O")?, 7, Limits::default())?;
    let chunk = mesh::Chunk { origin: [-1.0, -3.0, -1.0], voxel: 0.25, cells: [264, 32, 264] };
    let started = std::time::Instant::now();
    let (mesh, timing) = mesh::mesh(&mut area, chunk, 1.7, 4096)?;
    eprintln!(
        "the garden: {} triangles, sampled in {:.0} ms, meshed in {:.0} ms, {:.0} ms in all",
        mesh.indices.len() / 3,
        timing.sampling * 1e3,
        timing.meshing * 1e3,
        started.elapsed().as_secs_f64() * 1e3
    );
    Ok(mesh)
}

/// A material's colour, linear RGB, for the viewers.
pub fn colour(material: u32) -> [f32; 3] {
    match material {
        garden::GRASS => [0.16, 0.32, 0.07],
        garden::SOIL => [0.30, 0.18, 0.09],
        garden::STONE => [0.42, 0.40, 0.37],
        garden::WATER => [0.05, 0.16, 0.35],
        _ => [1.0, 0.0, 1.0],
    }
}
