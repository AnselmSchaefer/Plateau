//! Engine-managed hydrostatic water bodies & dynamic pond expansion (Phase A).
//!
//! Solves the SDF water limitation (`POC_PLAN.md`, step 1 & Phase A):
//! - Base `.wasm` areas bake their original ponds into `sample()` with a fixed horizontal reach,
//!   so carving outward from a pond bank previously left dry air (and even notched the pond water).
//! - `WaterLog` maintains engine-owned `WaterBody` volumes (`level` + `Vec<WaterBasin>`) composited
//!   **after** all solid `Edit`s and placed SDF elements via exact 1-Lipschitz `water_d.cut(solid_d)`.
//! - Whenever a player carves a solid `Edit` that connects to `Material::Water` (in the base area
//!   or an existing `WaterBody`) below `level`, `WaterLog::expand_from_carve` detects the water
//!   surface level and floods both the new carve and any transitively connected pre-dug dry cavities
//!   at that exact hydrostatic level—while solid `Op::Add` dams automatically cut cleanly through
//!   water and dry pits far from water remain dry!

use std::collections::VecDeque;

use wasmtime::bail;

use crate::chunk;
use crate::edit::{Edit, EditLog, Op, Shape};
use crate::{Answer, Material, Region, Result};

/// One geometric pocket belonging to a `WaterBody` (exact `Shape` + `center` expanded by `pad`
/// so the water presses seamlessly into carved dirt walls when cut by `solid_d`, without sticking
/// out into open air).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaterBasin {
    pub shape: Shape,
    pub center: [f32; 3],
    pub pad: f32,
}

impl WaterBasin {
    pub fn from_edit(edit: &Edit) -> WaterBasin {
        // Pad past the full `smooth_cut` blend radius proportionally to the edit's own scale
        // (tiny 1.7 cm carves get millimetre padding; person/giant carves get proportionate padding)
        // so `d_body == p.y - level` across the entire transition band where `Op::Carve` touched
        // existing pond water (zero seam, zero recession at every scale!).
        let pad = edit.blend + max_f32(0.15 * edit.shape.radius(), 0.002);
        WaterBasin { shape: edit.shape, center: edit.center, pad }
    }

    pub fn validate(&self) -> Result<()> {
        self.shape.validate()?;
        if !self.center.iter().all(|c| c.is_finite()) {
            bail!("water basin center must be finite, got {:?}", self.center);
        }
        if !self.pad.is_finite() || self.pad < 0.0 {
            bail!("water basin pad must be finite and non-negative, got {}", self.pad);
        }
        Ok(())
    }

    /// World AABB enclosing this basin up to `level`.
    pub fn bounds(&self, level: f32) -> ([f32; 3], [f32; 3]) {
        let h = self.shape.half_extents();
        let lo = [self.center[0] - h[0] - self.pad, self.center[1] - h[1] - self.pad, self.center[2] - h[2] - self.pad];
        let hi = [
            self.center[0] + h[0] + self.pad,
            min_f32(self.center[1] + h[1] + self.pad, level),
            self.center[2] + h[2] + self.pad,
        ];
        (lo, hi)
    }

    /// Exact 1-Lipschitz SDF of this basin capped at hydrostatic surface `y = level`.
    #[inline]
    pub fn distance(&self, p: [f32; 3], level: f32) -> f32 {
        let q = [p[0] - self.center[0], p[1] - self.center[1], p[2] - self.center[2]];
        let d_shape = self.shape.distance(q) - self.pad;
        max_f32(p[1] - level, d_shape)
    }
}

/// One hydrostatic water body (an expanded pond/channel or placed spring) at constant height `level`.
#[derive(Clone, Debug, PartialEq)]
pub struct WaterBody {
    pub id: u32,
    pub level: f32,
    pub seed_point: [f32; 3],
    pub basins: Vec<WaterBasin>,
    pub author: Option<String>,
}

