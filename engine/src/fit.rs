//! Whether an element fits the site it is for (Phase C): the laws a site-made element is held to,
//! and where a carried one (a round door of any size) goes to fit an opening.
//!
//! - **`cavity_fit`**, for an opening: the element's frame meets the wall all the way round, going
//!   into it a little and not too much (`rim_fit`: no floating gap, no burst wall); if a leaf fills
//!   it, nothing gets past the leaf while it is shut, `F` opens it, whoever walked in through the
//!   bare opening still walks in through the open door, and `F` shuts it as tight as it was.
//! - **`ground_fit`**, for anywhere else: the element stands on the ground in every quarter of its
//!   footprint (over water it may reach out), and is not buried.
//! - **`span_fit`**, for a bridge: its ends rest on both banks, it spans a gap between them, and
//!   whoever it is meant for walks across it from end to end on its deck.
//! - **`wall_fit`**, for a shelf or a sign: it meets the wall over its back face, neither standing
//!   off it nor sunk into it.
//! - **`style_fit`**, for any: how much of it is made of what is round about the site; never a
//!   failing, a note for the `look` round when little is.
//! - **`fit_check`** asks whichever the site calls for.
//! - **`fit_transform`** finds where an element made for no site in particular goes: turned to face
//!   out of the opening, and scaled until its frame just goes into the wall all round.
//!
//! The element is judged as it is played: made in its sandbox, its instances in a `SceneWorld`, `F`
//! pressed by a player standing before the door with the crosshair on it, `tick`ed at 30 Hz.
//! Like the site's own measure, the verdicts are IEEE arithmetic all the way: the same everywhere.

use crate::body::{Body, EYE, Intent, RADIUS, STEP as CLIMB};
use crate::compose::{CompiledElement, ElementRuntime, PlacedElement, PlacementTransform};
use crate::laws::{Verdict, length};
use crate::mesh_sdf::MeshSdf;
use crate::scene::{Combined, SceneWorld};
use crate::site::{CONTOUR, Cavity, GRID, Ray, Sight, SiteContext, abs, add, circle, max, min, passes, quat, scale, sub, unit};
use crate::{Answer, Description, Material, PlayerState, Region, Result, Scene};

/// How far a frame goes into the wall beyond the opening's outline, at least and at most, as
/// fractions of the opening's mean radius.
pub const LEAST_OVERLAP: f32 = 0.01;
pub const MOST_OVERLAP: f32 = 0.35;
/// How far `fit_transform` makes it go in where the outline reaches farthest.
const OVERLAP: f32 = 0.07;
/// The floor's part of the outline (within this much of straight down, as a cosine) may be left to
/// the leaf's foot, this far off it at most: a doorway need have no sill.
const FLOOR: f32 = 0.5;
const UNDER_LEAF: f32 = 0.05;
/// A frame's part of the wall is looked for this far in and out of the rim's plane.
const STATIONS: [f32; 5] = [0.0, 0.1, -0.1, 0.2, -0.2];
/// The steps it is looked for in, across the opening, all as fractions of the mean radius.
const STEP: f32 = 0.005;
/// How much of the opening a shut leaf may leave open to the eye (a keyhole), and none of it
/// beside the rim.
const KEYHOLE: f32 = 0.03;
/// The id an element is judged under: never a placed element's.
const JUDGED: u32 = 0x7fff_ffff;
/// How long a door may take to open, in seconds.
const OPENS_WITHIN: f32 = 4.0;

/// How a frame meets the wall round an opening (metres).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RimFit {
    /// How far it goes in beyond the outline where it does so least, and most (the floor left out).
    pub least: f32,
    pub most: f32,
}

/// How an element fits an opening.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CavityFit {
    pub rim: RimFit,
    /// Whether something fills the opening until `F` moves it.
    pub leaf: bool,
    /// Whether whoever measured the opening walks in through it (if they did through it bare).
    pub walkable: bool,
}

/// A clock's hour for the outline's point `k`, as seen from outside.
fn clock(k: usize) -> String {
    let twelfth = (CONTOUR + CONTOUR / 4 - k) % CONTOUR;
    let hour = (twelfth * 12 + CONTOUR / 2) / CONTOUR % 12;
    format!("{} o'clock", if hour == 0 { 12 } else { hour })
}

/// The solid parts of `scene` that do not move, or all of them if all may.
fn frame_of(scene: &Scene) -> Option<MeshSdf> {
    let still = Scene { meshes: scene.meshes.clone(), instances: scene.instances.iter().copied().filter(|i| i.id == 0).collect(), lights: Vec::new() };
    MeshSdf::from_solid_scene(&still).or_else(|| MeshSdf::from_solid_scene(scene))
}

fn inside(sdf: &MeshSdf, p: [f32; 3]) -> bool {
    sdf.sample(p, 1e-3).is_some_and(|a| a.distance < 0.0)
}

