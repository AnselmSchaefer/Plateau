//! Where each size can go (POC_PLAN.md, step 3): the places a body of each size can stand, and the
//! ways between them, searched from the area's start. The laws "gates are real" and "no traps"
//! are kept against it (`laws`), and step 4's reach map is drawn from it.
//!
//! - **Stands.** The area is cut into columns as wide as a body's radius (`CELL` of its height),
//!   and no wider than `WIDEST` (`cell`): a giant's are 40 cm, or it would stand on no wall.
//!   Down each, the floors are found as the body finds ground (water is a floor only to those it
//!   holds up; anyone else goes through it to what is under it), each with the ceiling over it,
//!   and whether a body fits standing there: its spheres above the step touch nothing within most
//!   of their radius (`ROOM`), found with the body's own rays, since a distance is not a contact.
//! - **Walking.** From a stand into the next column (of eight), a body goes onto the highest floor
//!   there no higher than it jumps (`JUMP` of its height), if it fits there and has room overhead on
//!   the way: down any drop, up at most a jump. Not across gaps.
//! - **Changing size**, at will: shrinking anywhere (onto the floor under the larger's feet),
//!   growing where the larger fits.
//! - **The tiny are searched only about the places meant for them.** Their world is too large to
//!   search whole (the area is four thousand of their heights across), and wherever a person
//!   stands a tiny person can be, by shrinking. So about each place for the tiny (`TINY_ACROSS`)
//!   the search follows them out from the place, through what no person stands on, to where a
//!   person who has been reached does (they are "covered"). Elsewhere the tiny are taken to go
//!   where a person goes, which they can by shrinking; without persons, they are not searched.
//!
//! Every ruling here leans the same way where it must guess: a body is taken to reach a little
//! more than it may (a covered stand of the tiny's is taken as reached, wherever in the person's
//! column it is), never less, so a place one size alone should reach is not wrongly passed.

use std::collections::{HashMap, HashSet};

use crate::body::{Body, FLOATS_BELOW, JUMP, RADIUS, STEP};
use crate::{Material, Place, Region, Result, START, Size, WIDTH, chunk};

/// The heights searched, in metres: an area's ground is about y = 0.
pub const LOWEST: f32 = -32.0;
pub const HIGHEST: f32 = 64.0;
/// A column's width, as a fraction of the size's height: the body's radius.
pub const CELL: f32 = RADIUS;
/// The widest a column is, in metres: no wider than a garden wall's top, which a giant stands on.
pub const WIDEST: f32 = 0.4;

/// How wide a size's columns are, in metres.
pub fn cell(size: Size) -> f32 {
    (CELL * size.height()).min(WIDEST)
}
/// How far about a place for the tiny they are searched, across the ground and up and down.
pub const TINY_ACROSS: f32 = 2.0;
pub const TINY_UP: f32 = 1.0;
/// The most columns searched for the tiny about one of their places, in one search.
pub const MOST_TINY_COLUMNS: usize = 400_000;
/// How many persons' columns away from a tiny stand a person who covers it may be.
pub const COVER: i32 = 2;
/// A body fits where its spheres touch nothing within this much of their radius.
pub const ROOM: f64 = 0.9;
/// Two sizes stand on the same floor if their floors are within this much of the larger's height:
/// the detail the larger does not see.
pub const SAME_FLOOR: f32 = 0.1;
/// A size stands at a place if it stands in the place's column within this much of the smaller
/// of its height and the place's size's of the place's height.
pub const AT_PLACE: f32 = 0.5;

/// A floor down a column: where solid begins under air, the ceiling over that air (the underside
/// of the solid above, or none), what it is, and whether a body fits standing on it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Floor {
    pub y: f32,
    pub ceiling: f32,
    pub material: Material,
    pub fits: bool,
}

/// Where a body can be: its size, its column, and which floor down that column (from the top).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Stand {
    pub size: Size,
    pub at: [i32; 2],
    pub floor: u8,
}

const NEIGHBOURS: [[i32; 2]; 8] = [[-1, -1], [0, -1], [1, -1], [-1, 0], [1, 0], [-1, 1], [0, 1], [1, 1]];