impl WaterBody {
    pub fn validate(&self) -> Result<()> {
        if self.id == 0 {
            bail!("water body id must be >= 1");
        }
        if !self.level.is_finite() {
            bail!("water level must be finite, got {}", self.level);
        }
        if !self.seed_point.iter().all(|c| c.is_finite()) {
            bail!("water seed_point must be finite, got {:?}", self.seed_point);
        }
        if let Some(a) = &self.author
            && (a.is_empty() || a.chars().any(char::is_whitespace))
        {
            bail!("invalid water author `{a}`");
        }
        for b in &self.basins {
            b.validate()?;
        }
        Ok(())
    }

    /// World AABB enclosing all basins of this water body (clipped to `y <= level`).
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for b in &self.basins {
            let (blo, bhi) = b.bounds(self.level);
            if bhi[1] <= blo[1] {
                continue;
            }
            for a in 0..3 {
                lo[a] = min_f32(lo[a], blo[a]);
                hi[a] = max_f32(hi[a], bhi[a]);
            }
        }
        (lo[0].is_finite() && hi[0] > lo[0]).then_some((lo, hi))
    }

    /// Exact 1-Lipschitz SDF of this water body (`min` over all basins).
    #[inline]
    pub fn distance(&self, p: [f32; 3]) -> f32 {
        let mut best = f32::INFINITY;
        for b in &self.basins {
            best = min_f32(best, b.distance(p, self.level));
        }
        best
    }
}

/// Immutable, thread-safe collection of engine-managed `WaterBody`s composited after solid edits.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WaterLog {
    bodies: Vec<WaterBody>,
    /// Precomputed world AABB per body (`None` if empty or wholly above water level).
    bounds: Vec<Option<([f32; 3], [f32; 3])>>,
    /// Union AABB of all active bodies for fast batch rejection.
    all_bounds: Option<([f32; 3], [f32; 3])>,
    revision: u64,
}

impl WaterLog {
    pub fn new() -> WaterLog {
        WaterLog::default()
    }

    pub fn from_bodies(bodies: Vec<WaterBody>) -> Result<WaterLog> {
        for b in &bodies {
            b.validate()?;
        }
        let mut log = WaterLog { bodies, bounds: Vec::new(), all_bounds: None, revision: 1 };
        log.rebuild_bounds();
        Ok(log)
    }

    pub fn bodies(&self) -> &[WaterBody] {
        &self.bodies
    }

    pub fn is_empty(&self) -> bool {
        self.all_bounds.is_none()
    }