/// Whether `element`'s frame, placed at `tf`, meets the wall all round the opening of `site` as it
/// is in `area`: how far it goes in, or where it does not (or goes in too far) and by how much.
pub fn rim_fit(area: &mut dyn Region, site: &SiteContext, scene: &Scene, tf: PlacementTransform) -> Result<std::result::Result<RimFit, String>> {
    let Some(cavity) = &site.cavity else { return Ok(Err("there is no opening here to fit".into())) };
    let Some(frame) = frame_of(scene) else { return Ok(Err("the element has no solid geometry to fit the opening with".into())) };
    let whole = MeshSdf::from_solid_scene(scene);
    let mean = cavity.mean_radius();
    // The wall a little in and out of the rim's plane: where in the frame's depth it can meet it.
    let mut walls: Vec<(f32, Vec<Option<f32>>)> = Vec::new();
    let mut sight = Sight::new(area, site.size);
    for z in STATIONS {
        let from = add(cavity.center, scale(cavity.axis, z * mean));
        let rays: Vec<Ray> = (0..CONTOUR).map(|k| Ray { from, along: cavity.spoke(k), far: 1.4 * cavity.radii[k] + 0.1 * mean }).collect();
        walls.push((z * mean, sight.cast(&rays, false)?.into_iter().map(|h| h.filter(|h| h.t > 0.0).map(|h| h.t)).collect()));
    }
    let steps = (MOST_OVERLAP / STEP) as usize + 2;
    let mut fit = RimFit { least: f32::INFINITY, most: f32::NEG_INFINITY };
    // The worst of each way not to fit, and where: a gap, a frame all within the wall, one too far in.
    type Worst = Option<(f32, usize)>;
    let (mut gap, mut buried, mut far): (Worst, Worst, Worst) = (None, None, None);
    for k in 0..CONTOUR {
        let way = cavity.spoke(k);
        let floor = -way[1] > FLOOR && cavity.upright();
        // How far in it goes at its best station, or else how far off the wall it is.
        let mut best: Option<f32> = None;
        let (mut off, mut beyond) = (f32::INFINITY, f32::INFINITY);
        for (z, radii) in &walls {
            // Not where the opening has flared out, or narrowed, from its measure.
            let Some(r) = radii[k].filter(|r| abs(r - cavity.radii[k]) < 0.1 * mean) else { continue };
            let at = |o: f32| tf.world_to_local_point(add(cavity.center, add(scale(cavity.axis, *z), scale(way, r + o))));
            if inside(&frame, at(0.0)) {
                let n = (1..=steps).take_while(|&i| inside(&frame, at(i as f32 * STEP * mean))).count();
                let o = n as f32 * STEP * mean;
                best = Some(best.map_or(o, |b| max(b, o)));
            } else if best.is_none() {
                let solid = if floor { whole.as_ref().unwrap_or(&frame) } else { &frame };
                if let Some(i) = (1..=60).find(|&i| inside(solid, at(-(i as f32) * STEP * mean))) {
                    off = min(off, i as f32 * STEP * mean);
                }
                if let Some(i) = (1..=steps).find(|&i| inside(&frame, at(i as f32 * STEP * mean))) {
                    beyond = min(beyond, i as f32 * STEP * mean);
                }
            }
        }
        match best {
            Some(o) if o >= LEAST_OVERLAP * mean => {
                if !floor {
                    fit.least = min(fit.least, o);
                    fit.most = max(fit.most, o);
                }
                if o > MOST_OVERLAP * mean && !floor && far.is_none_or(|f| o > f.0) {
                    far = Some((o, k));
                }
            }
            Some(_) => {
                if !floor && gap.is_none() {
                    gap = Some((0.0, k));
                }
            }
            None if floor && off <= UNDER_LEAF * mean => {}
            None if off.is_infinite() && beyond.is_finite() => {
                if buried.is_none_or(|b| beyond > b.0) {
                    buried = Some((beyond, k));
                }
            }
            None => {
                if gap.is_none_or(|g| off > g.0) {
                    gap = Some((off, k));
                }
            }
        }
    }
    let l = |m: f32| format!("{} ({:.3} m local)", length(m), m / tf.scale);
    let seen = |k: usize| format!("{} (as seen from outside)", clock(k));
    Ok(match (gap, buried, far) {
        (Some((off, k)), ..) if off.is_infinite() => {
            Err(format!("there is no frame at all near the wall at {}: it must go all the way round the opening's outline, in the rim's plane", seen(k)))
        }
        (Some((off, k)), ..) => Err(format!(
            "a gap of {} is left between the frame and the wall at {}: its outer edge must go {} to {} beyond the opening's outline all the way round, \
             in the rim's plane",
            l(off),
            seen(k),
            l(LEAST_OVERLAP * mean),
            l(MOST_OVERLAP * mean)
        )),
        (None, Some((beyond, k)), _) => Err(format!(
            "the frame is all inside the wall at {}: its inner edge is {} beyond the opening's outline, where nobody sees it; it must come out of the wall",
            seen(k),
            l(beyond)
        )),
        (None, None, Some((o, k))) => Err(format!(
            "the frame goes {} or more into the wall at {}: at most {} beyond the opening's outline",
            l(o),
            seen(k),
            l(MOST_OVERLAP * mean)
        )),
        (None, None, None) => Ok(fit),
    })
}

/// An element being judged, as it is played: made, placed among what else is there, and ticked.
struct Rig {
    runtime: ElementRuntime,
    world: SceneWorld,
    time: f32,
    size: f32,
}

impl Rig {
    fn new(world: &SceneWorld, element: &CompiledElement, seed: u64, params: &[f32], tf: PlacementTransform, size: f32) -> Result<Rig> {
        let mut rig = Rig { runtime: ElementRuntime::new(), world: world.clone(), time: 0.0, size };
        let placed = PlacedElement {
            id: JUDGED,
            module: element.module.clone(),
            seed,
            pos: tf.pos,
            yaw: 0.0,
            rot: Some(tf.rot),
            scale: tf.scale,
            stamp_size: size,
            after_edits: 0,
            promoted: None,
            clips: Vec::new(),
            local_edits: Vec::new(),
            author: None,
            params: params.to_vec(),
        };
        rig.runtime.spawn_for_size(placed, element, &mut rig.world, size)?;
        Ok(rig)
    }

    /// `F` pressed by whoever stands at `feet` looking at `at`, and what moves left to finish.
    fn press(&mut self, feet: [f32; 3], at: [f32; 3]) {
        let eye = add(feet, [0.0, EYE * self.size, 0.0]);
        let look = unit(sub(at, eye)).unwrap_or([0.0, 0.0, -1.0]);
        let (owner, target) = self.world.raycast_dynamic_owned(eye, look, 12.0 * self.size).map_or((0, 0), |(o, id, _)| (o, id));
        let mut player =
            PlayerState { feet, eye, look, velocity: [0.0; 3], size: self.size, grounded: true, holding_use: false, target, aim: at, time: self.time };
        self.runtime.interact(&player, owner, target, &mut self.world);
        let dt = 1.0 / 30.0;
        for _ in 0..(OPENS_WITHIN / dt) as usize {
            self.time += dt;
            player.time = self.time;
            self.runtime.tick_all(dt, &player, owner, target, &mut self.world);
        }
    }

