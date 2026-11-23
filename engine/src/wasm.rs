//! An area's module in wasmtime, with the limits the engine keeps (POC_PLAN.md, "Running areas"):
//! fuel for each batch, a memory limit, no SIMD and no threads, NaNs left as the machine makes them
//! (the law `laws::nans_unseen` stands in for making them one). The engine writes the points and
//! reads the answers and the description as the canonical ABI lays out `plateau:region@0.1.0`'s
//! types, checking every pointer, length and number the area gives back.

use wasmtime::error::Context;
use wasmtime::{Caller, Config, Engine, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc, format_err};

use crate::check::{self, HOST, MOST_MEMORY};
use crate::{Answer, Borders, Description, Edge, LARGEST, Material, Place, Prop, PropKind, Region, Result, SMALLEST, Size};

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

/// Fuel for a batch besides its points', and for `describe` / `props`.
pub const FUEL_PER_CALL: u64 = 1_000_000;
pub const FUEL_TO_DESCRIBE: u64 = 100_000_000;
/// The most smooth GPU props an area may emit.
pub const MOST_PROPS: usize = 16_384;
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
        let props = match (instance.get_typed_func(&mut store, "props"), instance.get_typed_func(&mut store, "cabi_post_props")) {
            (Ok(f), Ok(p)) => Some((f, p)),
            _ => None,
        };
        Ok(Wasm {
            realloc: instance.get_typed_func(&mut store, "cabi_realloc").context("the area's cabi_realloc")?,
            sample: instance.get_typed_func(&mut store, "sample").context("the area's sample")?,
            post_sample: instance.get_typed_func(&mut store, "cabi_post_sample").context("the area's cabi_post_sample")?,
            describe: instance.get_typed_func(&mut store, "describe").context("the area's describe")?,
            post_describe: instance.get_typed_func(&mut store, "cabi_post_describe").context("the area's cabi_post_describe")?,
            props,
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
    props: Option<(TypedFunc<(), u32>, TypedFunc<u32, ()>)>,
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

    fn props(&mut self) -> Result<Vec<Prop>> {
        let Some((call_props, post_props)) = self.props.clone() else {
            return Ok(Vec::new());
        };
        self.fuel(|_| FUEL_TO_DESCRIBE)?;
        let ret = call_props.call(&mut self.store, ())?;
        let d = Bytes(self.memory.data(&self.store));
        let (list, len) = (d.u32(ret)?, d.u32(ret + 4)? as usize);
        if len > MOST_PROPS {
            return Err(format_err!("the area gave {len} props, more than the {MOST_PROPS} it may"));
        }
        // `prop`: kind (u8, 0), material (u8, 1), pad (2), a (12, 4..16), b (12, 16..28), r (4, 28..32), yaw (4, 32..36).
        let raw = d.slice(list, len, 36)?;
        let mut out = Vec::with_capacity(len);
        for chunk in raw.as_chunks::<36>().0 {
            let kind = PropKind::from_number(chunk[0]).ok_or_else(|| format_err!("prop kind {} is not one of the engine's", chunk[0]))?;
            let material = Material::from_number(chunk[1]).ok_or_else(|| format_err!("prop material {} is not one of the engine's", chunk[1]))?;
            let f = |off: usize| f32::from_le_bytes([chunk[off], chunk[off + 1], chunk[off + 2], chunk[off + 3]]);
            let a = [f(4), f(8), f(12)];
            let b = [f(16), f(20), f(24)];
            let r = f(28);
            let yaw = f(32);
            if !a.iter().chain(&b).chain(&[r, yaw]).all(|v| v.is_finite()) || r < 0.0 || r > 32.0 {
                return Err(format_err!("a prop has an out-of-range or non-finite value"));
            }
            out.push(Prop { kind, material, a, b, r, yaw });
        }
        post_props.call(&mut self.store, ret)?;
        Ok(out)
    }
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
