//! The sketch (POC_PLAN.md, step 6): reading a sketched map's areas, places, arrows and words,
//! matching the repo's palette file, and holding areas to the sketch's two laws (`sketch_materials`
//! and `sketch_places`).

#[allow(dead_code)]
mod played;

use std::path::Path;

use plateau_engine::reach::World;
use plateau_engine::sketch::{PALETTE, Pen, Sketch, classify};
use plateau_engine::{Region, Size, laws};
use plateau_sdk::{Answer, Area, Material, Place, Point, View, cuboid, cut, point, union};
use played::Native;

/// A simple in-memory canvas in area metres (0..64) on a white background, for drawing test
/// sketches cleanly.
struct Canvas {
    w: usize,
    h: usize,
    rgba: Vec<[u8; 4]>,
}

impl Canvas {
    fn new(w: usize, h: usize) -> Self {
        Canvas { w, h, rgba: vec![[255, 255, 255, 255]; w * h] }
    }

    fn rgb(pen: Pen) -> [u8; 4] {
        for (p, c, _) in PALETTE {
            if p == pen {
                return [c[0], c[1], c[2], 255];
            }
        }
        [255, 255, 255, 255]
    }

    fn set_px(&mut self, i: isize, j: isize, c: [u8; 4]) {
        if i >= 0 && j >= 0 && i < self.w as isize && j < self.h as isize {
            self.rgba[j as usize * self.w + i as usize] = c;
        }
    }

    fn fill_rect(&mut self, x0: f32, z0: f32, x1: f32, z1: f32, pen: Pen) {
        let c = Self::rgb(pen);
        let (kx, kz) = (self.w as f32 / 64.0, self.h as f32 / 64.0);
        for j in (z0 * kz).floor() as isize..(z1 * kz).ceil() as isize {
            for i in (x0 * kx).floor() as isize..(x1 * kx).ceil() as isize {
                self.set_px(i, j, c);
            }
        }
    }

    fn fill_circle(&mut self, cx: f32, cz: f32, r: f32, pen: Pen) {
        let c = Self::rgb(pen);
        let (kx, kz) = (self.w as f32 / 64.0, self.h as f32 / 64.0);
        for j in ((cz - r) * kz).floor() as isize..=((cz + r) * kz).ceil() as isize {
            for i in ((cx - r) * kx).floor() as isize..=((cx + r) * kx).ceil() as isize {
                let x = (i as f32 + 0.5) / kx;
                let z = (j as f32 + 0.5) / kz;
                if (x - cx).powi(2) + (z - cz).powi(2) <= r * r {
                    self.set_px(i, j, c);
                }
            }
        }
    }

    fn draw_line(&mut self, x0: f32, z0: f32, x1: f32, z1: f32, thick_px: isize, pen: Pen) {
        let c = Self::rgb(pen);
        let (kx, kz) = (self.w as f32 / 64.0, self.h as f32 / 64.0);
        let (ax, az, bx, bz) = (x0 * kx, z0 * kz, x1 * kx, z1 * kz);
        let steps = ((bx - ax).hypot(bz - az).ceil() as usize).max(1) * 2;
        for s in 0..=steps {
            let t = s as f32 / steps as f32;
            let (px, pz) = ((ax + (bx - ax) * t).round() as isize, (az + (bz - az) * t).round() as isize);
            for dj in -thick_px..=thick_px {
                for di in -thick_px..=thick_px {
                    if di * di + dj * dj <= thick_px * thick_px {
                        self.set_px(px + di, pz + dj, c);
                    }
                }
            }
        }
    }

    fn to_sketch(&self) -> Sketch {
        Sketch::from_rgba(self.w, self.h, &self.rgba)
    }

    fn to_png(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut enc = png::Encoder::new(&mut bytes, self.w as u32, self.h as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(self.rgba.as_flattened()).unwrap();
        bytes
    }
}

#[test]
fn the_palette_file_matches_every_pen() {
    let gpl = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../sketches/plateau.gpl")).unwrap();
    let mut found = Vec::new();
    for line in gpl.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("GIMP") || line.starts_with("Name:") || line.starts_with("Columns:") {
            continue;
        }
        let nums: Vec<u8> = line.split_whitespace().take(3).map(|s| s.parse().unwrap()).collect();
        let rgb = [nums[0], nums[1], nums[2]];
        found.push((classify(rgb, 255), rgb));
    }
    assert_eq!(found.len(), PALETTE.len());
    for ((pen, rgb), &(want_pen, want_rgb, _)) in found.iter().zip(&PALETTE) {
        assert_eq!(*pen, want_pen, "palette colour {rgb:?} classified wrong");
        assert_eq!(*rgb, want_rgb);
    }
    assert_eq!(classify([255, 255, 255], 255), Pen::None);
    assert_eq!(classify([45, 125, 225], 0), Pen::None);
}

