//! A sketched map of an area, from above (CONCEPT.md, "The sketch"; POC_PLAN.md, step 6): drawn
//! in any paint program on a white background with the seven pens of `sketches/plateau.gpl`, north
//! at the top (z = 0) and east to the right (x = 64).
//!
//! The engine classifies each pixel into its pen and reads:
//! - **Areas** (`Brown` land/ground, `Grey` rock/cliff, `Blue` water): connected regions with
//!   simplified polygon outlines in area metres.
//! - **Places** (`Orange` tiny only, `Purple` giants only): connected marks with centres, bounds
//!   and outlines, held to the law `laws::sketch_places`.
//! - **Arrows** (`Red`): paths traced from tail to arrowhead, in metres.
//! - **Words and things** (`Black`): words read with `tesseract` when installed (on the black-only
//!   mask, so coloured backgrounds never interfere), plus any drawn black marks outside them.

use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::{Size, WIDTH};

/// One pen of the sketch palette (`sketches/plateau.gpl`), or `None` for background.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pen {
    None,
    Brown,
    Grey,
    Blue,
    Black,
    Red,
    Orange,
    Purple,
}

impl Pen {
    pub fn name(self) -> &'static str {
        match self {
            Pen::None => "background",
            Pen::Brown => "brown",
            Pen::Grey => "grey",
            Pen::Blue => "blue",
            Pen::Black => "black",
            Pen::Red => "red",
            Pen::Orange => "orange",
            Pen::Purple => "purple",
        }
    }

    pub fn meaning(self) -> &'static str {
        match self {
            Pen::None => "unmarked",
            Pen::Brown => "land / ground",
            Pen::Grey => "rock / cliff",
            Pen::Blue => "water",
            Pen::Black => "thing / word",
            Pen::Red => "moving path (arrow)",
            Pen::Orange => "place for the tiny only",
            Pen::Purple => "place for giants only",
        }
    }
}

/// The seven pens of `sketches/plateau.gpl`, sRGB. Orange and purple match `picture::COLOURS`.
pub const PALETTE: [(Pen, [u8; 3], &str); 7] = [
    (Pen::Brown, [140, 90, 43], "Brown — land, ground"),
    (Pen::Grey, [130, 130, 130], "Grey — rock, cliff"),
    (Pen::Blue, [45, 125, 225], "Blue — water"),
    (Pen::Black, [0, 0, 0], "Black — a thing, with a word"),
    (Pen::Red, [220, 35, 35], "Red — an arrow (path of motion)"),
    (Pen::Orange, [245, 140, 20], "Orange — a place for the tiny only"),
    (Pen::Purple, [160, 70, 215], "Purple — a place for giants only"),
];

/// Classifies one sRGBA pixel into a sketch pen. Exact palette colours match directly; hand-picked
/// approximations in any paint program fall into the matching hue/value band.
pub fn classify(rgb: [u8; 3], alpha: u8) -> Pen {
    if alpha < 128 {
        return Pen::None;
    }
    let [r, g, b] = rgb.map(|c| c as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let v = max;
    let s = if max > 0.0 { delta / max } else { 0.0 };

    if v < 0.28 {
        return Pen::Black;
    }
    if s < 0.16 {
        return if v > 0.88 { Pen::None } else { Pen::Grey };
    }
    // Very pale anti-aliased fringe against white.
    if v > 0.88 && s < 0.30 {
        return Pen::None;
    }

    let mut h = if delta == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / delta) % 6.0)
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    if h < 0.0 {
        h += 360.0;
    }

    if (h >= 340.0 || h < 14.0) && s >= 0.45 && v >= 0.45 {
        return Pen::Red;
    }
    if (14.0..50.0).contains(&h) && s >= 0.60 && v >= 0.72 {
        return Pen::Orange;
    }
    if (10.0..60.0).contains(&h) {
        return Pen::Brown;
    }
    if (175.0..255.0).contains(&h) {
        return Pen::Blue;
    }
    if (255.0..340.0).contains(&h) {
        return Pen::Purple;
    }

    // Fall back to the nearest palette colour if close enough.
    let mut best = (f32::INFINITY, Pen::None);
    for (pen, c, _) in PALETTE {
        let d = (0..3).map(|i| (rgb[i] as f32 - c[i] as f32).powi(2)).sum::<f32>().sqrt();
        if d < best.0 {
            best = (d, pen);
        }
    }
    if best.0 < 95.0 { best.1 } else { Pen::None }
}