    pub fn all_bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        self.all_bounds
    }

    /// Tight world AABB enclosing all basins that differ between `self` and `other` (for targeted
    /// chunk invalidation on undo/redo without rebuilding unrelated chunks).
    pub fn diff_bounds(&self, other: &WaterLog) -> Option<([f32; 3], [f32; 3])> {
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        let mut any = false;
        let mut include = |blo: [f32; 3], bhi: [f32; 3]| {
            if bhi[1] <= blo[1] {
                return;
            }
            any = true;
            for a in 0..3 {
                lo[a] = min_f32(lo[a], blo[a]);
                hi[a] = max_f32(hi[a], bhi[a]);
            }
        };
        let contains_basin = |log: &WaterLog, level: f32, basin: &WaterBasin| {
            log.bodies.iter().any(|b| (b.level - level).abs() <= 1e-4 && b.basins.contains(basin))
        };
        for body in &self.bodies {
            for b in &body.basins {
                if !contains_basin(other, body.level, b) {
                    let (blo, bhi) = b.bounds(body.level);
                    include(blo, bhi);
                }
            }
        }
        for body in &other.bodies {
            for b in &body.basins {
                if !contains_basin(self, body.level, b) {
                    let (blo, bhi) = b.bounds(body.level);
                    include(blo, bhi);
                }
            }
        }
        any.then_some((lo, hi))
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn next_id(&self) -> u32 {
        self.bodies.iter().map(|b| b.id).max().unwrap_or(0) + 1
    }

    /// Adds a standalone water body (e.g., player-placed spring/pool) and returns its id.
    pub fn push_body(&mut self, mut body: WaterBody) -> Result<u32> {
        if body.id == 0 {
            body.id = self.next_id();
        }
        body.validate()?;
        let id = body.id;
        self.bodies.push(body);
        self.revision += 1;
        self.rebuild_bounds();
        Ok(id)
    }

    fn rebuild_bounds(&mut self) {
        self.bounds.clear();
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        let mut any = false;
        for body in &self.bodies {
            let b = body.bounds();
            if let Some((blo, bhi)) = b {
                any = true;
                for a in 0..3 {
                    lo[a] = min_f32(lo[a], blo[a]);
                    hi[a] = max_f32(hi[a], bhi[a]);
                }
            }
            self.bounds.push(b);
        }
        self.all_bounds = any.then_some((lo, hi));
    }

    /// Composites all active `WaterBody` volumes onto `out` **after** solid edits & SDF elements:
    /// - Solid geometry (`cur.distance < 0`, non-water) always cuts water (`water_d = max(d_body, -solid_d)`),
    ///   so earth/stone dams and carved pond walls stay crisp and watertight.
    /// - Existing `Material::Water` from the base area unions smoothly with expanded basins.
    /// - Air (`cur.distance >= 0`) below `level` inside a basin fills with `Material::Water`, and
    ///   air above `level` receives accurate distance to the water surface.
    pub fn apply_batch(&self, points: &[[f32; 3]], out: &mut [Answer]) {
        let Some((alo, ahi)) = self.all_bounds else { return };
        if points.is_empty() {
            return;
        }
        let mut blo = [f32::INFINITY; 3];
        let mut bhi = [f32::NEG_INFINITY; 3];
        for p in points {
            for a in 0..3 {
                blo[a] = min_f32(blo[a], p[a]);
                bhi[a] = max_f32(bhi[a], p[a]);
            }
        }
        let batch_dist = aabb_to_aabb_distance(blo, bhi, alo, ahi);
        if batch_dist > 0.0 {
            let mut batch_max_d = f32::NEG_INFINITY;
            for ans in out.iter() {
                batch_max_d = max_f32(batch_max_d, ans.distance);
            }
            if batch_max_d <= batch_dist {
                return;
            }
        }

        for (body, b_opt) in self.bodies.iter().zip(&self.bounds) {
            let Some((wlo, whi)) = *b_opt else { continue };
            let bd = aabb_to_aabb_distance(blo, bhi, wlo, whi);
            if bd > 0.0 {
                let mut batch_max_d = f32::NEG_INFINITY;
                for ans in out.iter() {
                    batch_max_d = max_f32(batch_max_d, ans.distance);
                }
                if batch_max_d <= bd {
                    continue;
                }
            }
            for (&p, cur) in points.iter().zip(out.iter_mut()) {
                let pd = point_to_aabb_distance(p, wlo, whi);
                if pd > 0.0 && cur.distance <= pd {
                    continue;
                }
                let d_body = body.distance(p);
                if cur.material == Material::Water {
                    if d_body < cur.distance {
                        cur.distance = d_body;
                    }
                } else {
                    // Exact `water.cut(solid)`: never overwrites solid dirt/stone (`cur.distance < 0`),
                    // fills carved air cavities below `level` right up to the solid wall.
                    let d_water = max_f32(d_body, -cur.distance);
                    if d_water < cur.distance {
                        *cur = Answer { distance: d_water, material: Material::Water };
                    }
                }
            }
        }
    }

    /// Queries the hydrostatic water surface level at `(x, z)` from engine-managed `WaterBody`s,
    /// if `(x, y, z)` lies horizontally inside an active basin (used by buoyancy & connectivity).
    pub fn water_level_at(&self, p: [f32; 3]) -> Option<f32> {
        let mut best: Option<f32> = None;
        for (body, b_opt) in self.bodies.iter().zip(&self.bounds) {
            let Some((wlo, whi)) = *b_opt else { continue };
            if p[0] < wlo[0] || p[0] > whi[0] || p[2] < wlo[2] || p[2] > whi[2] {
                continue;
            }
            let probe = [p[0], min_f32(p[1], body.level - 0.01), p[2]];
            if body.distance(probe) <= 0.0 {
                best = Some(match best {
                    Some(cur) => max_f32(cur, body.level),
                    None => body.level,
                });
            }
        }
        best
    }

    /// Detects whether a newly applied `Op::Carve` edit connects to existing water (in `world_before`,
    /// which should include the base area + existing edits + existing `WaterLog`). If connected,
    /// floods `new_edit` AND any transitively overlapping earlier `Op::Carve` edits in `edits`
    /// below the detected water level, returning the world AABB `(lo, hi)` of newly flooded basins
    /// for targeted chunk invalidation.
    pub fn expand_from_carve(
        &mut self,
        world_before: &mut dyn Region,
        edits: &EditLog,
        new_edit: &Edit,
        author: Option<String>,
    ) -> Result<Option<([f32; 3], [f32; 3])>> {
        if new_edit.op != Op::Carve {
            return Ok(None);
        }
        let Some((level, seed_point)) = detect_connected_water(world_before, new_edit)? else {
            return Ok(None);
        };
        Ok(self.flood_from_seed(level, seed_point, edits, new_edit, author))
    }

    fn flood_from_seed(
        &mut self,
        level: f32,
        seed_point: [f32; 3],
        edits: &EditLog,
        new_edit: &Edit,
        author: Option<String>,
    ) -> Option<([f32; 3], [f32; 3])> {
        let (elo, _) = new_edit.influence_bounds();
        if elo[1] >= level {
            return None;
        }

        // Collect all `Op::Carve` edits transitively connected to `new_edit` below `level`,
        // so digging a dry basin first and then breaching the pond bank floods the entire basin!
        let all_edits = edits.edits();
        let mut visited = vec![false; all_edits.len()];
        let mut queue: VecDeque<Edit> = VecDeque::new();
        let mut connected: Vec<Edit> = Vec::new();
        queue.push_back(*new_edit);
        connected.push(*new_edit);

        while let Some(cur) = queue.pop_front() {
            let (clo, chi) = cur.influence_bounds();
            let cur_pad = max_f32(0.08 * cur.shape.radius(), 0.002);
            for (idx, candidate) in all_edits.iter().enumerate() {
                if visited[idx] || candidate.op != Op::Carve {
                    continue;
                }
                let (alo, ahi) = candidate.influence_bounds();
                if alo[1] >= level {
                    continue;
                }
                // Check overlap in 3D below `level`.
                let overlap_x = ahi[0] + cur_pad >= clo[0] && alo[0] - cur_pad <= chi[0];
                let overlap_z = ahi[2] + cur_pad >= clo[2] && alo[2] - cur_pad <= chi[2];
                let lo_y = max_f32(alo[1], clo[1]);
                let hi_y = min_f32(min_f32(ahi[1], chi[1]), level);
                if overlap_x && overlap_z && hi_y + cur_pad >= lo_y {
                    visited[idx] = true;
                    connected.push(*candidate);
                    queue.push_back(*candidate);
                }
            }
        }

        // Merge into an existing WaterBody only if it has the exact same hydrostatic level
        // (within 5 mm; bisection is sub-0.02 mm accurate), so distinct pools at different
        // elevations (e.g. a tiny grotto pool 9.8 cm above the main pond!) never merge or lift
        // one water body to another's height!
        let body_idx = self.bodies.iter().position(|b| (b.level - level).abs() <= 0.005);
        let target_idx = match body_idx {
            Some(i) => i,
            None => {
                let id = self.next_id();
                self.bodies.push(WaterBody { id, level, seed_point, basins: Vec::new(), author });
                self.bodies.len() - 1
            }
        };

        let target_level = self.bodies[target_idx].level;
        let mut flooded_lo = [f32::INFINITY; 3];
        let mut flooded_hi = [f32::NEG_INFINITY; 3];
        let mut added_any = false;

        for e in connected {
            let basin = WaterBasin::from_edit(&e);
            if !self.bodies[target_idx].basins.contains(&basin) {
                let (blo, bhi) = basin.bounds(target_level);
                if bhi[1] > blo[1] {
                    for a in 0..3 {
                        flooded_lo[a] = min_f32(flooded_lo[a], blo[a]);
                        flooded_hi[a] = max_f32(flooded_hi[a], bhi[a]);
                    }
                    self.bodies[target_idx].basins.push(basin);
                    added_any = true;
                }
            }
        }

        if added_any {
            self.revision += 1;
            self.rebuild_bounds();
            Some((flooded_lo, flooded_hi))
        } else {
            None
        }
    }

    /// Recomputes water expansions deterministically from scratch given `base_region` and an
    /// `EditLog` (used on undo/redo so undoing a pond breach cleanly recedes the water).
    pub fn recompute_from_edits(base_region: &mut dyn Region, edits: &EditLog) -> Result<WaterLog> {
        let mut water = WaterLog::new();
        let mut replay = EditLog::new();
        for &edit in edits.edits() {
            if edit.op == Op::Carve {
                let detected = {
                    let mut view = ReplayView { base: base_region, edits: &replay, water: &water };
                    detect_connected_water(&mut view, &edit)?
                };
                if let Some((level, seed_point)) = detected {
                    let _ = water.flood_from_seed(level, seed_point, &replay, &edit, None);
                }
            }
            replay.push(edit)?;
        }
        Ok(water)
    }

    /// Serializes all `water` and `water-basin` records for `WorldSave`.
    pub fn write_records(&self, out: &mut String) {
        for body in &self.bodies {
            out.push_str(&format!(
                "water {} level {} seed {} {} {}",
                body.id, body.level, body.seed_point[0], body.seed_point[1], body.seed_point[2]
            ));
            if let Some(author) = &body.author {
                out.push_str(&format!(" author {author}"));
            }
            out.push('\n');
            for b in &body.basins {
                let [cx, cy, cz] = b.center;
                match b.shape {
                    Shape::Sphere { radius } => {
                        out.push_str(&format!("water-basin {} sphere {cx} {cy} {cz} {radius} pad {}\n", body.id, b.pad));
                    }
                    Shape::Box { half: [hx, hy, hz], round } => {
                        out.push_str(&format!("water-basin {} box {cx} {cy} {cz} {hx} {hy} {hz} round {round} pad {}\n", body.id, b.pad));
                    }
                    Shape::Cylinder { radius, half_height, round } => {
                        out.push_str(&format!(
                            "water-basin {} cyl {cx} {cy} {cz} {radius} {half_height} round {round} pad {}\n",
                            body.id, b.pad
                        ));
                    }
                }
            }
        }
    }

    pub fn parse_water_line(line: &str, line_no: usize) -> Result<WaterBody> {
        let tok: Vec<&str> = line.split_whitespace().collect();
        if tok.len() != 8 && tok.len() != 10 {
            bail!("line {line_no}: expected 8 or 10 tokens in water record, got {}", tok.len());
        }
        let expect = |i: usize, kw: &str| -> Result<()> {
            if tok[i] == kw { Ok(()) } else { bail!("line {line_no}: expected `{kw}`, got `{}`", tok[i]) }
        };
        let pf = |i: usize| -> Result<f32> { tok[i].parse::<f32>().map_err(|e| wasmtime::format_err!("line {line_no}: bad float: {e}")) };
        let id: u32 = tok[1].parse().map_err(|e| wasmtime::format_err!("line {line_no}: bad water id: {e}"))?;
        expect(2, "level")?;
        let level = pf(3)?;
        expect(4, "seed")?;
        let seed_point = [pf(5)?, pf(6)?, pf(7)?];
        let author = if tok.len() == 10 {
            expect(8, "author")?;
            Some(tok[9].to_string())
        } else {
            None
        };
        let body = WaterBody { id, level, seed_point, basins: Vec::new(), author };
        body.validate()?;
        Ok(body)
    }

    pub fn parse_basin_line(line: &str, line_no: usize) -> Result<(u32, WaterBasin)> {
        let tok: Vec<&str> = line.split_whitespace().collect();
        let pf = |i: usize| -> Result<f32> {
            tok.get(i)
                .ok_or_else(|| wasmtime::format_err!("line {line_no}: unexpected end of water-basin record"))?
                .parse::<f32>()
                .map_err(|e| wasmtime::format_err!("line {line_no}: bad float: {e}"))
        };
        let expect = |i: usize, kw: &str| -> Result<()> {
            match tok.get(i) {
                Some(&s) if s == kw => Ok(()),
                other => bail!("line {line_no}: expected `{kw}`, got {other:?}"),
            }
        };
        let id: u32 = tok
            .get(1)
            .ok_or_else(|| wasmtime::format_err!("line {line_no}: missing water-basin id"))?
            .parse()
            .map_err(|e| wasmtime::format_err!("line {line_no}: bad water-basin id: {e}"))?;
        let kind = tok.get(2).copied().unwrap_or("");
        let (shape, center, tail) = match kind {
            "sphere" => (Shape::Sphere { radius: pf(6)? }, [pf(3)?, pf(4)?, pf(5)?], 7),
            "box" => {
                let c = [pf(3)?, pf(4)?, pf(5)?];
                let h = [pf(6)?, pf(7)?, pf(8)?];
                expect(9, "round")?;
                (Shape::Box { half: h, round: pf(10)? }, c, 11)
            }
            "cyl" => {
                let c = [pf(3)?, pf(4)?, pf(5)?];
                let r = pf(6)?;
                let hh = pf(7)?;
                expect(8, "round")?;
                (Shape::Cylinder { radius: r, half_height: hh, round: pf(9)? }, c, 10)
            }
            _ => bail!("line {line_no}: unknown water-basin shape `{kind}`"),
        };
        expect(tail, "pad")?;
        let pad = pf(tail + 1)?;
        if tok.len() != tail + 2 {
            bail!("line {line_no}: trailing tokens in water-basin record");
        }
        let basin = WaterBasin { shape, center, pad };
        basin.validate()?;
        Ok((id, basin))
    }
}