    /// How far out of and into the rim's plane the element reaches while shut, along the axis.
    fn reach(&self, scene: &Scene, tf: PlacementTransform, cavity: &Cavity) -> (f32, f32) {
        let (mut out, mut within) = (0.0f32, 0.0f32);
        for i in scene.instances.iter().filter(|i| i.solid) {
            let Some(mesh) = scene.meshes.get(i.mesh as usize) else { continue };
            let i = tf.transform_instance(*i);
            for v in &mesh.vertices {
                let scaled = [0, 1, 2].map(|a| v.pos[a] * i.scale[a]);
                let p = add(i.pos, crate::compose::quat_rotate(i.rot, scaled));
                let d = sub(p, cavity.center);
                let z = d[0] * cavity.axis[0] + d[1] * cavity.axis[1] + d[2] * cavity.axis[2];
                (out, within) = (max(out, z), min(within, z));
            }
        }
        (out, within)
    }

    /// Of lines straight in through the opening, those that get past the element and the wall: of
    /// those well within its outline, the share; of those beside it, the first one's hour.
    fn past(&self, area: &mut dyn Region, cavity: &Cavity, (out, within): (f32, f32)) -> Result<(f32, Option<usize>)> {
        let mut combined = Combined { region: area, world: &self.world };
        let mut sight = Sight::new(&mut combined, self.size);
        let pad = 0.03 * cavity.mean_radius();
        let far = out - within + 2.0 * pad;
        let bands = [0.1f32, 0.3, 0.5, 0.65, 0.8, 0.9, 0.97];
        let rays: Vec<Ray> = bands
            .iter()
            .flat_map(|&b| (0..CONTOUR).map(move |k| (b, k)))
            .map(|(b, k)| Ray {
                from: add(cavity.center, add(scale(cavity.axis, out + pad), scale(cavity.spoke(k), b * cavity.radii[k]))),
                along: scale(cavity.axis, -1.0),
                far,
            })
            .collect();
        let hits = sight.cast(&rays, false)?;
        let well = 4 * CONTOUR;
        let clear = hits[..well].iter().filter(|h| h.is_none()).count();
        let beside = hits[well..].iter().position(Option::is_none).map(|i| i % CONTOUR);
        Ok((clear as f32 / well as f32, beside))
    }
}

/// Holds `element`, placed at `tf` with `seed` and `params`, to the opening of `site` in `area`
/// (with `world`'s solid geometry about it): how it fits, or why it does not.
pub fn cavity_fit(
    area: &mut dyn Region,
    world: &SceneWorld,
    site: &SiteContext,
    element: &CompiledElement,
    seed: u64,
    params: &[f32],
    tf: PlacementTransform,
) -> Result<std::result::Result<CavityFit, String>> {
    let Some(cavity) = &site.cavity else { return Ok(Err("there is no opening here to fit".into())) };
    if element.is_sdf() {
        return Ok(Err("what fits an opening is made of meshes (`sdf_bounds() == None`), not a landform that fills it".into()));
    }
    let scene = element.compiled.instantiate_with(seed, params)?.scene()?;
    let rim = {
        let mut combined = Combined { region: &mut *area, world };
        match rim_fit(&mut combined, site, &scene, tf)? {
            Ok(rim) => rim,
            Err(why) => return Ok(Err(why)),
        }
    };
    let mut rig = Rig::new(world, element, seed, params, tf, site.size)?;
    let reach = rig.reach(&scene, tf, cavity);
    let (shut, beside) = rig.past(area, cavity, reach)?;
    // Most of it filled is a leaf: it must fill it all, open, and shut again.
    let leaf = shut < 0.5;
    if leaf {
        let tight = |(shut, beside): (f32, Option<usize>), when: &str| match beside {
            Some(k) => Err(format!("{when}, it can be seen past beside the rim at {} (as seen from outside): the leaf must meet the frame all the way round", clock(k))),
            None if shut > KEYHOLE => Err(format!("{when}, {:.0}% of the opening is left open (at most {:.0}%)", shut * 100.0, KEYHOLE * 100.0)),
            None => Ok(()),
        };
        if let Err(why) = tight((shut, beside), "shut") {
            return Ok(Err(why));
        }
        let h = site.size;
        let middle = cavity.center;
        let stood = {
            let p = add(cavity.center, scale(cavity.axis, max(1.1 * h, reach.0 + 0.5 * h)));
            [p[0], cavity.floor(), p[2]]
        };
        rig.press(stood, middle);
        let (open, _) = rig.past(area, cavity, reach)?;
        if cavity.walkable {
            let mut combined = Combined { region: &mut *area, world: &rig.world };
            if let Err(why) = passes(&mut Sight::new(&mut combined, h), cavity)? {
                return Ok(Err(format!(
                    "pressing F before it, with the crosshair on it, does not open a way in within {OPENS_WITHIN:.0} s: {why} (someone {} tall, {} across, \
                     walks in through the bare opening)",
                    length(h),
                    length(2.0 * RADIUS * h)
                )));
            }
        } else if open < 0.5 {
            return Ok(Err(format!("pressing F before it, with the crosshair on it, opens only {:.0}% of the opening", open * 100.0)));
        }
        rig.press(stood, middle);
        let again = rig.past(area, cavity, reach)?;
        if let Err(why) = tight(again, "shut again with F") {
            return Ok(Err(why));
        }
    } else if cavity.walkable {
        let mut combined = Combined { region: &mut *area, world: &rig.world };
        if let Err(why) = passes(&mut Sight::new(&mut combined, site.size), cavity)? {
            return Ok(Err(format!("it blocks the way in: {why} (someone {} tall walks in through the bare opening)", length(site.size))));
        }
    }
    Ok(Ok(CavityFit { rim, leaf, walkable: cavity.walkable }))
}

