//! A player's body (POC_PLAN.md, step 2): a capsule as tall as the player's size, colliding with
//! the area's distances asked at that size, walking, jumping and sitting, growing and shrinking.
//! The engine's physics by size, which no area writes:
//!
//! - **Everything in proportion.** A body's width, its step, its jump and its pace are fractions of
//!   its height, and gravity is scaled with it, so each size moves through its own world as a
//!   person moves through ours: a giant crosses the garden in a few strides, the tiny take a minute
//!   to cross a paving stone.
//! - **The tiny stand on water**: surface tension holds up whoever is smaller than `FLOATS_BELOW`.
//!   Anyone else wades through it to the floor, found where the area's answers turn from water to
//!   a solid (an answer in water says how far the water's surface is, not the floor's).
//! - **A giant sinks into soft ground**: weight grows with the cube of size and what holds it up
//!   with the square, so the larger the body, the deeper it stands in mud, sand and soil (`sinks`).
//!
//! The feet are held up by the ground found under them (five rays down, from a step above the feet
//! to a little below), so a body climbs anything lower than its step and follows the ground down
//! slopes; above the step, spheres along its axis are pushed out of whatever they touch. Positions
//! are in 64-bit floats, the area asked in 32.
//!
//! **A distance is not a contact.** An area's distances may be shorter than the true ones, and are
//! wherever it cuts a shape out of another: in the garden's gate, the wall says the nearest surface
//! is the gate's own outline, 15 cm off in open air. So the body never takes a short distance for
//! a touch: it looks for the surface itself, with rays stepped as far as each distance allows,
//! which stop only where a surface really is.

use crate::{Answer, LARGEST, Material, Region, Result, SMALLEST, chunk};

/// A body's proportions, as fractions of its height: its radius, its eye standing and seated
/// (above the seat), the highest step it climbs without jumping, how high it jumps, its pace
/// walking and running (heights a second), and how fast it falls at most.
pub const RADIUS: f32 = 0.15;
pub const EYE: f32 = 0.93;
pub const SEATED_EYE: f32 = 0.5;
pub const STEP: f32 = 0.25;
pub const JUMP: f32 = 0.35;
pub const WALK: f32 = 0.8;
pub const RUN: f32 = 2.4;
pub const FASTEST_FALL: f32 = 20.0;
/// Gravity for a person (1.7 m), in metres a second a second; scaled with height for other sizes.
pub const GRAVITY: f32 = 9.81;
const PERSON: f32 = 1.7;
/// How long growing or shrinking takes, in seconds.
pub const CHANGE_SECONDS: f32 = 1.0;
/// Water holds up whoever is shorter than this, in metres.
pub const FLOATS_BELOW: f32 = 0.05;
/// What a seat is, as fractions of the body's height: its top from this high above the feet...
pub const SEAT_LOWEST: f32 = 0.15;
/// ...to this high, within this far across the ground.
pub const SEAT_HIGHEST: f32 = 0.45;
pub const SEAT_REACH: f32 = 0.45;

/// How far a giant's foot sinks into a material, as a fraction of its height.
pub fn give(m: Material) -> f32 {
    match m {
        Material::Mud => 0.06,
        Material::Sand => 0.03,
        Material::Soil | Material::Moss => 0.02,
        Material::Grass => 0.015,
        Material::Stone | Material::Wood | Material::Leaf | Material::Water => 0.0,
    }
}

/// How far a body `size` tall sinks into a material, in metres: nothing for the tiny, a little for
/// a person, `give` of its height for a giant, the pressure under a foot growing with size.
pub fn sinks(m: Material, size: f32) -> f32 {
    give(m) * size * ((size / 0.17).log10() * 0.5).clamp(0.0, 1.0)
}

/// What the player asks of the body this moment: to go forward and to the right (each -1 to 1),
/// to run, to jump.
#[derive(Clone, Copy, Debug, Default)]
pub struct Intent {
    pub forward: f32,
    pub right: f32,
    pub run: bool,
    pub jump: bool,
}

/// A size being changed: from and to (metres), and how much of it is done (0 to 1).
#[derive(Clone, Copy, Debug)]
pub struct Change {
    pub from: f32,
    pub to: f32,
    pub done: f32,
}