/// Probes deterministic vertical columns across and just outside `edit` to find if it touches
/// `Material::Water`, and if so refines the exact hydrostatic water surface `level`.
///
/// Uses 17 horizontal columns (center + 8 inner ring + 8 outer ring) × 16 vertical strata plus a
/// surface-transition pass at every air→solid crossing, so even a 0.5 mm razor-thin shoreline
/// meniscus on a gently sloping `ground * 0.65` beach is detected reliably at Tiny (1.7 cm),
/// Person (1.7 m), and Giant (17 m) scales.
fn detect_connected_water(region: &mut dyn Region, edit: &Edit) -> Result<Option<(f32, [f32; 3])>> {
    let (lo, hi) = edit.influence_bounds();
    let r = edit.shape.radius();
    let margin = max_f32(0.30 * r, 0.004);
    let plo = [lo[0] - margin, lo[1] - margin, lo[2] - margin];
    let phi = [hi[0] + margin, hi[1] + margin, hi[2] + margin];

    // 17 deterministic XZ columns covering center, inner ring (0.5x), and outer rim (1.0x).
    let cx = 0.5 * (plo[0] + phi[0]);
    let cz = 0.5 * (plo[2] + phi[2]);
    let rx = 0.5 * (phi[0] - plo[0]);
    let rz = 0.5 * (phi[2] - plo[2]);
    const DIAG: f32 = 0.707_106_77;
    const DIRS: [[f32; 2]; 8] = [
        [1.0, 0.0],
        [DIAG, DIAG],
        [0.0, 1.0],
        [-DIAG, DIAG],
        [-1.0, 0.0],
        [-DIAG, -DIAG],
        [0.0, -1.0],
        [DIAG, -DIAG],
    ];
    let mut cols = Vec::with_capacity(17);
    cols.push([cx, cz]);
    for &[dx, dz] in &DIRS {
        cols.push([cx + dx * rx * 0.5, cz + dz * rz * 0.5]);
    }
    for &[dx, dz] in &DIRS {
        cols.push([cx + dx * rx, cz + dz * rz]);
    }

    const NY: usize = 16;
    let mut probes = Vec::with_capacity(cols.len() * NY);
    for &[x, z] in &cols {
        for iy in 0..NY {
            let ty = iy as f32 / (NY - 1) as f32;
            let y = plo[1] + (phi[1] - plo[1]) * ty;
            probes.push([x, y, z]);
        }
    }
    let size = max_f32(edit.stamp_size, 0.017).min(17.0);
    let mut out = Vec::new();
    chunk::sample(region, &probes, size, &mut out)?;

    // 1. Pick the highest probe point strictly submerged inside `Material::Water` (`distance <= 0.0`).
    let mut best_water: Option<[f32; 3]> = None;
    for (&p, ans) in probes.iter().zip(&out) {
        if ans.material == Material::Water && ans.distance <= 0.0 && best_water.is_none_or(|b| p[1] > b[1]) {
            best_water = Some(p);
        }
    }

    // 2. Surface-transition & shallow-water fallback:
    //    - Any air probe seeing `Material::Water` directly below steps down by a tiny dip.
    //    - Any vertical air→solid transition straddling a paper-thin (< y_step) beach meniscus
    //      (where `ground * 0.65` can hide shallow water from the air probe above!) probes 3
    //      sub-intervals right inside the transition bracket!
    if best_water.is_none() {
        let y_step = (phi[1] - plo[1]) / (NY - 1) as f32;
        let reach_limit = edit.blend + margin;
        let dip_primary = min_f32(max_f32(0.08 * r, 0.0004), 0.015);
        let mut fine_pts: Vec<[f32; 3]> = Vec::new();

        for ci in 0..cols.len() {
            let base_i = ci * NY;
            for iy in 0..NY {
                let p = probes[base_i + iy];
                let ans = out[base_i + iy];
                if ans.material == Material::Water && ans.distance > 0.0 && ans.distance <= y_step * 1.5 {
                    for dip in [dip_primary, 0.0002] {
                        let sub = [p[0], p[1] - ans.distance - dip, p[2]];
                        if edit.distance(sub) <= reach_limit {
                            fine_pts.push(sub);
                        }
                    }
                }
                if iy + 1 < NY {
                    let ans_up = out[base_i + iy + 1];
                    if ans.distance <= 0.0 && ans_up.distance > 0.0 {
                        let y0 = p[1];
                        let y1 = probes[base_i + iy + 1][1];
                        for frac in [0.15f32, 0.35, 0.55, 0.75] {
                            let sub = [p[0], y0 + (y1 - y0) * frac, p[2]];
                            if edit.distance(sub) <= reach_limit {
                                fine_pts.push(sub);
                            }
                        }
                    }
                }
            }
        }

        if !fine_pts.is_empty() {
            let mut fine_out = Vec::new();
            chunk::sample(region, &fine_pts, size, &mut fine_out)?;
            for (&p, ans) in fine_pts.iter().zip(&fine_out) {
                if ans.material == Material::Water && ans.distance <= 0.0 && best_water.is_none_or(|b| p[1] > b[1]) {
                    best_water = Some(p);
                }
            }
            // One final micro-dip if a fine transition point landed just millimetres above water.
            if best_water.is_none() {
                let mut micro_pts = Vec::new();
                for (&p, ans) in fine_pts.iter().zip(&fine_out) {
                    if ans.material == Material::Water && ans.distance > 0.0 && ans.distance <= y_step {
                        let sub = [p[0], p[1] - ans.distance - 0.0002, p[2]];
                        if edit.distance(sub) <= reach_limit {
                            micro_pts.push(sub);
                        }
                    }
                }
                if !micro_pts.is_empty() {
                    let mut micro_out = Vec::new();
                    chunk::sample(region, &micro_pts, size, &mut micro_out)?;
                    for (&p, ans) in micro_pts.iter().zip(&micro_out) {
                        if ans.material == Material::Water && ans.distance <= 0.0 && best_water.is_none_or(|b| p[1] > b[1]) {
                            best_water = Some(p);
                        }
                    }
                }
            }
        }
    }
    let Some(seed) = best_water else {
        return Ok(None);
    };

    // Refine exact hydrostatic water surface height straight up from the submerged `seed`.
    let mut y_in = seed[1];
    let mut step = max_f32(0.001, r * 0.1).min(0.5);
    let mut y_out = y_in;
    for _ in 0..80 {
        let test_y = y_in + step;
        chunk::sample(region, &[[seed[0], test_y, seed[2]]], size, &mut out)?;
        if out[0].material == Material::Water && out[0].distance <= 0.0 {
            y_in = test_y;
            step = min_f32(step * 1.4, 1.0);
        } else {
            y_out = test_y;
            break;
        }
    }
    if y_out <= y_in {
        y_out = y_in + step;
    }
    // 16 bisection steps -> sub-0.02 mm water surface accuracy so expanded water matches old water seamlessly.
    for _ in 0..16 {
        let mid = 0.5 * (y_in + y_out);
        chunk::sample(region, &[[seed[0], mid, seed[2]]], size, &mut out)?;
        if out[0].material == Material::Water && out[0].distance <= 0.0 {
            y_in = mid;
        } else {
            y_out = mid;
        }
    }
    let level = 0.5 * (y_in + y_out);
    Ok(Some((level, [seed[0], level, seed[2]])))
}