/// `world` with `element` placed at `tf` in it, as it is found, or (`pressed`) once whoever measured
/// `site` has pressed `F` before it and what moves has moved: for picturing it.
pub fn staged(
    world: &SceneWorld,
    site: &SiteContext,
    element: &CompiledElement,
    seed: u64,
    params: &[f32],
    tf: PlacementTransform,
    pressed: bool,
) -> Result<SceneWorld> {
    let mut rig = Rig::new(world, element, seed, params, tf, site.size)?;
    if pressed {
        let h = site.size;
        let out = tf.local_to_world_dir([0.0, 0.0, 1.0]);
        let (middle, floor) = match &site.cavity {
            Some(c) => (c.center, c.floor()),
            None => (add(tf.pos, [0.0, 0.5 * h, 0.0]), tf.pos[1]),
        };
        let p = add(middle, scale(out, 1.5 * h));
        rig.press([p[0], floor, p[2]], middle);
    }
    Ok(rig.world)
}

/// Holds `scene`, placed at `tf`, to the ground of `area`: it must stand on it in each quarter of
/// its footprint (those over water aside) and not be buried in it.
pub fn ground_fit(area: &mut dyn Region, size: f32, scene: &Scene, tf: PlacementTransform) -> Result<Verdict> {
    let Some(solid) = MeshSdf::from_solid_scene(scene) else { return Ok(Ok(())) };
    let (lo, hi) = (solid.lo(), solid.hi());
    const N: usize = 14;
    let tall = hi[1] - lo[1];
    let steps = 64usize;
    let (mut lows, mut highs, mut whose) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..N {
        for j in 0..N {
            let x = lo[0] + (hi[0] - lo[0]) * (i as f32 + 0.5) / N as f32;
            let z = lo[2] + (hi[2] - lo[2]) * (j as f32 + 0.5) / N as f32;
            let y = |k: usize| lo[1] + tall * (k as f32 + 0.5) / steps as f32;
            let Some(first) = (0..steps).find(|&k| inside(&solid, [x, y(k), z])) else { continue };
            let last = (0..steps).rev().find(|&k| inside(&solid, [x, y(k), z])).unwrap_or(first);
            lows.push(tf.local_to_world_point([x, y(first), z]));
            highs.push(tf.local_to_world_point([x, y(last), z]));
            whose.push((i * 2 >= N) as usize + 2 * (j * 2 >= N) as usize);
        }
    }
    if lows.is_empty() {
        return Ok(Ok(()));
    }
    let mut sight = Sight::new(area, size);
    let top = tf.pos[1] + max(hi[1], 0.0) * tf.scale + 3.0 * size;
    let rays: Vec<Ray> = lows.iter().map(|p| Ray { from: [p[0], top, p[2]], along: [0.0, -1.0, 0.0], far: 40.0 * size + 4.0 * tall * tf.scale }).collect();
    let wet = sight.cast(&rays, true)?;
    let dry = sight.cast(&rays, false)?;
    let give = 0.02 * tf.scale + tall * tf.scale / steps as f32;
    let names = ["left back (x < 0, z < 0)", "right back (x > 0, z < 0)", "left front (x < 0, z > 0)", "right front (x > 0, z > 0)"];
    let mut under = 0;
    for (quarter, name) in names.iter().enumerate() {
        let (mut nearest, mut over_water, mut columns) = (f32::INFINITY, 0, 0);
        for k in (0..lows.len()).filter(|&k| whose[k] == quarter) {
            columns += 1;
            over_water += wet[k].is_some_and(|w| w.material == Material::Water) as usize;
            if let Some(d) = dry[k] {
                let ground = top - d.t;
                nearest = min(nearest, lows[k][1] - ground);
                under += (highs[k][1] < ground) as usize;
            }
        }
        if columns > 0 && over_water * 2 < columns && nearest > give {
            return Ok(Err(format!(
                "it floats: its {name} quarter is {} ({:.3} m local) or more above the ground; sink its foundations or lower its feet to the ground's heights",
                length(nearest),
                nearest / tf.scale
            )));
        }
    }
    if under * 10 > lows.len() * 8 {
        return Ok(Err("it is buried: nearly all of it is under the ground".into()));
    }
    Ok(Ok(()))
}

/// How much of a span's length at each end is its end, resting on a bank.
const SPAN_END: f32 = 0.15;
/// How many points along it are looked at.
const SPAN_STATIONS: usize = 32;