#[test]
fn reads_areas_places_and_arrows_with_direction_and_round_trips_png() {
    let mut c = Canvas::new(256, 256);
    // South half brown land, north half blue water, a grey rocky cliff band along the coast.
    c.fill_rect(0.0, 0.0, 64.0, 26.0, Pen::Blue);
    c.fill_rect(0.0, 26.0, 64.0, 32.0, Pen::Grey);
    c.fill_rect(0.0, 32.0, 64.0, 64.0, Pen::Brown);
    // An orange tiny place in the rock, and a purple giant place in the sea.
    c.fill_circle(40.0, 29.0, 2.0, Pen::Orange);
    c.fill_circle(20.0, 14.0, 2.5, Pen::Purple);
    // A red arrow pointing east from (10, 10) to (46, 10) with two barbs at the east head.
    c.draw_line(10.0, 10.0, 46.0, 10.0, 2, Pen::Red);
    c.draw_line(46.0, 10.0, 40.0, 6.0, 2, Pen::Red);
    c.draw_line(46.0, 10.0, 40.0, 14.0, 2, Pen::Red);

    let png_bytes = c.to_png();
    let s = Sketch::from_png(&png_bytes).unwrap();

    assert!(s.regions.iter().any(|r| r.pen == Pen::Blue && r.area > 1400.0), "{:#?}", s.regions);
    assert!(s.regions.iter().any(|r| r.pen == Pen::Grey && r.area > 300.0), "{:#?}", s.regions);
    assert!(s.regions.iter().any(|r| r.pen == Pen::Brown && r.area > 1800.0), "{:#?}", s.regions);

    let tiny = s.places.iter().find(|p| p.size == Size::Tiny).expect("orange tiny place");
    assert!((tiny.centre[0] - 40.0).abs() < 1.0 && (tiny.centre[1] - 29.0).abs() < 1.0, "{tiny:?}");
    assert_eq!(tiny.distance_to(&s, 40.0, 29.0), 0.0);

    let giant = s.places.iter().find(|p| p.size == Size::Giant).expect("purple giant place");
    assert!((giant.centre[0] - 20.0).abs() < 1.0 && (giant.centre[1] - 14.0).abs() < 1.0, "{giant:?}");

    assert_eq!(s.arrows.len(), 1, "{:?}", s.arrows);
    let arrow = &s.arrows[0];
    let tail = arrow.points[0];
    let head = *arrow.points.last().unwrap();
    assert!((tail[0] - 10.0).abs() < 2.0 && (tail[1] - 10.0).abs() < 2.0, "tail: {tail:?}");
    assert!((head[0] - 46.0).abs() < 2.5 && (head[1] - 10.0).abs() < 2.0, "head: {head:?}");

    let desc = s.describe();
    assert!(desc.contains("**blue**") && desc.contains("**grey**") && desc.contains("**brown**"), "{desc}");
    assert!(desc.contains("orange (tiny only)") && desc.contains("purple (giants only)"), "{desc}");
    assert!(desc.contains("Arrows"), "{desc}");
}

#[test]
fn anti_aliased_black_text_on_white_is_not_mistaken_for_grey_rock() {
    let mut c = Canvas::new(128, 128);
    // Simulate an anti-aliased black stroke on white: black core with a 1-px grey fringe.
    c.fill_rect(18.0, 18.0, 34.0, 22.0, Pen::Grey);
    c.fill_rect(19.0, 19.0, 33.0, 21.0, Pen::Black);
    let s = c.to_sketch();
    assert!(s.regions.iter().all(|r| r.pen != Pen::Grey), "AA fringe became grey rock: {:?}", s.regions);
}