/// Helper `Region` combining base + partial `EditLog` + `WaterLog` during deterministic replay.
struct ReplayView<'a> {
    base: &'a mut dyn Region,
    edits: &'a EditLog,
    water: &'a WaterLog,
}

impl Region for ReplayView<'_> {
    fn sample(&mut self, points: &[[f32; 3]], size: f32, out: &mut Vec<Answer>) -> Result<()> {
        self.base.sample(points, size, out)?;
        self.edits.apply_batch(points, out);
        self.water.apply_batch(points, out);
        Ok(())
    }

    fn describe(&mut self) -> Result<crate::Description> {
        self.base.describe()
    }
}

/// Checks whether a world AABB `(lo, hi)` can affect chunk `key` (including border collar & AO).
pub fn bounds_affect_chunk(lo: [f32; 3], hi: [f32; 3], key: chunk::Key) -> bool {
    let o = key.origin();
    let side = key.side();
    let pad = 2.0 * key.voxel() + 0.4 * key.size() as f64;
    for a in 0..3 {
        if (hi[a] as f64) < o[a] - pad || (lo[a] as f64) > o[a] + side + pad {
            return false;
        }
    }
    true
}

#[inline]
fn min_f32(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

#[inline]
fn max_f32(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

#[inline]
fn point_to_aabb_distance(p: [f32; 3], lo: [f32; 3], hi: [f32; 3]) -> f32 {
    let dx = max_f32(max_f32(lo[0] - p[0], 0.0), p[0] - hi[0]);
    let dy = max_f32(max_f32(lo[1] - p[1], 0.0), p[1] - hi[1]);
    let dz = max_f32(max_f32(lo[2] - p[2], 0.0), p[2] - hi[2]);
    (dx * dx + dy * dy + dz * dz).sqrt()
}

#[inline]
fn aabb_to_aabb_distance(alo: [f32; 3], ahi: [f32; 3], blo: [f32; 3], bhi: [f32; 3]) -> f32 {
    let dx = max_f32(max_f32(blo[0] - ahi[0], 0.0), alo[0] - bhi[0]);
    let dy = max_f32(max_f32(blo[1] - ahi[1], 0.0), alo[1] - bhi[1]);
    let dz = max_f32(max_f32(blo[2] - ahi[2], 0.0), alo[2] - bhi[2]);
    (dx * dx + dy * dy + dz * dz).sqrt()
}