/// Sitting: where (the seat's top), and where the body stood before, to stand up again.
#[derive(Clone, Copy, Debug)]
pub struct Seat {
    pub at: [f64; 3],
    pub stood: [f64; 3],
}

#[derive(Clone, Debug)]
pub struct Body {
    /// The bottom of the body, in the area's metres.
    pub feet: [f64; 3],
    pub velocity: [f32; 3],
    /// Its height, in metres.
    pub size: f32,
    /// Which way it faces, in radians about the vertical: 0 is north (-z), and it turns
    /// counter-clockwise seen from above (west is a quarter turn).
    pub yaw: f32,
    /// What it stands on, if anything.
    pub on: Option<Material>,
    pub seat: Option<Seat>,
    pub change: Option<Change>,
    /// The last change of size stopped for want of room.
    pub blocked: bool,
}

/// What a ray found.
#[derive(Clone, Copy, Debug)]
enum Hit {
    /// A surface this far along it, of this material.
    At(f64, Material),
    /// It started inside a solid.
    Inside(Material),
    Miss,
}

/// A ray: from a point, along a direction of length one, at most so far.
#[derive(Clone, Copy, Debug)]
struct Ray {
    from: [f64; 3],
    along: [f64; 3],
    length: f64,
}

impl Ray {
    fn at(&self, t: f64) -> [f32; 3] {
        [0, 1, 2].map(|a| (self.from[a] + self.along[a] * t) as f32)
    }
}

/// The 26 ways out of a cube's middle, to its faces, edges and corners: where a sphere looks for
/// what it touches.
fn around() -> [[f64; 3]; 26] {
    let mut ways = [[0.0; 3]; 26];
    let mut k = 0;
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                if (x, y, z) != (0, 0, 0) {
                    let l = ((x * x + y * y + z * z) as f64).sqrt();
                    ways[k] = [x as f64 / l, y as f64 / l, z as f64 / l];
                    k += 1;
                }
            }
        }
    }
    ways
}

/// A surface a sphere touches: how far from its middle, and the way out of it (of length one).
type Touch = (f64, [f64; 3]);

const DOWN: [f64; 3] = [0.0, -1.0, 0.0];
const UP: [f64; 3] = [0.0, 1.0, 0.0];

impl Body {
    /// A body `size` tall standing on the ground under `from`, dropped from there.
    pub fn standing(area: &mut dyn Region, from: [f64; 3], size: f32) -> Result<Body> {
        let mut body = Body { feet: from, ..Body::of_size(size.clamp(SMALLEST, LARGEST)) };
        if let Hit::At(t, m) = body.cast(area, &[Ray { from, along: DOWN, length: 200.0 }])?[0] {
            body.feet[1] = from[1] - t - sinks(m, size) as f64;
            body.on = Some(m);
        }
        Ok(body)
    }

    /// A body `size` tall, nowhere yet: for asking what it touches (`reach`).
    pub fn of_size(size: f32) -> Body {
        Body { feet: [0.0; 3], velocity: [0.0; 3], size, yaw: 0.0, on: None, seat: None, change: None, blocked: false }
    }

    pub fn radius(&self) -> f32 {
        RADIUS * self.size
    }

    /// Whether water holds it up.
    pub fn floats(&self) -> bool {
        self.size < FLOATS_BELOW
    }

    /// Where its eyes are.
    pub fn eye(&self) -> [f64; 3] {
        match self.seat {
            Some(seat) => [seat.at[0], seat.at[1] + (SEATED_EYE * self.size) as f64, seat.at[2]],
            None => [self.feet[0], self.feet[1] + (EYE * self.size) as f64, self.feet[2]],
        }
    }

    /// Forward across the ground, as x and z.
    pub fn forward(&self) -> [f32; 2] {
        [-self.yaw.sin(), -self.yaw.cos()]
    }

    /// Starts growing or shrinking to `size`, over `CHANGE_SECONDS`.
    pub fn grow_to(&mut self, size: f32) {
        let to = size.clamp(SMALLEST, LARGEST);
        self.blocked = false;
        self.change = (to != self.size).then_some(Change { from: self.size, to, done: 0.0 });
    }