// A small test area matching the sketch below, made wrong one way at a time:
// - North (z < 24): water (y = 0) with a stone sea stack at (20, 14) rising to y = 3.5 (for giants).
// - Middle (24 <= z < 32): stone cliff band (y = 0.5) with a tiny-only room at (40, 28) behind a 1.6 cm slot.
// - South (z >= 32): grass land (y = 0).

const GOOD: u8 = 0;
const NO_WATER: u8 = 1;
const GRASS_CLIFF: u8 = 2;
const FLOODED_LAND: u8 = 3;
const MISSING_TINY_PLACE: u8 = 4;
const OPEN_TINY_PLACE: u8 = 5;
const LOW_STACK: u8 = 6;

struct Coast<const KIND: u8>;

impl<const KIND: u8> Area for Coast<KIND> {
    fn new(_: u64) -> Self {
        Coast
    }

    fn describe(&self) -> plateau_sdk::Description {
        let mut places = Vec::new();
        if KIND != MISSING_TINY_PLACE {
            places.push(Place::new("the smugglers' crack", plateau_sdk::Size::Tiny, point(40.0, 0.5, 28.0)));
        }
        let stack_h = if KIND == LOW_STACK { 0.15 } else { 3.5 };
        places.push(Place::new("the sea stack", plateau_sdk::Size::Giant, point(20.0, stack_h, 14.0)));
        plateau_sdk::Description {
            name: "Coast".into(),
            places,
            borders: plateau_sdk::Borders::of_ground(|_, _| 0.0),
        }
    }

    fn sample(&self, p: Point, _view: &View) -> Answer {
        let is_water = (KIND == FLOODED_LAND && p.z > 32.0) || (KIND != NO_WATER && p.z < 24.0);
        let mut ground = if is_water {
            // Shallow sea over a sandy seabed at y = -0.3, so a giant (and a person for LOW_STACK)
            // can wade across.
            let water = Answer::new((p.y - 0.0) * 0.7, Material::Water);
            let bed = Answer::new((p.y + 0.3) * 0.7, Material::Sand);
            if p.y < -0.15 { bed } else { water }
        } else {
            let mat = if p.z >= 24.0 && p.z < 32.0 {
                if KIND == GRASS_CLIFF { Material::Grass } else { Material::Stone }
            } else {
                Material::Grass
            };
            let ground_y = if p.z >= 24.0 && p.z < 32.0 { 0.5 } else { 0.0 };
            Answer::new((p.y - ground_y) * 0.7, mat)
        };

        // Sea stack at (20, 14), rising from seabed to 3.5 m (or 0.15 m if LOW_STACK).
        let stack_h = if KIND == LOW_STACK { 0.15 } else { 3.5 };
        let stack = cuboid(p - point(20.0, (stack_h - 1.5) * 0.5, 14.0), point(2.5, (stack_h + 1.5) * 0.5, 2.5));
        ground = ground.or(Answer::new(stack, Material::Stone));

        // Tall stone block on the cliff band at (40, 28) with a tiny room and slot on its south face.
        // Room and slot cut 1 mm below ground_y = 0.5 so no lid is left and the floor sits at 0.499.
        let block = cuboid(p - point(40.0, 2.0, 28.0), point(0.8, 1.5, 0.8));
        let room = if KIND == OPEN_TINY_PLACE {
            cuboid(p - point(40.0, 1.5, 28.0), point(0.45, 1.001, 0.45))
        } else {
            cuboid(p - point(40.0, 0.5295, 28.0), point(0.05, 0.0305, 0.05))
        };
        let slot = if KIND == OPEN_TINY_PLACE {
            cuboid(p - point(40.0, 1.5, 28.55), point(0.45, 1.001, 0.5))
        } else {
            cuboid(p - point(40.0, 0.5145, 28.45), point(0.008, 0.0155, 0.45))
        };
        let hollow = union(room, slot);
        let stone = cut(block, hollow);
        ground = ground.cut(hollow).or(Answer::new(stone, Material::Stone));
        ground
    }
}

fn coast_sketch() -> Sketch {
    let mut c = Canvas::new(256, 256);
    c.fill_rect(0.0, 0.0, 64.0, 24.0, Pen::Blue);
    c.fill_rect(0.0, 24.0, 64.0, 32.0, Pen::Grey);
    c.fill_rect(0.0, 32.0, 64.0, 64.0, Pen::Brown);
    c.fill_circle(40.0, 28.0, 2.0, Pen::Orange);
    c.fill_circle(20.0, 14.0, 2.5, Pen::Purple);
    c.to_sketch()
}