/// A painted area of land (Brown), rock/cliff (Grey) or water (Blue), in area metres.
#[derive(Clone, Debug)]
pub struct RegionShape {
    pub pen: Pen,
    /// Approximate area in square metres.
    pub area: f32,
    pub centre: [f32; 2],
    pub bounds: ([f32; 2], [f32; 2]),
    /// Simplified outer contour in area metres.
    pub outline: Vec<[f32; 2]>,
}

/// A place mark for one size: Orange (tiny) or Purple (giants).
#[derive(Clone, Debug)]
pub struct PlaceMark {
    pub size: Size,
    pub centre: [f32; 2],
    /// Equivalent radius (sqrt(area / pi)) in metres.
    pub radius: f32,
    pub bounds: ([f32; 2], [f32; 2]),
    pub outline: Vec<[f32; 2]>,
    /// Pixels of this mark (x, y), kept for exact distance queries in `laws::sketch_places`.
    pixels: Vec<(usize, usize)>,
}

impl PlaceMark {
    /// Minimum Euclidean distance in metres from `(x, z)` to this mark (0 if inside).
    pub fn distance_to(&self, sketch: &Sketch, x: f32, z: f32) -> f32 {
        let (hx, hz) = (0.5 * WIDTH / sketch.width as f32, 0.5 * WIDTH / sketch.height as f32);
        let mut best = f32::INFINITY;
        for &(i, j) in &self.pixels {
            let [px, pz] = sketch.pixel_centre(i, j);
            let dx = ((x - px).abs() - hx).max(0.0);
            let dz = ((z - pz).abs() - hz).max(0.0);
            let d = (dx * dx + dz * dz).sqrt();
            if d < best {
                best = d;
            }
        }
        best
    }
}

/// A red arrow: waypoints in metres from tail to arrowhead.
#[derive(Clone, Debug)]
pub struct Arrow {
    pub points: Vec<[f32; 2]>,
}

/// A word or phrase written in black on the sketch, with its position in area metres.
#[derive(Clone, Debug)]
pub struct Word {
    pub text: String,
    pub at: [f32; 2],
    pub bounds: ([f32; 2], [f32; 2]),
}

/// A black drawn mark (a thing, not part of a recognised word), in area metres.
#[derive(Clone, Debug)]
pub struct Mark {
    pub centre: [f32; 2],
    pub bounds: ([f32; 2], [f32; 2]),
    pub outline: Vec<[f32; 2]>,
}

/// A sketch map read by the engine.
#[derive(Clone, Debug)]
pub struct Sketch {
    pub width: usize,
    pub height: usize,
    pens: Vec<Pen>,
    /// Index into `regions` for each pixel that belongs to a kept area region (Brown, Grey, Blue).
    region_of: Vec<Option<usize>>,
    pub regions: Vec<RegionShape>,
    pub places: Vec<PlaceMark>,
    pub arrows: Vec<Arrow>,
    pub words: Vec<Word>,
    pub marks: Vec<Mark>,
}