    /// The body moved on by `dt` seconds, as the player asks.
    pub fn step(&mut self, area: &mut dyn Region, intent: Intent, dt: f32) -> Result<()> {
        if let Some(seat) = self.seat {
            if intent.forward == 0.0 && intent.right == 0.0 && !intent.jump {
                self.change = None;
                return Ok(());
            }
            self.feet = seat.stood;
            self.seat = None;
        }
        self.change_size(area, dt)?;
        let h = self.size;
        let [fx, fz] = self.forward();
        let (rx, rz) = (-fz, fx);
        let mut wish = [fx * intent.forward + rx * intent.right, fz * intent.forward + rz * intent.right];
        let len = (wish[0] * wish[0] + wish[1] * wish[1]).sqrt();
        if len > 1.0 {
            wish = wish.map(|w| w / len);
        }
        let pace = if intent.run { RUN } else { WALK } * h;
        let wish = wish.map(|w| w * pace);
        if self.on.is_some() {
            (self.velocity[0], self.velocity[2]) = (wish[0], wish[1]);
            if intent.jump {
                self.velocity[1] = (2.0 * self.gravity() * JUMP * h).sqrt();
                self.on = None;
            }
        } else {
            let k = (3.0 * dt).min(1.0);
            self.velocity[0] += (wish[0] - self.velocity[0]) * k;
            self.velocity[2] += (wish[1] - self.velocity[2]) * k;
        }
        let speed = self.velocity.iter().map(|v| v * v).sum::<f32>().sqrt() + self.gravity() * dt;
        let substeps = ((speed * dt / (0.3 * self.radius())).ceil() as usize).clamp(1, 32);
        for _ in 0..substeps {
            self.substep(area, dt / substeps as f32)?;
        }
        Ok(())
    }

    fn gravity(&self) -> f32 {
        GRAVITY * self.size / PERSON
    }

    fn substep(&mut self, area: &mut dyn Region, dt: f32) -> Result<()> {
        let h = self.size;
        let was_on = self.on.is_some();
        if !was_on {
            self.velocity[1] = (self.velocity[1] - self.gravity() * dt).max(-FASTEST_FALL * h);
        }
        for a in 0..3 {
            self.feet[a] += (self.velocity[a] * dt) as f64;
        }
        self.resolve(area)?;
        let snap = if was_on && self.velocity[1] <= 0.0 { 0.1 * h } else { 0.0 };
        match self.ground(area, snap)? {
            Some((y, m)) if self.velocity[1] <= 0.0 || y > self.feet[1] => {
                self.feet[1] = y;
                self.velocity[1] = 0.0;
                self.on = Some(m);
            }
            _ => self.on = None,
        }
        Ok(())
    }

    /// The middles of the spheres along the body's axis above its step.
    fn spheres(&self) -> [[f64; 3]; 4] {
        self.spheres_at(self.feet)
    }

    /// The same, for the body standing at `feet`.
    pub(crate) fn spheres_at(&self, feet: [f64; 3]) -> [[f64; 3]; 4] {
        let (h, r) = (self.size, self.radius());
        let (low, high) = (STEP * h + r, h - r);
        [0.0, 1.0, 2.0, 3.0].map(|k| [feet[0], feet[1] + (low + (high - low) * k / 3.0) as f64, feet[2]])
    }

    /// The distance to a solid an answer says, for this body: none for water it does not float on.
    fn solid(&self, a: &Answer) -> Option<f32> {
        (a.material != Material::Water || self.floats()).then_some(a.distance)
    }