/// Holds `scene`, placed at `tf` in `area` (with `world`'s solid geometry about it, but not the
/// element's own), to what a bridge is: its solid deck runs along its longer local axis (x or z),
/// both its ends rest on the ground (dry ground, not water), there is a gap under its middle (ground
/// lower than the banks by at least a fifth of the smallest of `sizes`, or water), nothing rises through
/// its deck, and someone of each of `sizes` walks across it from one end to the other on it,
/// neither too wide for it nor falling off.
pub fn span_fit(area: &mut dyn Region, world: &SceneWorld, scene: &Scene, tf: PlacementTransform, sizes: &[f32]) -> Result<Verdict> {
    let Some(solid) = MeshSdf::from_solid_scene(scene) else { return Ok(Err("it has no solid deck to span with".into())) };
    let (lo, hi) = (solid.lo(), solid.hi());
    let along = if hi[0] - lo[0] > hi[2] - lo[2] { 0 } else { 2 };
    let across = 2 - along;
    let (start, end) = (lo[along], hi[along]);
    let long = end - start;
    let mid = 0.5 * (lo[across] + hi[across]);
    let tall = hi[1] - lo[1];
    let steps = 96usize;
    let y = |k: usize| lo[1] + tall * (k as f32 + 0.5) / steps as f32;
    let local = |t: f32, c: f32, y: f32| {
        let mut p = [0.0f32; 3];
        p[along] = t;
        p[across] = c;
        p[1] = y;
        p
    };
    // The deck's bottom and top at each station, along its middle (world), where there is deck.
    let mut deck: Vec<(f32, [f32; 3], [f32; 3])> = Vec::new();
    for i in 0..SPAN_STATIONS {
        let t = start + long * (i as f32 + 0.5) / SPAN_STATIONS as f32;
        let Some(first) = (0..steps).find(|&k| inside(&solid, local(t, mid, y(k)))) else { continue };
        let last = (0..steps).rev().find(|&k| inside(&solid, local(t, mid, y(k)))).unwrap_or(first);
        deck.push(((t - start) / long, tf.local_to_world_point(local(t, mid, y(first))), tf.local_to_world_point(local(t, mid, y(last)))));
    }
    if deck.len() < SPAN_STATIONS / 2 {
        return Ok(Err("it has no deck running along it from end to end".into()));
    }
    let wide = (hi[across] - lo[across]) * tf.scale;
    for &size in sizes {
        if wide < 2.4 * RADIUS * size {
            return Ok(Err(format!("its deck is {} wide, too narrow for someone {} tall", length(wide), length(size))));
        }
    }
    let least = sizes.iter().copied().fold(f32::INFINITY, min);
    let least = if least.is_finite() { least } else { crate::SIZES[1].1 };
    let top = deck.iter().map(|d| d.2[1]).fold(f32::NEG_INFINITY, max) + 3.0 * least;
    let rays: Vec<Ray> = deck.iter().map(|d| Ray { from: [d.1[0], top, d.1[2]], along: [0.0, -1.0, 0.0], far: 60.0 * least + 4.0 * tall * tf.scale }).collect();
    let (wet, dry) = {
        let mut combined = Combined { region: &mut *area, world };
        let mut sight = Sight::new(&mut combined, least);
        (sight.cast(&rays, true)?, sight.cast(&rays, false)?)
    };
    let give = 0.02 * tf.scale + tall * tf.scale / steps as f32;
    let ground = |k: usize| dry[k].map(|h| top - h.t);
    let watery = |k: usize| wet[k].is_some_and(|w| w.material == Material::Water && dry[k].is_none_or(|d| w.t < d.t - 1e-4));
    // Each end rests on a bank.
    for (name, near) in [("near", true), ("far", false)] {
        let ends: Vec<usize> = (0..deck.len()).filter(|&k| if near { deck[k].0 < SPAN_END } else { deck[k].0 > 1.0 - SPAN_END }).collect();
        if ends.is_empty() {
            return Ok(Err(format!("its {name} end has no deck")));
        }
        if ends.iter().all(|&k| watery(k)) {
            return Ok(Err(format!("its {name} end stands in water, not on a bank")));
        }
        let off = ends.iter().map(|&k| ground(k).map_or(f32::INFINITY, |g| deck[k].1[1] - g)).fold(f32::INFINITY, min);
        if off > give {
            return Ok(Err(format!(
                "its {name} end does not rest on the bank: it is {} ({:.3} m local) above the ground there; lengthen it or lower its ends",
                if off.is_finite() { length(off) } else { "far".into() },
                off / tf.scale
            )));
        }
    }
    // A gap under its middle (ground lower than under its ends, or water), and nothing rising
    // through its deck.
    let bank = (0..deck.len())
        .filter(|&k| deck[k].0 < SPAN_END || deck[k].0 > 1.0 - SPAN_END)
        .filter_map(ground)
        .fold(f32::INFINITY, min);
    let mut gap = 0.0f32;
    for k in (0..deck.len()).filter(|&k| deck[k].0 >= SPAN_END && deck[k].0 <= 1.0 - SPAN_END) {
        let below = if watery(k) { f32::INFINITY } else { ground(k).map_or(f32::INFINITY, |g| bank - g) };
        gap = max(gap, below);
        if let Some(g) = ground(k)
            && g > deck[k].2[1] + give
        {
            return Ok(Err(format!("the ground rises through its deck, {:.0}% of the way across", deck[k].0 * 100.0)));
        }
    }
    if gap < 0.2 * least {
        return Ok(Err(format!("it spans no gap: the ground under its middle is at most {} below the banks it rests on", length(max(gap, 0.0)))));
    }
    // Walked across, end to end, by each it is meant for.
    let mut on = world.clone();
    on.add_layer(JUDGED, scene, Some(tf));
    let mut combined = Combined { region: area, world: &on };
    let (first, last) = (deck[0], deck[deck.len() - 1]);
    let way = unit(sub(last.2, first.2)).unwrap_or([0.0, 0.0, 1.0]);
    let span = {
        let d = sub(last.2, first.2);
        (d[0] * d[0] + d[2] * d[2]).sqrt()
    };
    let lowest = deck.iter().map(|d| d.2[1]).fold(f32::INFINITY, min);
    for &size in sizes {
        let from = add(first.2, [0.0, 2.0 * size, 0.0]);
        let mut body = Body::standing(&mut combined, from.map(|c| c as f64), size)?;
        body.yaw = crate::math::atan2(-way[0], -way[2]);
        let start = body.feet;
        let gone = |b: &Body| ((b.feet[0] - start[0]) * way[0] as f64 + (b.feet[2] - start[2]) * way[2] as f64) as f32;
        let mut deepest = body.feet[1] as f32;
        let speed_steps = (60.0 * (span / size) * 2.0) as usize + 120;
        for _ in 0..speed_steps.min(20_000) {
            body.step(&mut combined, Intent { forward: 1.0, ..Intent::default() }, 1.0 / 60.0)?;
            deepest = min(deepest, body.feet[1] as f32);
            if gone(&body) >= span {
                break;
            }
        }
        if deepest < lowest - CLIMB * size - give {
            return Ok(Err(format!(
                "someone {} tall falls off it, {:.0}% of the way across: {} below its deck",
                length(size),
                100.0 * gone(&body) / span,
                length(lowest - deepest)
            )));
        }
        if gone(&body) < span {
            return Ok(Err(format!("someone {} tall gets only {:.0}% of the way across it", length(size), 100.0 * max(gone(&body), 0.0) / span)));
        }
    }
    Ok(Ok(()))
}

/// How much of an element's back must meet the wall.
const WALL_MET: f32 = 0.75;
/// The back's grid, a side.
const WALL_GRID: usize = 12;

/// Where on an element's back a point is, in words, as seen from before it.
fn part(x: f32, y: f32) -> &'static str {
    match (y > 0.33, y < -0.33, x > 0.33, x < -0.33) {
        (true, _, true, _) => "top right",
        (true, _, _, true) => "top left",
        (true, ..) => "top",
        (_, true, true, _) => "bottom right",
        (_, true, _, true) => "bottom left",
        (_, true, ..) => "bottom",
        (.., true, _) => "right",
        (.., true) => "left",
        _ => "middle",
    }
}