impl Sketch {
    /// Opens and parses a PNG sketch from `path`.
    pub fn open(path: &Path) -> Result<Sketch, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Sketch::from_png(&bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Parses a PNG sketch from raw bytes.
    pub fn from_png(bytes: &[u8]) -> Result<Sketch, String> {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().map_err(|e| format!("cannot read PNG: {e}"))?;
        let mut buf = vec![0; reader.output_buffer_size().ok_or("PNG buffer size overflow")?];
        let info = reader.next_frame(&mut buf).map_err(|e| format!("cannot decode PNG: {e}"))?;
        let data = &buf[..info.buffer_size()];
        let (w, h) = (info.width as usize, info.height as usize);
        if w == 0 || h == 0 {
            return Err("empty PNG".into());
        }
        let rgba: Vec<[u8; 4]> = match info.color_type {
            png::ColorType::Rgba => data.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect(),
            png::ColorType::Rgb => data.chunks_exact(3).map(|c| [c[0], c[1], c[2], 255]).collect(),
            png::ColorType::GrayscaleAlpha => data.chunks_exact(2).map(|c| [c[0], c[0], c[0], c[1]]).collect(),
            png::ColorType::Grayscale => data.iter().map(|&g| [g, g, g, 255]).collect(),
            other => return Err(format!("unsupported PNG colour type: {other:?}")),
        };
        Ok(Sketch::from_rgba(w, h, &rgba))
    }

    /// Parses a sketch from sRGBA pixels, row by row from the top (north).
    pub fn from_rgba(width: usize, height: usize, rgba: &[[u8; 4]]) -> Sketch {
        assert_eq!(rgba.len(), width * height);
        let mut pens: Vec<Pen> = rgba.iter().map(|&[r, g, b, a]| classify([r, g, b], a)).collect();

        // Remove 1-2 px grey anti-aliased fringes where black strokes meet white background.
        let orig = pens.clone();
        for j in 0..height {
            for i in 0..width {
                if orig[j * width + i] != Pen::Grey {
                    continue;
                }
                let mut near_black = false;
                let mut near_none = false;
                for dj in -2isize..=2 {
                    for di in -2isize..=2 {
                        let (ni, nj) = (i as isize + di, j as isize + dj);
                        if ni >= 0 && nj >= 0 && ni < width as isize && nj < height as isize {
                            match orig[nj as usize * width + ni as usize] {
                                Pen::Black => near_black = true,
                                Pen::None => near_none = true,
                                _ => {}
                            }
                        }
                    }
                }
                if near_black && near_none {
                    pens[j * width + i] = Pen::None;
                }
            }
        }

        let px_area = (WIDTH / width as f32) * (WIDTH / height as f32);
        let min_region_area = 0.5f32; // ignore tiny anti-aliasing specks

        // 1. Area regions: Brown, Grey, Blue.
        let mut raw_regions: Vec<(RegionShape, Vec<(usize, usize)>)> = Vec::new();
        for pen in [Pen::Brown, Pen::Grey, Pen::Blue] {
            for comp in components(&pens, width, height, pen, false) {
                let area = comp.len() as f32 * px_area;
                if area < min_region_area || !has_interior(&comp, &pens, width, height, pen) {
                    // Clear stray thin anti-alias fringes from the grid so laws ignore them.
                    for &(i, j) in &comp {
                        pens[j * width + i] = Pen::None;
                    }
                    continue;
                }
                let (centre, bounds) = stats(&comp, width, height);
                let outline = simplify_closed(&trace_outer_contour(&comp, width, height), 0.75);
                raw_regions.push((RegionShape { pen, area, centre, bounds, outline }, comp));
            }
        }
        // Largest regions first within each pen.
        raw_regions.sort_by(|a, b| (a.0.pen as u8).cmp(&(b.0.pen as u8)).then(b.0.area.total_cmp(&a.0.area)));
        let mut region_of = vec![None; width * height];
        let mut regions = Vec::with_capacity(raw_regions.len());
        for (idx, (shape, comp)) in raw_regions.into_iter().enumerate() {
            for (i, j) in comp {
                region_of[j * width + i] = Some(idx);
            }
            regions.push(shape);
        }

        // 2. Place marks: Orange (Tiny) and Purple (Giant).
        let mut places = Vec::new();
        for (pen, size) in [(Pen::Orange, Size::Tiny), (Pen::Purple, Size::Giant)] {
            for comp in components(&pens, width, height, pen, true) {
                if comp.len() < 4 {
                    continue;
                }
                let area = comp.len() as f32 * px_area;
                let (centre, bounds) = stats(&comp, width, height);
                let radius = (area / std::f32::consts::PI).sqrt();
                let outline = simplify_closed(&trace_outer_contour(&comp, width, height), 0.35);
                places.push(PlaceMark { size, centre, radius, bounds, outline, pixels: comp });
            }
        }
        places.sort_by(|a, b| (a.size as u8).cmp(&(b.size as u8)).then(a.centre[0].total_cmp(&b.centre[0])));

        // 3. Red arrows.
        let mut arrows = Vec::new();
        for comp in components(&pens, width, height, Pen::Red, true) {
            if comp.len() < 8 {
                continue;
            }
            let path = trace_arrow(&comp, width, height);
            if path.len() >= 2 {
                arrows.push(Arrow { points: path });
            }
        }

        // 4. Black words (via tesseract when installed) and non-word black marks.
        let words = read_words(&pens, width, height);
        let mut marks = Vec::new();
        for comp in components(&pens, width, height, Pen::Black, true) {
            if comp.len() < 6 {
                continue;
            }
            let (centre, bounds) = stats(&comp, width, height);
            let in_word = words.iter().any(|w| {
                let pad = 0.6;
                bounds.0[0] >= w.bounds.0[0] - pad
                    && bounds.1[0] <= w.bounds.1[0] + pad
                    && bounds.0[1] >= w.bounds.0[1] - pad
                    && bounds.1[1] <= w.bounds.1[1] + pad
            });
            if in_word {
                continue;
            }
            let outline = simplify_closed(&trace_outer_contour(&comp, width, height), 0.4);
            marks.push(Mark { centre, bounds, outline });
        }

        Sketch { width, height, pens, region_of, regions, places, arrows, words, marks }
    }

    /// Centre of pixel `(i, j)` in area metres `(x, z)`.
    pub fn pixel_centre(&self, i: usize, j: usize) -> [f32; 2] {
        [(i as f32 + 0.5) / self.width as f32 * WIDTH, (j as f32 + 0.5) / self.height as f32 * WIDTH]
    }

    /// Classified pen at `(x, z)` in area metres (`Pen::None` if outside `[0, 64)`).
    pub fn pen_at(&self, x: f32, z: f32) -> Pen {
        if !(0.0..WIDTH).contains(&x) || !(0.0..WIDTH).contains(&z) {
            return Pen::None;
        }
        let i = ((x / WIDTH) * self.width as f32).floor().clamp(0.0, (self.width - 1) as f32) as usize;
        let j = ((z / WIDTH) * self.height as f32).floor().clamp(0.0, (self.height - 1) as f32) as usize;
        self.pens[j * self.width + i]
    }

    /// Index into `self.regions` at `(x, z)` in area metres, if inside a Brown, Grey or Blue region.
    pub fn region_at(&self, x: f32, z: f32) -> Option<usize> {
        if !(0.0..WIDTH).contains(&x) || !(0.0..WIDTH).contains(&z) {
            return None;
        }
        let i = ((x / WIDTH) * self.width as f32).floor().clamp(0.0, (self.width - 1) as f32) as usize;
        let j = ((z / WIDTH) * self.height as f32).floor().clamp(0.0, (self.height - 1) as f32) as usize;
        self.region_of[j * self.width + i]
    }

    /// Formats the sketch's extracted shapes, places, arrows, words and marks in area metres
    /// (for Claude alongside the sketch PNG).
    pub fn describe(&self) -> String {
        let mut s = String::new();
        if !self.regions.is_empty() {
            s += "### Areas (metres: x west→east 0..64, z north→south 0..64)\n\n";
            for r in &self.regions {
                let pts: Vec<String> = r.outline.iter().map(|p| format!("({:.1}, {:.1})", p[0], p[1])).collect();
                s += &format!(
                    "- **{}** ({}): ~{:.0} m², centre ({:.1}, {:.1}), bounds x=[{:.1}, {:.1}] z=[{:.1}, {:.1}], outline: {}\n",
                    r.pen.name(),
                    r.pen.meaning(),
                    r.area,
                    r.centre[0],
                    r.centre[1],
                    r.bounds.0[0],
                    r.bounds.1[0],
                    r.bounds.0[1],
                    r.bounds.1[1],
                    pts.join(" → ")
                );
            }
            s += "\n";
        }
        if !self.places.is_empty() {
            s += "### Places (orange = tiny only, purple = giants only)\n\n";
            for p in &self.places {
                let colour = if p.size == Size::Tiny { "orange (tiny only)" } else { "purple (giants only)" };
                s += &format!(
                    "- **{colour}**: centre ({:.1}, {:.1}), radius ~{:.1} m, bounds x=[{:.1}, {:.1}] z=[{:.1}, {:.1}]\n",
                    p.centre[0],
                    p.centre[1],
                    p.radius,
                    p.bounds.0[0],
                    p.bounds.1[0],
                    p.bounds.0[1],
                    p.bounds.1[1],
                );
            }
            s += "\n";
        }
        if !self.words.is_empty() {
            s += "### Words (black labels)\n\n";
            for w in &self.words {
                s += &format!(
                    "- **\"{}\"** at ({:.1}, {:.1}) (box x=[{:.1}, {:.1}] z=[{:.1}, {:.1}])\n",
                    w.text, w.at[0], w.at[1], w.bounds.0[0], w.bounds.1[0], w.bounds.0[1], w.bounds.1[1]
                );
            }
            s += "\n";
        }
        if !self.marks.is_empty() {
            s += "### Drawn marks (black)\n\n";
            for m in &self.marks {
                s += &format!(
                    "- mark at ({:.1}, {:.1}), bounds x=[{:.1}, {:.1}] z=[{:.1}, {:.1}]\n",
                    m.centre[0], m.centre[1], m.bounds.0[0], m.bounds.1[0], m.bounds.0[1], m.bounds.1[1]
                );
            }
            s += "\n";
        }
        if !self.arrows.is_empty() {
            s += "### Arrows (red paths, tail → head)\n\n";
            for a in &self.arrows {
                let pts: Vec<String> = a.points.iter().map(|p| format!("({:.1}, {:.1})", p[0], p[1])).collect();
                s += &format!("- {}\n", pts.join(" → "));
            }
            s += "\n";
        }
        s
    }
}

/// Connected components of `pen` in `pens` (4-connected when `diag` is false, 8-connected when true).
fn components(pens: &[Pen], w: usize, h: usize, pen: Pen, diag: bool) -> Vec<Vec<(usize, usize)>> {
    let mut seen = vec![false; w * h];
    let mut out = Vec::new();
    let mut queue = VecDeque::new();
    let dirs4: [(isize, isize); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];
    let dirs8: [(isize, isize); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];
    for j in 0..h {
        for i in 0..w {
            let idx = j * w + i;
            if seen[idx] || pens[idx] != pen {
                continue;
            }
            seen[idx] = true;
            queue.push_back((i, j));
            let mut comp = Vec::new();
            while let Some((x, y)) = queue.pop_front() {
                comp.push((x, y));
                let dirs: &[(isize, isize)] = if diag { &dirs8 } else { &dirs4 };
                for &(dx, dy) in dirs {
                    let (nx, ny) = (x as isize + dx, y as isize + dy);
                    if nx >= 0 && ny >= 0 && nx < w as isize && ny < h as isize {
                        let (nx, ny) = (nx as usize, ny as usize);
                        let nidx = ny * w + nx;
                        if !seen[nidx] && pens[nidx] == pen {
                            seen[nidx] = true;
                            queue.push_back((nx, ny));
                        }
                    }
                }
            }
            out.push(comp);
        }
    }
    out
}

/// Whether at least one pixel in `comp` has all four cardinal neighbours also of `pen`
/// (filters out 1-pixel anti-aliased lines masquerading as area regions).
fn has_interior(comp: &[(usize, usize)], pens: &[Pen], w: usize, h: usize, pen: Pen) -> bool {
    comp.iter().any(|&(i, j)| {
        i > 0
            && j > 0
            && i + 1 < w
            && j + 1 < h
            && pens[j * w + (i - 1)] == pen
            && pens[j * w + (i + 1)] == pen
            && pens[(j - 1) * w + i] == pen
            && pens[(j + 1) * w + i] == pen
    })
}

/// Centroid and bounding box of a pixel component, in area metres.
fn stats(comp: &[(usize, usize)], w: usize, h: usize) -> ([f32; 2], ([f32; 2], [f32; 2])) {
    let (mut sx, mut sz) = (0.0f64, 0.0f64);
    let (mut min_x, mut min_z) = (f32::INFINITY, f32::INFINITY);
    let (mut max_x, mut max_z) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    let (kx, kz) = (WIDTH / w as f32, WIDTH / h as f32);
    for &(i, j) in comp {
        let (x, z) = ((i as f32 + 0.5) * kx, (j as f32 + 0.5) * kz);
        sx += x as f64;
        sz += z as f64;
        min_x = min_x.min(i as f32 * kx);
        max_x = max_x.max((i + 1) as f32 * kx);
        min_z = min_z.min(j as f32 * kz);
        max_z = max_z.max((j + 1) as f32 * kz);
    }
    let n = comp.len().max(1) as f64;
    ([(sx / n) as f32, (sz / n) as f32], ([min_x, min_z], [max_x, max_z]))
}

/// Outer boundary contour of a pixel component, in area metres (largest enclosed loop among the
/// crack-followed pixel-edge boundaries, ignoring interior holes).
fn trace_outer_contour(comp: &[(usize, usize)], w: usize, h: usize) -> Vec<[f32; 2]> {
    let mut mask = vec![false; w * h];
    for &(i, j) in comp {
        mask[j * w + i] = true;
    }
    // Directed boundary edges around the component (clockwise around solid in screen coords).
    let mut adj: HashMap<(usize, usize), Vec<(usize, usize)>> = HashMap::new();
    for &(i, j) in comp {
        if j == 0 || !mask[(j - 1) * w + i] {
            adj.entry((i, j)).or_default().push((i + 1, j));
        }
        if i + 1 == w || !mask[j * w + (i + 1)] {
            adj.entry((i + 1, j)).or_default().push((i + 1, j + 1));
        }
        if j + 1 == h || !mask[(j + 1) * w + i] {
            adj.entry((i + 1, j + 1)).or_default().push((i, j + 1));
        }
        if i == 0 || !mask[j * w + (i - 1)] {
            adj.entry((i, j + 1)).or_default().push((i, j));
        }
    }

    let mut best_loop: Vec<(usize, usize)> = Vec::new();
    let mut best_area = -1.0f64;
    let keys: Vec<(usize, usize)> = adj.keys().copied().collect();
    for start in keys {
        while let Some(first_next) = adj.get_mut(&start).and_then(|v| v.pop()) {
            let mut ring = vec![start];
            let mut prev = start;
            let mut cur = first_next;
            while cur != start {
                ring.push(cur);
                let Some(nexts) = adj.get_mut(&cur) else { break };
                if nexts.is_empty() {
                    break;
                }
                // If two edges leave a checkerboard vertex, prefer turning right to keep the outer loop.
                let pick = if nexts.len() == 1 {
                    0
                } else {
                    let dx = cur.0 as isize - prev.0 as isize;
                    let dy = cur.1 as isize - prev.1 as isize;
                    // Right turn in screen coords (x right, y down): (dx, dy) -> (-dy, dx).
                    nexts
                        .iter()
                        .position(|&(nx, ny)| nx as isize - cur.0 as isize == -dy && ny as isize - cur.1 as isize == dx)
                        .unwrap_or(0)
                };
                let next = nexts.swap_remove(pick);
                prev = cur;
                cur = next;
            }
            // Shoelace signed area (absolute value).
            let mut area = 0.0f64;
            for k in 0..ring.len() {
                let a = ring[k];
                let b = ring[(k + 1) % ring.len()];
                area += a.0 as f64 * b.1 as f64 - b.0 as f64 * a.1 as f64;
            }
            let area = area.abs() * 0.5;
            if area > best_area {
                best_area = area;
                best_loop = ring;
            }
        }
    }

    let (kx, kz) = (WIDTH / w as f32, WIDTH / h as f32);
    best_loop.into_iter().map(|(u, v)| [u as f32 * kx, v as f32 * kz]).collect()
}

/// Ramer-Douglas-Peucker simplification for an open polyline.
fn simplify_open(pts: &[[f32; 2]], eps: f32) -> Vec<[f32; 2]> {
    if pts.len() <= 2 {
        return pts.to_vec();
    }
    let a = pts[0];
    let b = *pts.last().unwrap();
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dz * dz;
    let mut best = (0.0f32, 0usize);
    for (i, &p) in pts.iter().enumerate().skip(1).take(pts.len() - 2) {
        let d = if len2 < 1e-9 {
            ((p[0] - a[0]).powi(2) + (p[1] - a[1]).powi(2)).sqrt()
        } else {
            let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / len2).clamp(0.0, 1.0);
            let qx = a[0] + t * dx;
            let qz = a[1] + t * dz;
            ((p[0] - qx).powi(2) + (p[1] - qz).powi(2)).sqrt()
        };
        if d > best.0 {
            best = (d, i);
        }
    }
    if best.0 > eps {
        let mut left = simplify_open(&pts[..=best.1], eps);
        let right = simplify_open(&pts[best.1..], eps);
        left.pop();
        left.extend(right);
        left
    } else {
        vec![a, b]
    }
}