    /// For each of `middles`, how far the nearest surface within `reach` is and which way, if one
    /// is: rays out in 26 directions, each from as far as the middle's distance says is clear.
    pub(crate) fn touching(&self, area: &mut dyn Region, middles: &[[f64; 3]], reach: f64) -> Result<Vec<Option<Touch>>> {
        let mut out = Vec::new();
        let points: Vec<[f32; 3]> = middles.iter().map(|p| p.map(|c| c as f32)).collect();
        chunk::sample(area, &points, self.size, &mut out)?;
        let clear: Vec<Option<f64>> = out.iter().map(|a| self.solid(a).map(|d| d as f64)).collect();
        let ways = around();
        let mut rays = Vec::new();
        let mut whose = Vec::new();
        for (k, m) in middles.iter().enumerate() {
            let start = clear[k].unwrap_or(0.0).max(0.0);
            if start >= reach {
                continue;
            }
            for w in &ways {
                rays.push(Ray { from: [0, 1, 2].map(|a| m[a] + w[a] * start), along: *w, length: reach - start });
                whose.push((k, start));
            }
        }
        let hits = self.cast(area, &rays)?;
        let mut found: Vec<Option<Touch>> = vec![None; middles.len()];
        let mut weights = vec![([0.0f64; 3], 0.0f64); middles.len()];
        for ((ray, &(k, start)), hit) in rays.iter().zip(&whose).zip(&hits) {
            let t = match *hit {
                Hit::At(t, _) => start + t,
                Hit::Inside(_) => start,
                Hit::Miss => continue,
            };
            // The way out of it: away from the directions it was met in, the nearest most.
            let w = reach - t;
            for a in 0..3 {
                weights[k].0[a] -= ray.along[a] * w;
            }
            weights[k].1 = weights[k].1.max(w);
            if found[k].is_none_or(|(near, _)| t < near) {
                found[k] = Some((t, [0.0; 3]));
            }
        }
        for (k, f) in found.iter_mut().enumerate() {
            if let Some((t, way)) = f {
                let l = weights[k].0.iter().map(|c| c * c).sum::<f64>().sqrt();
                *way = if l > 1e-6 * reach { weights[k].0.map(|c| c / l) } else { UP };
                *t = (*t).min(reach);
            }
        }
        Ok(found)
    }

    /// Pushes the spheres above the step out of what they touch, across the ground or down, never
    /// up: the ground under the feet holds the body up.
    fn resolve(&mut self, area: &mut dyn Region) -> Result<()> {
        let r = self.radius() as f64;
        for _ in 0..3 {
            let touching = self.touching(area, &self.spheres(), r)?;
            if touching.iter().all(Option::is_none) {
                break;
            }
            let (mut most, mut least) = ([0f64; 3], [0f64; 3]);
            for (t, way) in touching.into_iter().flatten() {
                let mut push = way.map(|c| c * (r - t));
                push[1] = push[1].min(0.0);
                for a in 0..3 {
                    most[a] = most[a].max(push[a]);
                    least[a] = least[a].min(push[a]);
                }
            }
            for a in 0..3 {
                self.feet[a] += most[a] + least[a];
            }
            if least[1] < 0.0 && self.velocity[1] > 0.0 {
                self.velocity[1] = 0.0;
            }
        }
        Ok(())
    }

    /// The ground under the feet: the highest surface found by five rays down (the middle of the
    /// foot and four about its edge), from a step above the feet to `below` under them, less what
    /// the body sinks into it. If every ray starts inside a solid, the body is lifted by its step.
    fn ground(&self, area: &mut dyn Region, below: f32) -> Result<Option<(f64, Material)>> {
        let (h, r) = (self.size as f64, self.radius() as f64 * 0.6);
        let [x, y, z] = self.feet;
        let top = y + STEP as f64 * h;
        let rays: Vec<Ray> = [[0.0, 0.0], [r, 0.0], [-r, 0.0], [0.0, r], [0.0, -r]]
            .map(|[dx, dz]| Ray { from: [x + dx, top, z + dz], along: DOWN, length: top - y + below as f64 })
            .to_vec();
        let found = self.cast(area, &rays)?;
        let mut best: Option<(f64, Material)> = None;
        for hit in &found {
            if let Hit::At(t, m) = *hit {
                let at = top - t - sinks(m, self.size) as f64;
                if best.is_none_or(|(b, _)| at > b) {
                    best = Some((at, m));
                }
            }
        }
        if best.is_none()
            && let Some(m) = found.iter().map(|h| if let Hit::Inside(m) = h { Some(*m) } else { None }).collect::<Option<Vec<_>>>()
        {
            return Ok(Some((top, m[0])));
        }
        Ok(best)
    }