/// The columns a step from `at` may need: those next to it, and those beyond them.
fn around(at: [i32; 2]) -> impl Iterator<Item = [i32; 2]> {
    NEIGHBOURS.iter().flat_map(move |d| [1, 2].map(|k| [at[0] + k * d[0], at[1] + k * d[1]]))
}

/// One size's columns, found as they are needed.
struct Layer {
    size: Size,
    cell: f32,
    /// Where columns are searched: boxes, each its low corner and its high one.
    boxes: Vec<([f32; 3], [f32; 3])>,
    columns: HashMap<[i32; 2], Vec<Floor>>,
}

impl Layer {
    fn height(&self) -> f32 {
        self.size.height()
    }

    fn middle(&self, at: [i32; 2]) -> [f32; 2] {
        at.map(|i| (i as f32 + 0.5) * self.cell)
    }

    fn column(&self, x: f32, z: f32) -> [i32; 2] {
        [(x / self.cell).floor() as i32, (z / self.cell).floor() as i32]
    }

    /// The heights searched down a column, if it is searched at all.
    fn range(&self, at: [i32; 2]) -> Option<(f32, f32)> {
        let [x, z] = self.middle(at);
        if !(0.0..WIDTH).contains(&x) || !(0.0..WIDTH).contains(&z) {
            return None;
        }
        self.boxes
            .iter()
            .filter(|(lo, hi)| (lo[0]..=hi[0]).contains(&x) && (lo[2]..=hi[2]).contains(&z))
            .map(|(lo, hi)| (lo[1], hi[1]))
            .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
    }

    fn floor(&self, s: Stand) -> Floor {
        self.columns[&s.at][s.floor as usize]
    }
}

/// An area's stands, found as the searches need them, kept between searches.
pub struct World<'a> {
    area: &'a mut dyn Region,
    layers: [Layer; 3],
    places: Vec<Place>,
    tiny_columns: usize,
    /// Whether the tiny walk straight from a stand of theirs to a person's, as found.
    walked: HashMap<(Stand, Stand), bool>,
}

/// What a search found: every stand reached, how each was first come to, and the ways out of it.
#[derive(Default)]
pub struct Reach {
    pub stands: Vec<Stand>,
    index: HashMap<Stand, usize>,
    from: Vec<Option<usize>>,
    next: Vec<Vec<usize>>,
    starts: Vec<usize>,
    /// The places for the tiny about which there was too much to search.
    pub unsettled: Vec<String>,
    /// About each place for the tiny: how many stands of theirs were found out from it, and at how
    /// many of those a person covers them.
    pub about: Vec<(String, usize, usize)>,
}

impl Reach {
    fn add(&mut self, s: Stand, from: Option<usize>) -> (usize, bool) {
        if let Some(&i) = self.index.get(&s) {
            return (i, false);
        }
        let i = self.stands.len();
        self.stands.push(s);
        self.index.insert(s, i);
        self.from.push(from);
        self.next.push(Vec::new());
        (i, true)
    }

    fn link(&mut self, a: usize, b: usize) {
        if !self.next[a].contains(&b) {
            self.next[a].push(b);
        }
    }

    pub fn has(&self, s: Stand) -> bool {
        self.index.contains_key(&s)
    }

    /// The way from the start to a stand reached, as the search first came to it: the shortest
    /// there is, in steps.
    pub fn way(&self, s: Stand) -> Vec<Stand> {
        let mut way = Vec::new();
        let mut at = self.index.get(&s).copied();
        while let Some(i) = at {
            way.push(self.stands[i]);
            at = self.from[i];
        }
        way.reverse();
        way
    }

    /// For each stand, whether the start can be got back to from it.
    pub fn back(&self) -> Vec<bool> {
        let mut before = vec![Vec::new(); self.stands.len()];
        for (a, next) in self.next.iter().enumerate() {
            for &b in next {
                before[b].push(a);
            }
        }
        let mut back = vec![false; self.stands.len()];
        let mut wave = self.starts.clone();
        for &i in &wave {
            back[i] = true;
        }
        while let Some(i) = wave.pop() {
            for &a in &before[i] {
                if !back[a] {
                    back[a] = true;
                    wave.push(a);
                }
            }
        }
        back
    }
}