/// Simplifies a closed polygon loop by splitting at its two farthest vertices.
fn simplify_closed(pts: &[[f32; 2]], eps: f32) -> Vec<[f32; 2]> {
    if pts.len() <= 4 {
        return pts.to_vec();
    }
    // Find vertex farthest from pts[0], then vertex farthest from that.
    let dist2 = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2);
    let i0 = (0..pts.len()).max_by(|&a, &b| dist2(pts[0], pts[a]).total_cmp(&dist2(pts[0], pts[b]))).unwrap_or(0);
    let i1 = (0..pts.len()).max_by(|&a, &b| dist2(pts[i0], pts[a]).total_cmp(&dist2(pts[i0], pts[b]))).unwrap_or(0);
    if i0 == i1 {
        return vec![pts[0]];
    }
    let (lo, hi) = if i0 < i1 { (i0, i1) } else { (i1, i0) };
    let first = &pts[lo..=hi];
    let mut second: Vec<[f32; 2]> = pts[hi..].to_vec();
    second.extend_from_slice(&pts[..=lo]);
    let mut out = simplify_open(first, eps);
    let mut back = simplify_open(&second, eps);
    out.pop();
    back.pop();
    out.extend(back);
    out
}

/// Traces a red arrow component into waypoints from tail to arrowhead, in area metres.
fn trace_arrow(comp: &[(usize, usize)], w: usize, h: usize) -> Vec<[f32; 2]> {
    let mut index: HashMap<(usize, usize), usize> = HashMap::with_capacity(comp.len());
    for (k, &p) in comp.iter().enumerate() {
        index.insert(p, k);
    }
    let bfs = |start: usize| -> (usize, Vec<usize>, Vec<u32>) {
        let mut dist = vec![u32::MAX; comp.len()];
        let mut parent = vec![usize::MAX; comp.len()];
        let mut q = VecDeque::new();
        dist[start] = 0;
        q.push_back(start);
        let mut far = start;
        let dirs: [(isize, isize); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];
        while let Some(u) = q.pop_front() {
            far = u;
            let (x, y) = comp[u];
            for (dx, dy) in dirs {
                let (nx, ny) = (x as isize + dx, y as isize + dy);
                if nx >= 0
                    && ny >= 0
                    && let Some(&v) = index.get(&(nx as usize, ny as usize))
                    && dist[v] == u32::MAX
                {
                    dist[v] = dist[u] + 1;
                    parent[v] = u;
                    q.push_back(v);
                }
            }
        }
        (far, parent, dist)
    };
    let (a, _, _) = bfs(0);
    let (b, parent, dist) = bfs(a);
    let mut path = Vec::new();
    let mut cur = b;
    while cur != usize::MAX {
        path.push(cur);
        cur = parent[cur];
    }
    path.reverse();
    if path.len() < 2 {
        return Vec::new();
    }

    // Decide which end is the arrowhead: the head has more pixels (barbs or filled wedge) within
    // a neighbourhood of `r` pixels than the tail does.
    let len_px = dist[b].max(4) as f32;
    let r = (len_px * 0.22).clamp(4.0, 40.0);
    let r2 = r * r;
    let mass_near = |k: usize| -> usize {
        let (x0, y0) = comp[k];
        comp.iter()
            .filter(|&&(x, y)| {
                let (dx, dy) = (x as f32 - x0 as f32, y as f32 - y0 as f32);
                dx * dx + dy * dy <= r2
            })
            .count()
    };
    if mass_near(a) > mass_near(b) {
        path.reverse();
    }

    // If the geodesic endpoint at the head landed on a barb tip rather than the arrow's apex,
    // advance along the shaft's forward direction to the farthest pixel near the head.
    let n = path.len();
    if n >= 6 {
        let p_back = comp[path[n * 2 / 3]];
        let p_mid = comp[path[n * 5 / 6]];
        let (mut fx, mut fy) = (p_mid.0 as f32 - p_back.0 as f32, p_mid.1 as f32 - p_back.1 as f32);
        let fl = (fx * fx + fy * fy).sqrt();
        if fl > 1e-3 {
            fx /= fl;
            fy /= fl;
            let head = comp[*path.last().unwrap()];
            let mut best = (f32::NEG_INFINITY, head);
            for &(x, y) in comp {
                let (dx, dy) = (x as f32 - head.0 as f32, y as f32 - head.1 as f32);
                if dx * dx + dy * dy <= r2 * 1.5 {
                    let proj = (x as f32 - p_back.0 as f32) * fx + (y as f32 - p_back.1 as f32) * fy;
                    if proj > best.0 {
                        best = (proj, (x, y));
                    }
                }
            }
            if best.1 != head {
                // Trim back to the last path point closer to p_back than the apex, then append apex.
                let apex_k = index[&best.1];
                let head_proj = (head.0 as f32 - p_back.0 as f32) * fx + (head.1 as f32 - p_back.1 as f32) * fy;
                if best.0 > head_proj + 1.5 {
                    // Trim the barb backtrack: keep up to where projection starts dropping.
                    let mut peak = n - 1;
                    let mut max_p = f32::NEG_INFINITY;
                    for (idx, &k) in path.iter().enumerate().skip(n / 2) {
                        let c = comp[k];
                        let pr = (c.0 as f32 - p_back.0 as f32) * fx + (c.1 as f32 - p_back.1 as f32) * fy;
                        if pr > max_p {
                            max_p = pr;
                            peak = idx;
                        }
                    }
                    path.truncate(peak + 1);
                    if *path.last().unwrap() != apex_k {
                        path.push(apex_k);
                    }
                }
            }
        }
    }

    let (kx, kz) = (WIDTH / w as f32, WIDTH / h as f32);
    let metres: Vec<[f32; 2]> = path.into_iter().map(|k| [(comp[k].0 as f32 + 0.5) * kx, (comp[k].1 as f32 + 0.5) * kz]).collect();
    simplify_open(&metres, 0.5)
}