/// Holds `scene`, placed at `tf` in `area` (with `world`'s solid geometry about it, but not the
/// element's own), to a wall: its solid's back face (its local -Z side) meets the wall behind it
/// over at least three quarters of it, standing off it by no more than a little and sunk into it by
/// no more than a little more.
pub fn wall_fit(area: &mut dyn Region, world: &SceneWorld, scene: &Scene, tf: PlacementTransform, size: f32) -> Result<Verdict> {
    let Some(solid) = MeshSdf::from_solid_scene(scene) else { return Ok(Err("it has no solid back to meet a wall with".into())) };
    let (lo, hi) = (solid.lo(), solid.hi());
    let thick = hi[2] - lo[2];
    let give = 0.02 * tf.scale;
    let probe = lo[2] + min(0.1 * thick, 0.01);
    // The back: where the solid is just in front of its least z.
    let mut back = Vec::new();
    for i in 0..WALL_GRID {
        for j in 0..WALL_GRID {
            let x = lo[0] + (hi[0] - lo[0]) * (i as f32 + 0.5) / WALL_GRID as f32;
            let y = lo[1] + (hi[1] - lo[1]) * (j as f32 + 0.5) / WALL_GRID as f32;
            if inside(&solid, [x, y, probe]) {
                let (cx, cy) = (0.5 * (lo[0] + hi[0]), 0.5 * (lo[1] + hi[1]));
                back.push((part((x - cx) / max(0.5 * (hi[0] - lo[0]), 1e-6), (y - cy) / max(0.5 * (hi[1] - lo[1]), 1e-6)), [x, y]));
            }
        }
    }
    if back.is_empty() {
        return Ok(Err("it has no flat back facing -Z to meet a wall with".into()));
    }
    // From a little in front of the back, in towards the wall.
    let into = tf.local_to_world_dir([0.0, 0.0, -1.0]);
    let into = unit(into).unwrap_or([0.0, 0.0, -1.0]);
    let before = 3.0 * give;
    let rays: Vec<Ray> =
        back.iter().map(|(_, [x, y])| Ray { from: tf.local_to_world_point([*x, *y, lo[2] + before / tf.scale]), along: into, far: before + 2.0 * size }).collect();
    let hits = {
        let mut combined = Combined { region: area, world };
        Sight::new(&mut combined, size).cast(&rays, false)?
    };
    let (mut met, mut off, mut sunk): (usize, Option<(f32, &str)>, Option<&str>) = (0, None, None);
    for ((name, _), hit) in back.iter().zip(&hits) {
        match hit {
            // Started within the wall: sunk in more than three gives.
            Some(h) if h.t <= 0.0 => {
                sunk.get_or_insert(*name);
            }
            Some(h) if h.t - before <= give => met += 1,
            Some(h) => {
                if off.is_none_or(|(o, _)| h.t - before > o) {
                    off = Some((h.t - before, *name));
                }
            }
            None => {
                if off.is_none_or(|(o, _)| o.is_finite()) {
                    off = Some((f32::INFINITY, *name));
                }
            }
        }
    }
    if met as f32 >= WALL_MET * back.len() as f32 {
        return Ok(Ok(()));
    }
    let share = 100.0 * met as f32 / back.len() as f32;
    Ok(Err(match (sunk, off) {
        (Some(at), _) if met == 0 || off.is_none() => format!("it is sunk into the wall at its {at}: its back must lie on the wall's face, not within it"),
        (_, Some((o, at))) if o.is_infinite() => format!("there is no wall behind its {at} (its back meets the wall over only {share:.0}% of it)"),
        (_, Some((o, at))) => format!(
            "a gap of {} ({:.3} m local) is left between its back and the wall at its {at}; its back meets the wall over only {share:.0}% of it (at least {:.0}%)",
            length(o),
            o / tf.scale,
            WALL_MET * 100.0
        ),
        _ => format!("its back meets the wall over only {share:.0}% of it"),
    }))
}

/// How much of an element is made of what is round about its site.
#[derive(Clone, Debug, PartialEq)]
pub struct StyleFit {
    /// The share of its surface (by area, as placed) made of a material that is at least a
    /// twentieth of what the site's surfaces round about are made of.
    pub share: f32,
    /// Its own materials, most first, as shares of one.
    pub materials: Vec<(Material, f32)>,
    /// For the `look` round, when little of it is of its site: what to look at.
    pub note: Option<String>,
}