/// What is reached being any size, and being never each size: what the gates and the reach maps
/// are judged by.
pub struct Reaches {
    pub all: Reach,
    /// By size: what is reached by those who are never it.
    pub without: [Reach; 3],
}

impl<'a> World<'a> {
    /// Everything reached being any size, and being never each size in turn.
    pub fn reaches(&mut self) -> Result<Reaches> {
        let all = self.reach(&Size::ALL)?;
        let mut without = Vec::new();
        for s in Size::ALL {
            without.push(self.reach(&Size::ALL.into_iter().filter(|&o| o != s).collect::<Vec<_>>())?);
        }
        let [t, p, g]: [Reach; 3] = without.try_into().ok().expect("three sizes");
        Ok(Reaches { all, without: [t, p, g] })
    }

    /// The stands of `area`, whose places are `places`, none found yet.
    pub fn new(area: &'a mut dyn Region, places: &[Place]) -> World<'a> {
        let layer = |size: Size, boxes| Layer { size, cell: cell(size), boxes, columns: HashMap::new() };
        let whole = vec![([0.0, LOWEST, 0.0], [WIDTH, HIGHEST, WIDTH])];
        let tiny = places
            .iter()
            .filter(|p| p.size == Size::Tiny)
            .map(|p| {
                let [x, y, z] = p.at;
                ([x - TINY_ACROSS, y - TINY_UP, z - TINY_ACROSS], [x + TINY_ACROSS, y + TINY_UP, z + TINY_ACROSS])
            })
            .collect();
        World {
            area,
            layers: [layer(Size::Tiny, tiny), layer(Size::Person, whole.clone()), layer(Size::Giant, whole)],
            places: places.to_vec(),
            tiny_columns: 0,
            walked: HashMap::new(),
        }
    }

    fn layer(&self, size: Size) -> &Layer {
        &self.layers[size as usize]
    }

    /// Where a stand is: the middle of its column, on its floor.
    pub fn point(&self, s: Stand) -> [f32; 3] {
        let layer = self.layer(s.size);
        let [x, z] = layer.middle(s.at);
        [x, layer.floor(s).y, z]
    }

    pub fn floor(&self, s: Stand) -> Floor {
        self.layer(s.size).floor(s)
    }

    /// The floors of the column of `size` that a point across the ground is in, if found.
    pub fn floors_at(&self, size: Size, x: f32, z: f32) -> Option<&[Floor]> {
        let layer = self.layer(size);
        layer.columns.get(&layer.column(x, z)).map(Vec::as_slice)
    }

    /// The floors of the column a stand is in, from the top.
    pub fn column(&self, s: Stand) -> &[Floor] {
        &self.layer(s.size).columns[&s.at]
    }

    /// The columns found so far for a size, with their floors.
    pub fn columns(&self, size: Size) -> impl Iterator<Item = ([f32; 2], &[Floor])> {
        let layer = self.layer(size);
        layer.columns.iter().map(|(&at, floors)| (layer.middle(at), floors.as_slice()))
    }

    /// Finds the floors of the columns not yet found, all together.
    fn find(&mut self, size: Size, ats: impl IntoIterator<Item = [i32; 2]>) -> Result<()> {
        let layer = &self.layers[size as usize];
        let mut todo = Vec::new();
        let mut seen = HashSet::new();
        for at in ats {
            if layer.columns.contains_key(&at) || !seen.insert(at) {
                continue;
            }
            todo.push((at, layer.middle(at), layer.range(at)));
        }
        let searched: Vec<_> = todo.iter().filter_map(|&(_, m, r)| r.map(|(lo, hi)| (m, lo, hi))).collect();
        let found = floors(self.area, size.height(), &searched)?;
        if size == Size::Tiny {
            self.tiny_columns += searched.len();
        }
        let layer = &mut self.layers[size as usize];
        let mut found = found.into_iter();
        for (at, _, range) in todo {
            let floors = if range.is_some() { found.next().unwrap() } else { Vec::new() };
            layer.columns.insert(at, floors);
        }
        Ok(())
    }

    /// The floor a body standing on `f` lands on stepping into the column `at` from beside it: the
    /// highest no higher than it jumps, if there is room overhead on the way.
    fn landing(&self, size: Size, f: Floor, at: [i32; 2]) -> Option<(usize, Floor)> {
        let h = size.height();
        let floors = self.layer(size).columns.get(&at)?;
        let j = floors.iter().position(|g| g.y <= f.y + JUMP * h)?;
        let g = floors[j];
        let overhead = g.ceiling >= f.y.max(g.y) + 0.9 * h && (g.y <= f.y || f.ceiling >= g.y + 0.9 * h);
        overhead.then_some((j, g))
    }

    /// Where a body standing at `from` goes stepping into the column `at` next to it, if it can.
    /// Dropping into a hollow whose edge has no room for it, it slides on into the column beyond,
    /// as a body falling against the hollow's side is pushed off it.
    fn step(&self, from: Stand, at: [i32; 2]) -> Option<Stand> {
        let layer = self.layer(from.size);
        let h = layer.height();
        let f = layer.floor(from);
        let (j, g) = self.landing(from.size, f, at)?;
        if g.fits {
            return Some(Stand { size: from.size, at, floor: j as u8 });
        }
        if g.y >= f.y - STEP * h {
            return None;
        }
        let beyond = [2 * at[0] - from.at[0], 2 * at[1] - from.at[1]];
        let (k, b) = self.landing(from.size, g, beyond)?;
        (b.fits && (b.y - g.y).abs() <= STEP * h).then_some(Stand { size: from.size, at: beyond, floor: k as u8 })
    }

    /// Every stand a body standing at `from` walks to in one step, among the columns found.
    fn walk(&self, from: Stand) -> Vec<Stand> {
        NEIGHBOURS.iter().filter_map(|d| self.step(from, [from.at[0] + d[0], from.at[1] + d[1]])).collect()
    }

    /// The column of `size` that the point of `s` is in.
    fn column_of(&self, s: Stand, size: Size) -> [i32; 2] {
        let [x, _, z] = self.point(s);
        self.layer(size).column(x, z)
    }

    /// Where `from` stands as `size` after shrinking or growing to it, if it fits there: the
    /// highest floor of that size's column under its feet (no higher than the detail the larger did
    /// not see), and growing, no lower than the larger's step.
    fn change(&self, from: Stand, size: Size) -> Option<Stand> {
        let at = self.column_of(from, size);
        let floors = self.layer(size).columns.get(&at)?;
        let y = self.floor(from).y;
        let larger = from.size.height().max(size.height());
        let j = floors.iter().position(|g| g.y <= y + SAME_FLOOR * larger)?;
        let g = floors[j];
        let growing = size.height() > from.size.height();
        (g.fits && (!growing || g.y >= y - STEP * larger)).then_some(Stand { size, at, floor: j as u8 })
    }

    /// The stand of `size` at a place, if it can stand there: on a floor at the place's very point
    /// (not just in its column: a giant's is two and a half metres wide) within `AT_PLACE` of the
    /// smaller of the two sizes, with room; and the stand of its column on that same floor.
    pub fn at_place(&mut self, place: &Place, size: Size) -> Result<Option<Stand>> {
        let [x, y, z] = place.at;
        let h = size.height();
        let here = floors(self.area, h, &[([x, z], y - TINY_UP.max(h), y + TINY_UP.max(2.0 * h))])?.remove(0);
        let near = AT_PLACE * h.min(place.size.height());
        let Some(f) = here.iter().filter(|f| f.fits && (f.y - y).abs() <= near).min_by(|a, b| (a.y - y).abs().total_cmp(&(b.y - y).abs()))
        else {
            return Ok(None);
        };
        // The column it is in, or the nearest beside it, on the same floor.
        let middle = self.layer(size).column(x, z);
        let around: Vec<[i32; 2]> = std::iter::once([0, 0]).chain(NEIGHBOURS).map(|d| [middle[0] + d[0], middle[1] + d[1]]).collect();
        self.find(size, around.iter().copied())?;
        let layer = self.layer(size);
        let mut best: Option<(f32, Stand)> = None;
        for at in around {
            let [cx, cz] = layer.middle(at);
            let far = (cx - x).powi(2) + (cz - z).powi(2);
            for (j, g) in layer.columns[&at].iter().enumerate() {
                if g.fits && (g.y - f.y).abs() <= SAME_FLOOR * h && best.is_none_or(|(b, _)| far < b) {
                    best = Some((far, Stand { size, at, floor: j as u8 }));
                }
            }
        }
        Ok(best.map(|(_, s)| s))
    }

    /// The stand of `size` at the start: the highest it fits on in the start's column.
    fn start(&mut self, size: Size) -> Result<Option<Stand>> {
        let at = self.layer(size).column(START[0] as f32, START[1] as f32);
        self.find(size, [at])?;
        let floors = &self.layer(size).columns[&at];
        Ok(floors.iter().position(|g| g.fits).map(|j| Stand { size, at, floor: j as u8 }))
    }

    /// Everything a body reaches from the start, being only `sizes` (and changing between them at
    /// will).
    pub fn reach(&mut self, sizes: &[Size]) -> Result<Reach> {
        let mut reach = Reach::default();
        let mut wave = Vec::new();
        for size in [Size::Person, Size::Giant] {
            if sizes.contains(&size)
                && let Some(s) = self.start(size)?
            {
                let (i, _) = reach.add(s, None);
                reach.starts.push(i);
                wave.push(i);
            }
        }
        // The tiny come and go where persons stand: without persons, they are not searched (to come
        // from a giant shrinking, a tiny person would walk out of the giant's reach of every wall).
        let tiny = sizes.contains(&Size::Tiny) && sizes.contains(&Size::Person);
        let mut tiny_done = false;
        loop {
            self.spread(&mut reach, &mut wave, sizes)?;
            if !tiny || tiny_done {
                break;
            }
            tiny_done = true;
            for place in self.places.clone().iter().filter(|p| p.size == Size::Tiny) {
                wave.extend(self.tiny_about(&mut reach, place)?);
            }
            if wave.is_empty() {
                break;
            }
        }
        Ok(reach)
    }

    /// The search across the persons' and giants' columns, from the stands of `wave` on.
    fn spread(&mut self, reach: &mut Reach, wave: &mut Vec<usize>, sizes: &[Size]) -> Result<()> {
        let other = |s: Size| match s {
            Size::Person if sizes.contains(&Size::Giant) => Some(Size::Giant),
            Size::Giant if sizes.contains(&Size::Person) => Some(Size::Person),
            _ => None,
        };
        while !wave.is_empty() {
            for size in [Size::Person, Size::Giant] {
                let mut needed = Vec::new();
                let mut changed = Vec::new();
                for &i in wave.iter() {
                    let s = reach.stands[i];
                    if s.size == size {
                        needed.extend(around(s.at));
                    } else if other(s.size) == Some(size) {
                        changed.push(self.column_of(s, size));
                    }
                }
                self.find(size, needed.into_iter().chain(changed))?;
            }
            let mut next = Vec::new();
            for &i in wave.iter() {
                let s = reach.stands[i];
                if s.size == Size::Tiny {
                    continue;
                }
                let mut to = self.walk(s);
                to.extend(other(s.size).and_then(|o| self.change(s, o)));
                for t in to {
                    let (j, new) = reach.add(t, Some(i));
                    reach.link(i, j);
                    if new {
                        next.push(j);
                    }
                }
            }
            *wave = next;
        }
        Ok(())
    }

    /// For each of the tiny stands `wave`, the person reached near it on the same floor, if the tiny
    /// can walk straight from one to the other (`COVER` persons' columns away at most, half the
    /// tiny's body clear all the way, and a person sees air just above it): there, a tiny person can
    /// be by shrinking and walking a little.
    /// The nearest two persons are tried.
    fn covering(&mut self, reach: &Reach, wave: &[Stand]) -> Result<Vec<Option<usize>>> {
        let (tiny, by, cover) = (Size::Tiny.height(), Size::Person, COVER);
        // First, whether a person sees open air there at all, a little above the floor: inside
        // what is solid to a person (a crack, a cave) nobody covers the tiny.
        let above: Vec<[f32; 3]> = wave.iter().map(|&s| self.point(s)).map(|p| [p[0], p[1] + 0.05 * by.height(), p[2]]).collect();
        let mut open = Vec::new();
        chunk::sample(self.area, &above, by.height(), &mut open)?;
        let (mut lines, mut asked, mut whose) = (Vec::new(), Vec::new(), Vec::new());
        for (k, &s) in wave.iter().enumerate() {
            if open[k].distance <= 0.0 && open[k].material != Material::Water {
                continue;
            }
            let p = self.point(s);
            let at = self.column_of(s, by);
            let layer = self.layer(by);
            let mut near = Vec::new();
            for dx in -cover..=cover {
                for dz in -cover..=cover {
                    let c = [at[0] + dx, at[1] + dz];
                    let Some(floors) = layer.columns.get(&c) else { continue };
                    let [cx, cz] = layer.middle(c);
                    for (j, g) in floors.iter().enumerate() {
                        if (g.y - p[1]).abs() <= SAME_FLOOR * by.height()
                            && let Some(&i) = reach.index.get(&Stand { size: by, at: c, floor: j as u8 })
                        {
                            near.push(((cx - p[0]).powi(2) + (cz - p[2]).powi(2), i, [cx, g.y, cz]));
                        }
                    }
                }
            }
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (_, i, q) in near.into_iter().take(2) {
                let pair = (s, reach.stands[i]);
                if !self.walked.contains_key(&pair) {
                    lines.push(([p[0], p[1] + 0.5 * tiny, p[2]], [q[0], q[1] + 0.5 * tiny, q[2]]));
                    asked.push(pair);
                }
                whose.push((k, i, pair));
            }
        }
        let clear = clear(self.area, tiny, &lines, 0.5 * RADIUS * tiny)?;
        self.walked.extend(asked.into_iter().zip(clear));
        let mut out = vec![None; wave.len()];
        for &(k, i, pair) in &whose {
            if self.walked[&pair] && out[k].is_none() {
                out[k] = Some(i);
            }
        }
        Ok(out)
    }

    /// The tiny about one of their places: from the place out through what no person stands on
    /// to where one does; then onward from there, as far as the tiny go. The persons the tiny grow
    /// into somewhere no person was are what it gives back, to be searched on from.
    fn tiny_about(&mut self, reach: &mut Reach, place: &Place) -> Result<Vec<usize>> {
        let (by, before) = (Size::Person, self.tiny_columns);
        let Some(at) = self.at_place(place, Size::Tiny)? else { return Ok(Vec::new()) };
        // Out from the place, along ways in either direction.
        let mut region = HashSet::from([at]);
        let mut rim: HashMap<Stand, usize> = HashMap::new();
        let mut wave = vec![at];
        while !wave.is_empty() {
            if self.tiny_columns - before > MOST_TINY_COLUMNS {
                reach.unsettled.push(place.name.clone());
                break;
            }
            let mut needed = Vec::new();
            for (s, c) in wave.iter().zip(self.covering(reach, &wave)?) {
                if let Some(c) = c {
                    rim.insert(*s, c);
                    continue;
                }
                needed.extend(around(s.at));
            }
            self.find(Size::Tiny, needed)?;
            let mut next = Vec::new();
            for s in wave.into_iter().filter(|s| !rim.contains_key(s)) {
                let mut near = self.walk(s);
                for d in NEIGHBOURS {
                    let c = [s.at[0] + d[0], s.at[1] + d[1]];
                    let n = self.layer(Size::Tiny).columns[&c].len();
                    near.extend((0..n).map(|j| Stand { size: Size::Tiny, at: c, floor: j as u8 }).filter(|&b| self.step(b, s.at) == Some(s)));
                }
                for b in near {
                    if region.insert(b) {
                        next.push(b);
                    }
                }
            }
            wave = next;
        }
        reach.about.push((place.name.clone(), region.len(), rim.len()));
        // Onward from where a person stands, within what was found.
        let mut wave = Vec::new();
        for (&s, &c) in &rim {
            let (i, new) = reach.add(s, Some(c));
            reach.link(c, i);
            reach.link(i, c);
            if new {
                wave.push(i);
            }
        }
        let mut reached = Vec::new();
        while !wave.is_empty() {
            let mut next = Vec::new();
            for &i in &wave {
                reached.push(i);
                for t in self.walk(reach.stands[i]).into_iter().filter(|t| region.contains(t)) {
                    let (j, new) = reach.add(t, Some(i));
                    reach.link(i, j);
                    if new {
                        next.push(j);
                    }
                }
            }
            wave = next;
        }
        // Growing, where there is room, back into a person.
        let grown: Vec<_> = reached.iter().map(|&i| self.column_of(reach.stands[i], by)).collect();
        self.find(by, grown)?;
        let mut persons = Vec::new();
        for i in reached {
            if let Some(p) = self.change(reach.stands[i], by) {
                let (j, new) = reach.add(p, Some(i));
                reach.link(i, j);
                reach.link(j, i);
                if new {
                    persons.push(j);
                }
            }
        }
        Ok(persons)
    }
}

/// The floors down columns, each its middle across the ground and the heights searched, for a
/// body `h` tall: all the columns marched down together, a batch of points at a time.
fn floors(area: &mut dyn Region, h: f32, columns: &[([f32; 2], f32, f32)]) -> Result<Vec<Vec<Floor>>> {
    enum State {
        Air,
        Solid,
        /// A floor between two heights, being narrowed.
        Narrowing { above: f32, below: f32, times: u32, material: Material },
    }
    struct March {
        y: f32,
        above: f32,
        ceiling: f32,
        state: State,
        first: bool,
        steps: u32,
        floors: Vec<Floor>,
        /// The last floor was found on entering the solid now being gone through.
        fresh: bool,
    }
    let floats = h < FLOATS_BELOW;
    // Within this of a surface is on it; no step is shorter than `least` (a thinner floor is
    // stepped over), none through water longer than a twentieth of the body.
    let (close, least) = (0.002 * h, h / 40.0);
    let mut marches: Vec<March> = columns
        .iter()
        .map(|&(_, _, high)| March { y: high, above: high, ceiling: f32::INFINITY, state: State::Air, first: true, steps: 0, floors: Vec::new(), fresh: false })
        .collect();
    let mut active: Vec<usize> = (0..columns.len()).collect();
    let (mut points, mut out) = (Vec::new(), Vec::new());
    while !active.is_empty() {
        points.clear();
        points.extend(active.iter().map(|&k| [columns[k].0[0], marches[k].y, columns[k].0[1]]));
        chunk::sample(area, &points, h, &mut out)?;
        let mut still = Vec::with_capacity(active.len());
        for (j, &k) in active.iter().enumerate() {
            let a = out[j];
            let m = &mut marches[k];
            let solid = (a.material != Material::Water || floats).then_some(a.distance);
            let air_step = |d: Option<f32>| match d {
                Some(d) => d.max(least),
                None => a.distance.abs().clamp(least, h / 20.0),
            };
            match m.state {
                State::Narrowing { above, below, times, material } => {
                    let (above, below, material) =
                        if solid.is_some_and(|d| d < 0.0) { (above, m.y, a.material) } else { (m.y, below, material) };
                    if times >= 5 {
                        m.floors.push(Floor { y: above, ceiling: m.ceiling, material, fits: false });
                        m.fresh = true;
                        m.state = State::Solid;
                        m.y = below;
                    } else {
                        m.state = State::Narrowing { above, below, times: times + 1, material };
                        m.y = (above + below) * 0.5;
                    }
                }
                State::Solid => match solid {
                    Some(d) if d <= 0.0 => m.y -= (-d).max(least),
                    // A floor with nothing under it, thinner than a step: none (where a shape
                    // cut out ends exactly at the surface it is cut from, the distance is nought
                    // across its mouth; the law `surfaces_have_an_inside` refuses it).
                    _ if m.fresh && m.floors.last().is_some_and(|f| f.y - m.y <= 1.5 * least) => {
                        m.floors.pop();
                        m.fresh = false;
                        m.state = State::Air;
                        m.above = m.y;
                        m.y -= air_step(solid);
                    }
                    _ => {
                        m.fresh = false;
                        m.ceiling = m.y;
                        m.state = State::Air;
                        m.above = m.y;
                        m.y -= air_step(solid);
                    }
                },
                State::Air => match solid {
                    Some(d) if d > close => {
                        m.above = m.y;
                        m.y -= air_step(Some(d));
                    }
                    None => {
                        m.above = m.y;
                        m.y -= air_step(None);
                    }
                    Some(_) if m.first => m.state = State::Solid,
                    Some(d) if d >= -close => {
                        m.floors.push(Floor { y: m.y, ceiling: m.ceiling, material: a.material, fits: false });
                        m.fresh = true;
                        m.state = State::Solid;
                        m.y -= least;
                    }
                    Some(_) => {
                        m.state = State::Narrowing { above: m.above, below: m.y, times: 0, material: a.material };
                        m.y = (m.above + m.y) * 0.5;
                    }
                },
            }
            m.first = false;
            m.steps += 1;
            if m.y >= columns[k].1 && m.steps < 4000 {
                still.push(k);
            }
        }
        active = still;
    }
    // Whether a body fits on each floor: room overhead, and its spheres touching nothing.
    let body = Body::of_size(h);
    let (mut middles, mut whose) = (Vec::new(), Vec::new());
    for (k, m) in marches.iter().enumerate() {
        let [x, z] = columns[k].0;
        for (i, f) in m.floors.iter().enumerate() {
            if f.ceiling - f.y >= 0.95 * h {
                middles.extend(body.spheres_at([x as f64, f.y as f64, z as f64]));
                whose.push((k, i));
            }
        }
    }
    let touching = body.touching(area, &middles, ROOM * body.radius() as f64)?;
    for (n, &(k, i)) in whose.iter().enumerate() {
        marches[k].floors[i].fits = touching[n * 4..n * 4 + 4].iter().all(Option::is_none);
    }
    Ok(marches.into_iter().map(|m| m.floors).collect())
}

/// For each line, whether a ball of radius `r` goes along it from one end to the other touching
/// nothing, as a viewer `size` tall sees the area.
fn clear(area: &mut dyn Region, size: f32, lines: &[([f32; 3], [f32; 3])], r: f32) -> Result<Vec<bool>> {
    let dirs: Vec<([f32; 3], f32)> = lines
        .iter()
        .map(|(a, b)| {
            let d = [0, 1, 2].map(|i| b[i] - a[i]);
            let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            (d.map(|c| c / l.max(1e-9)), l)
        })
        .collect();
    let mut t = vec![0.0f32; lines.len()];
    let mut out = vec![false; lines.len()];
    let mut active: Vec<usize> = (0..lines.len()).collect();
    let mut answers = Vec::new();
    for _ in 0..400 {
        if active.is_empty() {
            break;
        }
        let points: Vec<[f32; 3]> = active.iter().map(|&k| [0, 1, 2].map(|i| lines[k].0[i] + dirs[k].0[i] * t[k])).collect();
        chunk::sample(area, &points, size, &mut answers)?;
        let mut still = Vec::new();
        for (j, &k) in active.iter().enumerate() {
            let a = answers[j];
            // Water is no wall to anyone: the tiny walk on it, the others through it.
            let d = if a.material == Material::Water { a.distance.abs().max(r * 2.0) } else { a.distance };
            if d < r {
                continue;
            }
            t[k] += d - r * 0.9;
            if t[k] >= dirs[k].1 {
                out[k] = true;
            } else {
                still.push(k);
            }
        }
        active = still;
    }
    Ok(out)
}