/// Reads words written in black from the black-only mask using `tesseract`, if installed.
/// Returns an empty list if there are no black pixels or `tesseract` is not available.
fn read_words(pens: &[Pen], w: usize, h: usize) -> Vec<Word> {
    if !pens.iter().any(|&p| p == Pen::Black) {
        return Vec::new();
    }
    // Upscale small images 2x with a white border so small paint-program text reads cleanly.
    let scale = if w < 800 && h < 800 { 2usize } else { 1usize };
    let pad = 20usize;
    let (ow, oh) = (w * scale + 2 * pad, h * scale + 2 * pad);
    let mut gray = vec![255u8; ow * oh];
    for j in 0..h {
        for i in 0..w {
            if pens[j * w + i] == Pen::Black {
                for sy in 0..scale {
                    for sx in 0..scale {
                        gray[(j * scale + sy + pad) * ow + (i * scale + sx + pad)] = 0;
                    }
                }
            }
        }
    }

    let mut png_bytes = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png_bytes, ow as u32, oh as u32);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Eight);
        let Ok(mut writer) = enc.write_header() else { return Vec::new() };
        if writer.write_image_data(&gray).is_err() {
            return Vec::new();
        }
    }

    let Ok(mut child) = Command::new("tesseract")
        .args(["stdin", "stdout", "-l", "eng", "--psm", "11", "tsv"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return Vec::new();
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(&png_bytes);
    }
    let Ok(output) = child.wait_with_output() else { return Vec::new() };
    if !output.status.success() {
        return Vec::new();
    }

    // Group word tokens by (block, par, line) into phrases.
    let mut lines: HashMap<(u32, u32, u32), Vec<(f32, f32, f32, f32, String)>> = HashMap::new();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let (kx, kz) = (WIDTH / (w * scale) as f32, WIDTH / (h * scale) as f32);
    for row in stdout.lines().skip(1) {
        let cols: Vec<&str> = row.split('\t').collect();
        if cols.len() < 12 || cols[0] != "5" {
            continue;
        }
        let conf: f32 = cols[10].parse().unwrap_or(-1.0);
        let text = cols[11].trim();
        if conf < 30.0 || text.is_empty() || !text.chars().any(|c| c.is_alphanumeric()) {
            continue;
        }
        let Ok(block) = cols[2].parse::<u32>() else { continue };
        let Ok(par) = cols[3].parse::<u32>() else { continue };
        let Ok(line) = cols[4].parse::<u32>() else { continue };
        let Ok(left) = cols[6].parse::<f32>() else { continue };
        let Ok(top) = cols[7].parse::<f32>() else { continue };
        let Ok(bw) = cols[8].parse::<f32>() else { continue };
        let Ok(bh) = cols[9].parse::<f32>() else { continue };
        let x0 = ((left - pad as f32) * kx).clamp(0.0, WIDTH);
        let z0 = ((top - pad as f32) * kz).clamp(0.0, WIDTH);
        let x1 = ((left + bw - pad as f32) * kx).clamp(0.0, WIDTH);
        let z1 = ((top + bh - pad as f32) * kz).clamp(0.0, WIDTH);
        lines.entry((block, par, line)).or_default().push((x0, z0, x1, z1, text.to_string()));
    }

    let mut words = Vec::new();
    for (_, mut parts) in lines {
        parts.sort_by(|a, b| a.0.total_cmp(&b.0));
        let text = parts.iter().map(|p| p.4.as_str()).collect::<Vec<_>>().join(" ");
        // Drop lone single-character detections (usually drawn circles/lines misread as O or |).
        if text.chars().filter(|c| c.is_alphanumeric()).count() < 2 {
            continue;
        }
        let x0 = parts.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
        let z0 = parts.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
        let x1 = parts.iter().map(|p| p.2).fold(f32::NEG_INFINITY, f32::max);
        let z1 = parts.iter().map(|p| p.3).fold(f32::NEG_INFINITY, f32::max);
        words.push(Word { text, at: [(x0 + x1) * 0.5, (z0 + z1) * 0.5], bounds: ([x0, z0], [x1, z1]) });
    }
    words.sort_by(|a, b| a.at[1].total_cmp(&b.at[1]).then(a.at[0].total_cmp(&b.at[0])));
    words
}