/// How `scene` goes with what is round about `site` (`SiteContext::materials`): never a failing (an
/// element may be meant to stand out), but noted when less than half of it is of what is there.
pub fn style_fit(site: &SiteContext, scene: &Scene) -> StyleFit {
    let mut by = [0.0f32; Material::ALL.len()];
    for i in &scene.instances {
        let Some(mesh) = scene.meshes.get(i.mesh as usize) else { continue };
        let k = abs(i.scale[0] * i.scale[1] * i.scale[2]).sqrt();
        for t in mesh.indices.as_chunks::<3>().0 {
            let [a, b, c] = [t[0], t[1], t[2]].map(|v| mesh.vertices.get(v as usize));
            let (Some(a), Some(b), Some(c)) = (a, b, c) else { continue };
            let (u, v) = (sub(b.pos, a.pos), sub(c.pos, a.pos));
            let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
            let area = 0.5 * (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() * k;
            by[a.material as usize] += area;
        }
    }
    let total: f32 = by.iter().sum();
    let mut materials: Vec<(Material, f32)> = Material::ALL.iter().map(|&m| (m, if total > 0.0 { by[m as usize] / total } else { 0.0 })).filter(|(_, k)| *k > 0.0).collect();
    materials.sort_by(|a, b| b.1.total_cmp(&a.1));
    let round: Vec<Material> = site.materials.iter().filter(|(_, k)| *k >= 0.05).map(|(m, _)| *m).collect();
    let share = materials.iter().filter(|(m, _)| round.contains(m)).map(|(_, k)| k).sum::<f32>();
    let words = |ms: &[(Material, f32)]| ms.iter().take(3).map(|(m, k)| format!("{} {:.0}%", m.name(), k * 100.0)).collect::<Vec<_>>().join(", ");
    let note = (share < 0.5 && !materials.is_empty()).then(|| {
        format!(
            "only {:.0}% of it is made of what is round about it (it is {}; round about is {}): look that it goes with its site, or stands out from it as it is meant to",
            share * 100.0,
            words(&materials),
            words(&site.materials)
        )
    });
    StyleFit { share, materials, note }
}

/// Holds an element made for `site` to it, where it is placed for it (`site.frame`), given `params`:
/// to its opening if it has one (`cavity_fit`), else to its ground (`ground_fit`).
pub fn fit_check(area: &mut dyn Region, world: &SceneWorld, site: &SiteContext, element: &CompiledElement, seed: u64, params: &[f32]) -> Result<Verdict> {
    if site.cavity.is_some() {
        return Ok(cavity_fit(area, world, site, element, seed, params, site.frame)?.map(|_| ()));
    }
    let scene = element.compiled.instantiate_with(seed, params)?.scene()?;
    let mut combined = Combined { region: area, world };
    ground_fit(&mut combined, site.size, &scene, site.frame)
}

/// Where an element goes to fit the opening of `site`, given what: placed where the site is
/// (`site.frame`), given the site's params, if it makes itself to fit them; else given none, turned
/// and scaled to fit (`fit_transform`).
#[derive(Clone, Debug, PartialEq)]
pub struct Fitted {
    pub tf: PlacementTransform,
    pub params: Vec<f32>,
    pub rim: RimFit,
}

/// Where `element` goes to fit the opening of `site` in `area` (with `world`'s solid geometry), and
/// what it is given: the site's params, where the site is, if it then fits (it made itself for the
/// opening); else none, turned and scaled to fit if it can be. Its rim is judged as made with
/// `seed`; `cavity_fit` holds it to the rest.
pub fn fit_to_site(area: &mut dyn Region, world: &SceneWorld, site: &SiteContext, element: &CompiledElement, seed: u64) -> Result<std::result::Result<Fitted, String>> {
    if site.cavity.is_none() {
        return Ok(Err("there is no opening here to fit".into()));
    }
    if element.is_sdf() {
        return Ok(Err("what fits an opening is made of meshes (`sdf_bounds() == None`), not a landform that fills it".into()));
    }
    let params = site.params();
    let made = element.compiled.instantiate_with(seed, &params)?.scene()?;
    let by_params = {
        let mut combined = Combined { region: &mut *area, world };
        rim_fit(&mut combined, site, &made, site.frame)?
    };
    let made_for_it = match by_params {
        Ok(rim) => return Ok(Ok(Fitted { tf: site.frame, params, rim })),
        Err(why) => why,
    };
    let scene = element.compiled.instantiate(seed)?.scene()?;
    Ok(match fit_transform(area, world, site, &scene)? {
        Ok((tf, rim)) => Ok(Fitted { tf, params: Vec::new(), rim }),
        Err(why) => Err(format!("given the site's params, {made_for_it}; given none, turned and scaled to fit, {why}")),
    })
}

/// Where `element` goes to fit the opening of `site`, and how its rim does: as it was made, if it was
/// made for an opening just like it; else facing out of it, its frame's middle on the opening's,
/// scaled until it goes a little into the wall everywhere. Or why it does not fit.
pub fn fit_transform(
    area: &mut dyn Region,
    world: &SceneWorld,
    site: &SiteContext,
    scene: &Scene,
) -> Result<std::result::Result<(PlacementTransform, RimFit), String>> {
    let Some(cavity) = &site.cavity else { return Ok(Err("there is no opening here to fit".into())) };
    let Some(frame) = frame_of(scene) else { return Ok(Err("it has no solid frame to fit an opening with".into())) };
    let mut combined = Combined { region: area, world };
    if let Ok(rim) = rim_fit(&mut combined, site, scene, site.frame)? {
        return Ok(Ok((site.frame, rim)));
    }
    // The frame's own outline in the rim's plane, about the middle of what it goes round.
    let (lo, hi) = (frame.lo(), frame.hi());
    let most = {
        let (x, y) = (hi[0] - lo[0], hi[1] - lo[1]);
        (x * x + y * y).sqrt()
    };
    let steps = 400usize;
    let far = |i: usize| most * i as f32 / steps as f32;
    let way = |k: usize| {
        let a = std::f32::consts::TAU * k as f32 / CONTOUR as f32;
        [crate::compose::cos_f32(a), crate::compose::sin_f32(a)]
    };
    let solid = |m: [f32; 2], k: usize, i: usize| inside(&frame, [m[0] + far(i) * way(k)[0], m[1] + far(i) * way(k)[1], 0.0]);
    let about = [0.5 * (lo[0] + hi[0]), 0.5 * (lo[1] + hi[1])];
    let mut within = Vec::with_capacity(CONTOUR);
    for k in 0..CONTOUR {
        match (1..=steps).find(|&i| solid(about, k, i)) {
            Some(i) => within.push([about[0] + far(i) * way(k)[0], about[1] + far(i) * way(k)[1]]),
            // An open foot: as far as the frame goes.
            None if -way(k)[1] > FLOOR => within.push([about[0] + way(k)[0] / -way(k)[1] * (about[1] - lo[1]), lo[1]]),
            None => return Ok(Err("it has no frame going round an opening in its own z = 0 plane".into())),
        }
    }
    let (middle, _) = circle(&within);
    let mut outline = Vec::with_capacity(CONTOUR);
    for k in 0..CONTOUR {
        outline.push(match (1..=steps).rev().find(|&i| solid(middle, k, i)) {
            Some(i) => far(i),
            None if -way(k)[1] > FLOOR => (middle[1] - lo[1]) / -way(k)[1],
            None => return Ok(Err("it has no frame going round an opening in its own z = 0 plane".into())),
        });
    }
    // Scaled so it covers the opening's outline, seen from the middle of the circle round it.
    let (c, _) = cavity.circle();
    let mean = cavity.mean_radius();
    let (right, up) = cavity.across();
    let center = add(cavity.center, add(scale(right, c[0]), scale(up, c[1])));
    let mut k = 0.0f32;
    for i in 0..CONTOUR {
        let d = sub(add(cavity.center, scale(cavity.spoke(i), cavity.radii[i])), center);
        let along = |v: [f32; 3]| d[0] * v[0] + d[1] * v[1] + d[2] * v[2];
        let (x, y) = (along(right), along(up));
        let r = (x * x + y * y).sqrt();
        // The frame's edge that way, between the two of its points it lies between.
        let edge = edge_towards(&outline, x, y);
        if edge <= 1e-6 {
            return Ok(Err("it has no frame going round an opening in its own z = 0 plane".into()));
        }
        k = max(k, (r + OVERLAP * mean) / edge);
    }
    if !(k > 0.0 && k <= 100.0) {
        return Ok(Err("it is too large or small an opening for it".into()));
    }
    let rot = quat(right, up, cavity.axis);
    let offset = crate::compose::quat_rotate(rot, [middle[0] * k, middle[1] * k, 0.0]);
    let tf = PlacementTransform::with_rot(sub(center, offset), rot, k);
    Ok(rim_fit(&mut combined, site, scene, tf)?.map(|rim| (tf, rim)))
}

/// How far an outline's edge is towards (x, y): of its points, the nearest in direction's, shortened
/// to the straight edge to its neighbours (the least there is of the two).
fn edge_towards(outline: &[f32], x: f32, y: f32) -> f32 {
    let l = max((x * x + y * y).sqrt(), 1e-9);
    let (mut best, mut far) = (0usize, f32::NEG_INFINITY);
    for k in 0..outline.len() {
        let a = std::f32::consts::TAU * k as f32 / outline.len() as f32;
        let d = (crate::compose::cos_f32(a) * x + crate::compose::sin_f32(a) * y) / l;
        if d > far {
            (best, far) = (k, d);
        }
    }
    let n = outline.len();
    min(outline[best], min(outline[(best + 1) % n], outline[(best + n - 1) % n]))
}

/// The place a site was measured in, as far as its measure tells, as a `Region` of its own: for an
/// opening, wall from a little before its rim in, but for the measured outline to its measured
/// depth; the ground as its grid lies. What an element is judged and pictured in when only the site
/// file is there, not the world it was measured in.
#[derive(Clone, Debug)]
pub struct SiteRegion {
    site: SiteContext,
    outline: Vec<[f32; 2]>,
    /// How much shorter than the height above it the ground may be (its steepest slope's cosine).
    steep: f32,
}

impl SiteRegion {
    pub fn new(site: SiteContext) -> SiteRegion {
        let outline = site.outline();
        let mut rise = 0.0f32;
        for (k, a) in site.ground.heights.iter().enumerate() {
            let next = |step: usize, on: bool| if on { site.ground.heights.get(k + step).copied().flatten() } else { None };
            for b in [next(1, k % GRID + 1 < GRID), next(GRID, true)].into_iter().flatten() {
                rise = max(rise, abs(b - a.unwrap_or(b)) / max(site.ground.step, 1e-6));
            }
        }
        SiteRegion { site, outline, steep: 1.0 / (1.0 + 2.0 * rise * rise).sqrt() }
    }

    /// The ground's local height at local (x, z), between the grid's points.
    fn height(&self, x: f32, z: f32) -> f32 {
        let g = &self.site.ground;
        let at = |v: f32| min(max(v, 0.0), (GRID - 1) as f32);
        let (u, v) = (at(x / max(g.step, 1e-6) + (GRID / 2) as f32), at((z - g.z0) / max(g.step, 1e-6)));
        let (i, j) = (min(u.floor(), (GRID - 2) as f32) as usize, min(v.floor(), (GRID - 2) as f32) as usize);
        let y = |i: usize, j: usize| g.heights.get(j * GRID + i).copied().flatten().unwrap_or(0.0);
        let (fu, fv) = (u - i as f32, v - j as f32);
        let mix = |a: f32, b: f32, t: f32| a + (b - a) * t;
        mix(mix(y(i, j), y(i + 1, j), fu), mix(y(i, j + 1), y(i + 1, j + 1), fu), fv)
    }

    /// How far (x, y) is from the opening's outline, negative inside it.
    fn opening(&self, x: f32, y: f32) -> f32 {
        let n = self.outline.len();
        let (mut d2, mut within) = (f32::INFINITY, false);
        for k in 0..n {
            let (a, b) = (self.outline[k], self.outline[(k + 1) % n]);
            let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
            let (px, py) = (x - a[0], y - a[1]);
            let t = min(max((px * ex + py * ey) / max(ex * ex + ey * ey, 1e-12), 0.0), 1.0);
            let (qx, qy) = (px - ex * t, py - ey * t);
            d2 = min(d2, qx * qx + qy * qy);
            if (a[1] > y) != (b[1] > y) && x < a[0] + ex * (y - a[1]) / ey {
                within = !within;
            }
        }
        let d = d2.sqrt();
        if within { -d } else { d }
    }
}

impl Region for SiteRegion {
    fn sample(&mut self, points: &[[f32; 3]], _size: f32, out: &mut Vec<Answer>) -> Result<()> {
        out.clear();
        let tf = self.site.frame;
        let soil = self.site.materials.iter().map(|m| m.0).find(|&m| m != Material::Water).unwrap_or(Material::Soil);
        for &p in points {
            let [x, y, z] = tf.world_to_local_point(p);
            let mut d = (y - self.height(x, z)) * self.steep;
            let mut material = soil;
            if let Some(c) = &self.site.cavity {
                // The rim was measured a little in from the wall's face.
                let face = 0.08 * c.mean_radius() / tf.scale;
                let hole = max(self.opening(x, y), -z - c.depth / tf.scale);
                let wall = max(z - face, -hole);
                d = max(d, face - z);
                if wall < d {
                    (d, material) = (wall, c.material);
                }
            }
            out.push(Answer { distance: d * tf.scale, material });
        }
        Ok(())
    }

    fn describe(&mut self) -> Result<Description> {
        wasmtime::bail!("a site's wall is no area: it describes nothing")
    }
}