    /// The rays, together: each stepped as far as the area says is clear (through water it does
    /// not float on, a twentieth of the body at a time, since there the answers say nothing of the
    /// floor), and brought back to the surface by halving if it stepped into a solid.
    fn cast(&self, area: &mut dyn Region, rays: &[Ray]) -> Result<Vec<Hit>> {
        let h = self.size as f64;
        let close = 0.002 * h;
        let mut t = vec![0.0f64; rays.len()];
        let mut outside: Vec<Option<f64>> = vec![None; rays.len()];
        let mut found = vec![Hit::Miss; rays.len()];
        let mut active: Vec<usize> = (0..rays.len()).collect();
        // Rays that went into a solid: (ray, last clear, first inside, material).
        let mut refine = Vec::new();
        let mut out = Vec::new();
        for _ in 0..256 {
            if active.is_empty() {
                break;
            }
            let points: Vec<[f32; 3]> = active.iter().map(|&k| rays[k].at(t[k])).collect();
            chunk::sample(area, &points, self.size, &mut out)?;
            let mut still = Vec::with_capacity(active.len());
            for (j, &k) in active.iter().enumerate() {
                let a = out[j];
                let step = match self.solid(&a) {
                    Some(d) if (d as f64) <= close => {
                        match outside[k] {
                            None if (d as f64) < -close => found[k] = Hit::Inside(a.material),
                            None => found[k] = Hit::At(t[k], a.material),
                            Some(_) if d >= 0.0 => found[k] = Hit::At(t[k], a.material),
                            Some(clear) => refine.push((k, clear, t[k], a.material)),
                        }
                        continue;
                    }
                    Some(d) => d as f64,
                    None => (a.distance.abs() as f64).clamp(close, h / 20.0),
                };
                outside[k] = Some(t[k]);
                if t[k] < rays[k].length {
                    t[k] = (t[k] + step).min(rays[k].length);
                    still.push(k);
                }
            }
            active = still;
        }
        for _ in 0..10 {
            if refine.is_empty() {
                break;
            }
            let points: Vec<[f32; 3]> = refine.iter().map(|&(k, o, i, _)| rays[k].at((o + i) * 0.5)).collect();
            chunk::sample(area, &points, self.size, &mut out)?;
            for (j, r) in refine.iter_mut().enumerate() {
                let mid = (r.1 + r.2) * 0.5;
                match self.solid(&out[j]) {
                    Some(d) if d < 0.0 => (r.2, r.3) = (mid, out[j].material),
                    _ => r.1 = mid,
                }
            }
        }
        for (k, clear, _, m) in refine {
            found[k] = Hit::At(clear, m);
        }
        Ok(found)
    }

    /// Whether the spheres above the step are clear of anything solid within most of their radius.
    fn room(&self, area: &mut dyn Region) -> Result<bool> {
        Ok(self.touching(area, &self.spheres(), 0.7 * self.radius() as f64)?.iter().all(Option::is_none))
    }

    /// If the body is embedded inside solid geometry (for example after raising terrain where the
    /// player stands), lifts `feet[1]` straight up to the first clear surface above `(feet[0],
    /// feet[2])` within `max_lift`, zeroing velocity so the player rises smoothly onto the new
    /// surface instead of being pushed sideways or catapulted.
    pub fn surface_from_solid(&mut self, area: &mut dyn Region, max_lift: f64) -> Result<bool> {
        let r = self.radius() as f64;
        if self.touching(area, &self.spheres(), r)?.iter().all(Option::is_none) {
            return Ok(false);
        }
        let h = self.size as f64;
        let step = (0.15 * h).max(1e-4);
        let limit = max_lift.max(h);
        let orig_y = self.feet[1];
        let mut y = orig_y + step;
        while y <= orig_y + limit {
            let candidate = [self.feet[0], y, self.feet[2]];
            let clear = self.touching(area, &self.spheres_at(candidate), r)?.iter().all(Option::is_none);
            if clear {
                let probe_from = [candidate[0], y + STEP as f64 * h, candidate[2]];
                if let Hit::At(t, m) = self.cast(area, &[Ray { from: probe_from, along: DOWN, length: (y - orig_y) + STEP as f64 * h }])?[0] {
                    self.feet[1] = probe_from[1] - t - sinks(m, self.size) as f64;
                    self.on = Some(m);
                } else {
                    self.feet[1] = y;
                    self.on = None;
                }
                self.velocity = [0.0; 3];
                self.seat = None;
                return Ok(true);
            }
            y += step;
        }
        Ok(false)
    }