fn check_coast<const KIND: u8>(sketch: &Sketch) -> (laws::Verdict, laws::Verdict) {
    let mut area = Native(Box::new(Coast::<KIND>));
    let d = area.describe().unwrap();
    let mat = laws::sketch_materials(&mut area, sketch).unwrap();
    let mut world = World::new(&mut area, &d.places);
    let all = world.reach(&Size::ALL).unwrap();
    let places = laws::sketch_places(&mut world, &all, &d, sketch).unwrap();
    (mat, places)
}

#[test]
fn an_area_matching_the_sketch_keeps_both_sketch_laws() {
    let sketch = coast_sketch();
    let (mat, places) = check_coast::<GOOD>(&sketch);
    assert_eq!(mat, Ok(()), "materials");
    assert_eq!(places, Ok(()), "places");
}

#[test]
fn missing_water_grassy_cliff_or_flooded_land_breaks_sketch_materials() {
    let sketch = coast_sketch();

    let (mat, _) = check_coast::<NO_WATER>(&sketch);
    let why = mat.expect_err("no water where blue");
    assert!(why.contains("water (blue)"), "{why}");

    let (mat, _) = check_coast::<GRASS_CLIFF>(&sketch);
    let why = mat.expect_err("grass where grey");
    assert!(why.contains("rock or cliff (grey)"), "{why}");

    let (mat, _) = check_coast::<FLOODED_LAND>(&sketch);
    let why = mat.expect_err("water where brown");
    assert!(why.contains("land (brown)"), "{why}");
}

#[test]
fn missing_or_reachable_places_break_sketch_places() {
    let sketch = coast_sketch();

    let (_, places) = check_coast::<MISSING_TINY_PLACE>(&sketch);
    let why = places.expect_err("no tiny place at orange");
    assert!(why.contains("orange mark") && why.contains("no place for a tiny person"), "{why}");

    let (_, places) = check_coast::<OPEN_TINY_PLACE>(&sketch);
    let why = places.expect_err("person reaches orange");
    assert!(why.contains("orange mark") && why.contains("reached by a person"), "{why}");

    let (_, places) = check_coast::<LOW_STACK>(&sketch);
    let why = places.expect_err("person reaches purple");
    assert!(why.contains("purple mark") && why.contains("reached by a person"), "{why}");
}

#[test]
fn the_repo_s_sketches_for_headland_and_pond_are_read_cleanly() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../sketches");

    let headland = Sketch::open(&dir.join("headland.png")).unwrap();
    assert!(headland.regions.iter().any(|r| r.pen == Pen::Blue));
    assert!(headland.regions.iter().any(|r| r.pen == Pen::Brown));
    assert!(headland.regions.iter().filter(|r| r.pen == Pen::Grey).count() >= 2);
    assert_eq!(headland.places.iter().filter(|p| p.size == Size::Tiny).count(), 1);
    assert_eq!(headland.places.iter().filter(|p| p.size == Size::Giant).count(), 1);
    let words: String = headland.words.iter().map(|w| w.text.to_lowercase()).collect::<Vec<_>>().join(" | ");
    if !headland.words.is_empty() {
        assert!(words.contains("lighthouse") && words.contains("cliff") && words.contains("sea stack"), "headland words: {words}");
    }

    let pond = Sketch::open(&dir.join("pond.png")).unwrap();
    assert!(pond.regions.iter().any(|r| r.pen == Pen::Blue));
    assert!(pond.regions.iter().any(|r| r.pen == Pen::Brown));
    assert!(pond.regions.iter().any(|r| r.pen == Pen::Grey));
    assert_eq!(pond.places.iter().filter(|p| p.size == Size::Tiny).count(), 1);
    assert_eq!(pond.places.iter().filter(|p| p.size == Size::Giant).count(), 1);
    let words: String = pond.words.iter().map(|w| w.text.to_lowercase()).collect::<Vec<_>>().join(" | ");
    if !pond.words.is_empty() {
        assert!(words.contains("lily pads") && words.contains("mossy rock") && words.contains("heron"), "pond words: {words}");
    }
}