    /// A change of size carried on: growing or shrinking evenly in proportion, eased in and out;
    /// growing stops where there is no room to.
    fn change_size(&mut self, area: &mut dyn Region, dt: f32) -> Result<()> {
        let Some(mut change) = self.change else { return Ok(()) };
        change.done = (change.done + dt / CHANGE_SECONDS).min(1.0);
        let t = change.done * change.done * (3.0 - 2.0 * change.done);
        let before = self.size;
        self.size = change.from * (change.to / change.from).powf(t);
        if self.size > before && !self.room(area)? {
            self.size = before;
            self.change = None;
            self.blocked = true;
            return Ok(());
        }
        self.change = (change.done < 1.0).then_some(change);
        Ok(())
    }

    /// Sits on the nearest seat within reach, if there is one: a surface at about knee height, with
    /// room above it. Whether it sat. A giant's bench is a seat to a giant and a roof to a person.
    pub fn sit(&mut self, area: &mut dyn Region) -> Result<bool> {
        if self.on.is_none() || self.seat.is_some() {
            return Ok(false);
        }
        let h = self.size as f64;
        let [x, y, z] = self.feet;
        let (top, k) = (y + SEAT_HIGHEST as f64 * h, SEAT_REACH as f64 / 6.0 * h);
        let mut rays = Vec::new();
        for i in -6i32..=6 {
            for j in -6i32..=6 {
                if i * i + j * j <= 36 {
                    rays.push(Ray { from: [x + i as f64 * k, top, z + j as f64 * k], along: DOWN, length: (SEAT_HIGHEST - SEAT_LOWEST) as f64 * h });
                }
            }
        }
        let found = self.cast(area, &rays)?;
        let seats: Vec<([f64; 3], Material)> = rays
            .iter()
            .zip(&found)
            .filter_map(|(r, hit)| if let Hit::At(t, m) = *hit { Some(([r.from[0], top - t, r.from[2]], m)) } else { None })
            .collect();
        // Room to sit: nothing above the seat for half the body's height.
        let up: Vec<Ray> = seats.iter().map(|(at, _)| Ray { from: [at[0], at[1] + 0.02 * h, at[2]], along: UP, length: 0.5 * h }).collect();
        let clear = self.cast(area, &up)?;
        let far = |at: &[f64; 3]| (at[0] - x).powi(2) + (at[2] - z).powi(2);
        let best = seats.iter().zip(&clear).filter(|(_, c)| matches!(c, Hit::Miss)).map(|(s, _)| *s).min_by(|a, b| far(&a.0).total_cmp(&far(&b.0)));
        let Some((at, m)) = best else { return Ok(false) };
        self.seat = Some(Seat { at, stood: self.feet });
        self.velocity = [0.0; 3];
        self.on = Some(m);
        self.change = None;
        Ok(true)
    }
}

/// Where a ray from `from` along `dir` (of length one) first meets a surface, as a viewer `size`
/// tall sees it, within `far` metres: the point a viewer is looking at.
pub fn look(area: &mut dyn Region, from: [f64; 3], dir: [f32; 3], size: f32, far: f32) -> Result<Option<[f64; 3]>> {
    let mut t = 0.0f32;
    let mut out = Vec::new();
    for _ in 0..300 {
        let p = [0, 1, 2].map(|a| (from[a] + (dir[a] * t) as f64) as f32);
        chunk::sample(area, &[p], size, &mut out)?;
        let d = out[0].distance;
        if d < 0.001 * size + 0.001 * t {
            return Ok(Some([0, 1, 2].map(|a| from[a] + (dir[a] * t) as f64)));
        }
        t += d.max(0.0005 * size);
        if t > far {
            break;
        }
    }
    Ok(None)
}
