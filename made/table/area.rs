use plateau_sdk::*;

/// The giant's kitchen table: a kitchen floor of great flagstones in the open, the giant's table
/// and chair on it, a hearth ring of boulders, and hills of grass round about.
///
/// - **A giant** jumps onto the chair's seat, 4.5 m up, and from there steps onto the table, set
///   with a loaf on its board, a stoneware mug and a bundle of herbs.
/// - **A person** walks in under the table, a hall with a wooden roof seven metres up, among
///   fallen crumbs, past a dropped spoon to a heap of spilled sugar.
/// - **The tiny** find, under the table, a giant's wooden bowl lying upside down, and in its rim a
///   little arched door. Inside lives a village: houses of wood and stone roofed in leaves and moss,
///   some with chimneys, on streets of packed earth round a cotton spool wound with green thread.
struct Kitchen {
    seed: u32,
    /// The floor's height under the bowl, where the village stands.
    fl: f32,
    /// The floor's height at the hearth.
    hearth_y: f32,
    stones: [Point; 7],
    radii: [f32; 7],
}

const TABLE: Point = point(32.0, 0.0, 26.0);
const TOP: f32 = 7.6;
const CHAIR: Point = point(38.0, 0.0, 36.3);
const SEAT: f32 = 4.5;

const BOWL: Point = point(27.0, 0.0, 27.0);
const BOWL_R: f32 = 1.6;
const SINK: f32 = 0.4;
const SHELL: f32 = 0.08;
/// The way the bowl's door faces, across the ground.
const DOOR: (f32, f32) = (0.6, 0.8);
/// The village square, between the spool and the door.
const SQUARE: Point = point(27.372, 0.0, 27.496);

const HEAP: Point = point(38.0, 0.0, 23.0);
const HEARTH: Point = point(13.5, 0.0, 13.5);
const FLAG: f32 = 6.0;

const SPOON_A: Point = point(29.0, 0.17, 30.0);
const SPOON_B: Point = point(34.6, 0.2, 31.12);
const SPOON_C: Point = point(35.28, 0.0, 31.26);

impl Area for Kitchen {
    fn new(seed: u64) -> Kitchen {
        let seed = (seed ^ (seed >> 32)) as u32;
        let mut k = Kitchen { seed, fl: 0.0, hearth_y: 0.0, stones: [point(0.0, 0.0, 0.0); 7], radii: [1.0; 7] };
        k.fl = k.base(BOWL.x, BOWL.z) + k.offset((BOWL.x / FLAG).floor() as i32, (BOWL.z / FLAG).floor() as i32);
        k.hearth_y = k.base(HEARTH.x, HEARTH.z) + k.offset((HEARTH.x / FLAG).floor() as i32, (HEARTH.z / FLAG).floor() as i32);
        let mut s = Seeds::of(seed as u64, 99);
        for i in 0..7 {
            let a = i as f32 * TAU / 7.0 + s.next(-0.06, 0.06);
            let r = s.next(1.0, 1.3);
            k.stones[i] = point(4.6 * cos(a), r * 0.3, 4.6 * sin(a));
            k.radii[i] = r;
        }
        k
    }

    fn describe(&self) -> Description {
        let tiny = View::new(TINY);
        let person = View::new(PERSON);
        let square = self.surface(SQUARE.x, SQUARE.z, &tiny).0;
        let heap = self.surface(HEAP.x, HEAP.z, &person).0;
        Description {
            name: "The giant's kitchen table".into(),
            places: vec![
                Place::new("the village under the upturned bowl", Size::Tiny, point(SQUARE.x, square, SQUARE.z)),
                Place::new("the spilled sugar under the table", Size::Person, point(HEAP.x, heap, HEAP.z)),
                Place::new("the giant's chair", Size::Giant, point(CHAIR.x, SEAT, CHAIR.z - 0.3)),
            ],
            borders: Borders::of_ground(|x, z| self.base(x, z)),
        }
    }

    fn sample(&self, p: Point, view: &View) -> Answer {
        self.ground(p, view)
            .or(self.table(p, view))
            .or(self.chair(p, view))
            .or(self.spoon(p, view))
            .or(self.bowl(p, view))
            .or(self.hearth(p, view))
    }
}

area!(Kitchen);

/// How far outside the kitchen's floor a point is, across the ground (negative inside).
fn kitchen(x: f32, z: f32) -> f32 {
    max(abs(x - 32.0) - 23.0, abs(z - 28.0) - 21.0)
}

/// A distance roughened by `detail`, only near the surface, faded out further off into the most it
/// could take away, so it never jumps; made shorter by `k` for the roughness's slopes.
fn rough(d: f32, p: Point, view: &View, wide: f32, steep: f32, seed: u32, k: f32) -> f32 {
    let most = steep * wide * 2.0;
    let near = between(d, most + 0.06, most + 0.02);
    let det = if near > 0.0 { detail(p, view, wide, steep, seed) } else { 0.0 };
    (d + mix(-most, det, near)) * k
}

impl Kitchen {
    /// The ground's height without its flagstones or roughness: a floor worn nearly level, hills
    /// rising outside it, lower along the path from the south.
    fn base(&self, x: f32, z: f32) -> f32 {
        let s = kitchen(x, z);
        let wear = 0.04 * noise(point(x / 14.0, 0.0, z / 14.0), self.seed);
        let path = mix(0.35, 1.0, between(abs(x - 32.0), 3.0, 8.0));
        let hills = 1.6 * between(s, 0.5, 10.0) * path * (0.6 + 0.4 * noise(point(x / 11.0, 0.0, z / 11.0), self.seed ^ 0x51));
        wear + hills
    }

    /// How much a flagstone stands proud or sunk, each its own.
    fn offset(&self, i: i32, j: i32) -> f32 {
        0.015 * ((hash(i, 0, j, self.seed ^ 0xf1a6) >> 8) as f32 / 8_388_608.0 - 1.0)
    }

    /// The flagstones: each at its own height, the joints between sunk `depth`; the heights meet
    /// at the joints' middles, so nothing jumps from one stone to the next. Also how far onto a
    /// stone from its joint (0 in the joint, 1 on the stone).
    fn flag(&self, x: f32, z: f32, depth: f32) -> (f32, f32) {
        let (fx, fz) = (x / FLAG, z / FLAG);
        let jx = abs(fx - (fx + 0.5).floor()) * FLAG;
        let jz = abs(fz - (fz + 0.5).floor()) * FLAG;
        let edge = between(min(jx, jz), 0.05, 0.13);
        let off = self.offset(fx.floor() as i32, fz.floor() as i32);
        (mix(-depth, off, edge), edge)
    }

    /// The heap of spilled sugar's height above the floor.
    fn heap(&self, x: f32, z: f32) -> f32 {
        let r = (point(x, 0.0, z) - HEAP).flat_length();
        if r > 3.2 {
            return 0.0;
        }
        let r = r + 0.3 * noise(point(x / 0.9, 0.0, z / 0.9), self.seed ^ 0x5a);
        0.55 * ease(1.0 - r / 2.6)
    }

    /// The ground's surface at `(x, z)` for `view`: its height, material, and the sugar's height.
    fn surface(&self, x: f32, z: f32, view: &View) -> (f32, Material, f32) {
        let b = self.base(x, z);
        let s = kitchen(x, z);
        let inside = between(-s, 0.0, 0.6);
        let (flag, edge) = self.flag(x, z, 0.03 * view.sees(0.25));
        let floor = b + inside * flag;
        let heap = self.heap(x, z);
        let heaped = b + heap - 0.05;
        let top = max(floor, heaped);
        // Rough stone and earth, smoothed under the bowl where the village keeps its floor swept.
        let (bx, bz) = (x - BOWL.x, z - BOWL.z);
        let r = (bx * bx + bz * bz).sqrt();
        let mask = between(r, 1.7, 2.1);
        let roughness = if mask > 0.0 { mask * detail(point(x, 0.0, z), view, 0.6, 0.03, self.seed ^ 0x9e37) } else { 0.0 };
        let path = abs(x - 32.0) + 0.8 * noise(point(x / 2.0, 0.0, z / 2.0), self.seed ^ 0x71) < 1.6 && z > 48.5;
        // Inside the bowl: streets of packed earth, the lane to the door and the square paved.
        let u = bx * DOOR.0 + bz * DOOR.1;
        let v = bz * DOOR.0 - bx * DOOR.1;
        let (sx, sz) = (x - SQUARE.x, z - SQUARE.z);
        let paved = (u > 0.0 && abs(v) < 0.045) || (sx * sx + sz * sz) < 0.16 * 0.16;
        let material = if heaped > floor {
            Material::Sand
        } else if r < 1.44 && !paved {
            Material::Soil
        } else if inside > 0.5 {
            if edge < 0.35 { Material::Soil } else { Material::Stone }
        } else if path {
            Material::Soil
        } else if s + 1.5 * noise(point(x / 3.0, 0.0, z / 3.0), self.seed ^ 0x33) > 2.0 {
            Material::Grass
        } else {
            Material::Soil
        };
        (top + roughness, material, heap)
    }

    /// The ground; under the table, crumbs fallen from it; and on the sugar, for the tiny, its
    /// grains: crystals as big as they are.
    fn ground(&self, p: Point, view: &View) -> Answer {
        let b = self.base(p.x, p.z);
        let above = p.y - b - 0.7;
        if above > 0.3 {
            return Answer::new(above * 0.6, Material::Grass);
        }
        let (h, material, heap) = self.surface(p.x, p.z, view);
        // The joints' bevels and the roughness slope together: the distance halved.
        let mut ground = Answer::new((p.y - h) * 0.5, material);

        // Crumbs, a hand across to a person, boulders to the tiny, unseen by a giant; none about
        // the bowl's door or on the sugar.
        let cs = view.sees(0.08);
        if cs > 0.0 && abs(p.y - h) < 0.25 {
            let under = between(11.5 - abs(p.x - TABLE.x), 0.0, 0.6) * between(6.5 - abs(p.z - TABLE.z), 0.0, 0.6);
            let clear = between((p - BOWL).flat_length(), 2.3, 2.7) * between((p - HEAP).flat_length(), 3.0, 3.4);
            let size = cs * under * clear;
            if size > 0.0 {
                let crumbs = scatter(point(p.x, p.y - h, p.z), 0.9, self.seed ^ 0xc7, |q, s| {
                    if s.next(0.0, 1.0) > 0.45 {
                        return 0.8;
                    }
                    let r = s.next(0.04, 0.12) * size;
                    let c = q.turned(s.next(0.0, TAU)) - point(0.0, r * 0.3, 0.0);
                    rounded_cuboid(c, point(r, r * 0.7 + 0.001, r * 0.8), r * 0.4)
                });
                ground = ground.or(Answer::new(crumbs * 0.6, Material::Soil));
            }
        }

        let seen = view.sees(0.006);
        if seen == 0.0 || heap <= 0.0 || abs(p.y - h) > 0.012 {
            return ground;
        }
        let size = seen * between(heap, 0.0, 0.04);
        let crystals = scatter(point(p.x, p.y - h, p.z), 0.012, self.seed ^ 0x5c, |q, s| {
            let r = s.next(0.0015, 0.003) * size;
            cuboid(q.turned(s.next(0.0, TAU)), point(r, r, r))
        });
        // Placed by their height above the sloping heap: shorter, as the ground's.
        ground.or(Answer::new(crystals * 0.6, Material::Sand))
    }

    /// Wood, its grain running along x.
    fn wood(&self, d: f32, p: Point, view: &View, seed: u32) -> f32 {
        rough(d, point(p.x * 0.25, p.y, p.z), view, 0.3, 0.04, seed, 0.75)
    }

    /// The giant's table: a top a metre thick on four turned legs, an apron under its edge, and on
    /// it a breadboard with a loaf, a mug and a bundle of herbs.
    fn table(&self, p: Point, view: &View) -> Answer {
        let q = p - TABLE;
        let bound = cuboid(q - point(0.0, 5.0, 0.0), point(12.6, 5.6, 7.6));
        if bound > 0.5 {
            return Answer::new(bound, Material::Wood);
        }
        let top = rounded_cuboid(q - point(0.0, TOP - 0.5, 0.0), point(12.0, 0.5, 7.0), 0.12);
        let apron = cut(
            cuboid(q - point(0.0, 6.15, 0.0), point(11.0, 0.55, 6.0)),
            cuboid(q - point(0.0, 6.15, 0.0), point(10.7, 0.8, 5.7)),
        );
        let l = point(abs(q.x) - 10.6, q.y, abs(q.z) - 5.6);
        let block = cuboid(l - point(0.0, 5.85, 0.0), point(0.6, 0.85, 0.6));
        let r = 0.42 + 0.08 * cos(l.y * 1.6);
        let shaft = max(l.flat_length() - r, abs(l.y - 2.6) - 3.2) * 0.9;
        let foot = cuboid(l, point(0.55, 0.6, 0.55));
        let legs = min(min(block, shaft), foot);
        let board = cuboid(q - point(-4.0, TOP + 0.25, -2.0), point(3.2, 0.3, 2.2));
        let wood = min(min(top, apron), min(legs, board));
        let mut a = Answer::new(self.wood(wood, p, view, self.seed ^ 0x3d), Material::Wood);

        let loaf = rounded_cuboid(q - point(-4.2, TOP + 1.3, -2.0), point(2.2, 0.85, 1.2), 0.7);
        a = a.or(Answer::new(rough(loaf, p, view, 0.5, 0.05, self.seed ^ 0x10, 0.75), Material::Soil));

        let m = q - point(5.0, TOP, -1.5);
        let body = cylinder(m - point(0.0, 1.15, 0.0), 1.0, 1.2);
        let hollow = cylinder(m - point(0.0, 2.35, 0.0), 0.78, 0.35);
        let handle = torus(point(m.x - 1.0, m.z, m.y - 1.2), 0.6, 0.16);
        let mug = min(cut(body, hollow), handle);
        a = a.or(Answer::new(rough(mug, p, view, 0.2, 0.02, self.seed ^ 0x12, 0.85), Material::Stone));

        let hb = q - point(-8.5, TOP + 0.45, 3.5);
        let herbs = min(min(sphere(hb, 0.75), sphere(hb - point(0.9, -0.05, 0.5), 0.6)), sphere(hb - point(-0.4, 0.0, 0.9), 0.55));
        a.or(Answer::new(rough(herbs, p, view, 0.3, 0.1, self.seed ^ 0x14, 0.6), Material::Leaf))
    }

    /// The giant's chair, pushed up to the table: a seat a metre thick, 4.5 m up, a back of rails
    /// and slats.
    fn chair(&self, p: Point, view: &View) -> Answer {
        let q = p - CHAIR;
        let bound = cuboid(q - point(0.0, 6.0, 0.3), point(3.6, 6.6, 3.9));
        if bound > 0.5 {
            return Answer::new(bound, Material::Wood);
        }
        let seat = rounded_cuboid(q - point(0.0, SEAT - 0.5, 0.0), point(3.0, 0.5, 3.0), 0.1);
        let x = abs(q.x);
        let front = cuboid(point(x - 2.55, q.y - 2.0, q.z + 2.55), point(0.4, 2.6, 0.4));
        let back = cuboid(point(x - 2.6, q.y - 5.6, q.z - 3.15), point(0.4, 6.2, 0.35));
        let rail = min(
            cuboid(q - point(0.0, 8.0, 3.15), point(2.6, 0.45, 0.3)),
            rounded_cuboid(q - point(0.0, 11.0, 3.15), point(2.95, 0.6, 0.32), 0.15),
        );
        let slat = |c: f32| cuboid(q - point(c, 9.5, 3.15), point(0.3, 1.6, 0.22));
        let slats = min(min(slat(-1.2), slat(0.0)), slat(1.2));
        let d = min(min(seat, min(front, back)), min(rail, slats));
        Answer::new(self.wood(d, p, view, self.seed ^ 0x77), Material::Wood)
    }

    /// A wooden spoon dropped under the table.
    fn spoon(&self, p: Point, view: &View) -> Answer {
        let bound = capsule(p, SPOON_A, SPOON_C, 1.2);
        if bound > 0.5 {
            return Answer::new(bound, Material::Wood);
        }
        let handle = capsule(p, SPOON_A, SPOON_B, 0.22);
        let cup = point(p.x - SPOON_C.x, p.y, p.z - SPOON_C.z);
        let back = intersect(sphere(cup - point(0.0, -0.2, 0.0), 0.95), cup.y - 0.42);
        let dish = cut(back, sphere(cup - point(0.0, 1.35, 0.0), 1.05));
        Answer::new(self.wood(min(handle, dish), p, view, self.seed ^ 0x2c), Material::Wood)
    }

    /// The upturned bowl, and for those small enough to see them, its door and the village in it.
    fn bowl(&self, p: Point, view: &View) -> Answer {
        let c = point(BOWL.x, self.fl - SINK, BOWL.z);
        let ball = sphere(p - c, BOWL_R);
        if ball > 0.4 {
            return Answer::new(ball - 0.1, Material::Wood);
        }
        let foot = torus(p - c - point(0.0, 1.5, 0.0), 0.6, 0.08);
        let dome = min(ball, foot);
        let outer = rough(dome, p, view, 0.04, 0.03, self.seed ^ 0x21, 0.8);
        let seen = view.sees(0.06);
        if seen == 0.0 {
            // Too large to see the door: a dark arch painted where it is, 24 cm wide and 20 cm
            // high, so a person sees there is a way in for someone smaller. Only the material
            // changes; the surface is the same.
            let (dx, dz) = (p.x - BOWL.x, p.z - BOWL.z);
            let (u, v, y) = (dx * DOOR.0 + dz * DOOR.1, dz * DOOR.0 - dx * DOOR.1, p.y - self.fl);
            let arch = u > 1.0 && y > -0.1 && ((v.abs() < 0.12 && y < 0.08) || v * v + (y - 0.08) * (y - 0.08) < 0.0144);
            return Answer::new(outer, if arch { Material::Mud } else { Material::Wood });
        }
        let inner = sphere(p - c, BOWL_R - SHELL);
        let (dx, dz) = (p.x - BOWL.x, p.z - BOWL.z);
        let u = dx * DOOR.0 + dz * DOOR.1;
        let v = dz * DOOR.0 - dx * DOOR.1;
        let y = p.y - self.fl;
        // The door: 2 cm wide, arched, 4.5 cm high, cut from below the floor right through the rim.
        let door = min(
            cuboid(point(u - 1.5, y + 0.0075, v), point(0.15, 0.0425, 0.01)),
            capsule(point(u, y, v), point(1.35, 0.035, 0.0), point(1.65, 0.035, 0.0), 0.01),
        );
        let shell = cut(cut(dome, inner), door);
        let mut tiny = Answer::new(rough(shell, p, view, 0.04, 0.03, self.seed ^ 0x21, 0.8), Material::Wood);
        if inner < 0.1 {
            tiny = tiny.or(self.spool(p, view));
            if y < 0.09 {
                tiny = tiny.or(self.houses(p));
            } else {
                // Above the roofs and chimneys: no nearer to them than this.
                tiny.distance = min(tiny.distance, y - 0.072);
            }
        } else {
            // Outside the hollow: the village is no nearer than it.
            tiny.distance = min(tiny.distance, inner);
        }
        Answer::new(mix(outer, tiny.distance, seen), tiny.material)
    }

    /// The village's tower: a cotton spool wound with green thread.
    fn spool(&self, p: Point, view: &View) -> Answer {
        let q = point(p.x - BOWL.x, p.y - self.fl, p.z - BOWL.z);
        let flanges = min(
            cylinder(q - point(0.0, 0.02, 0.0), 0.2, 0.035),
            cylinder(q - point(0.0, 0.57, 0.0), 0.2, 0.03),
        );
        let ridges = 0.0008 * view.sees(0.006) * sin(q.y * 1500.0);
        let thread = max(q.flat_length() - 0.16 - ridges, abs(q.y - 0.3) - 0.26) * 0.6;
        Answer::new(flanges, Material::Wood).or(Answer::new(thread, Material::Leaf))
    }

    /// The village's houses, one in some of the squares 11 cm wide, clear of the spool, the square
    /// and the lane to the door: walls of wood or stone, roofs of leaves or moss, a doorway each,
    /// and on some a chimney of stone.
    fn houses(&self, p: Point) -> Answer {
        let sp = 0.11;
        let (cx, cz) = ((p.x / sp).floor() as i32, (p.z / sp).floor() as i32);
        // No house beyond these squares is nearer than this.
        let mut best = Answer::new(0.08, Material::Wood);
        for dz in -1..=1 {
            for dx in -1..=1 {
                let (i, j) = (cx.wrapping_add(dx), cz.wrapping_add(dz));
                let mut s = Seeds::of(self.seed as u64, hash(i, 7, j, self.seed) as usize);
                let hx = (i as f32 + s.next(0.25, 0.75)) * sp;
                let hz = (j as f32 + s.next(0.25, 0.75)) * sp;
                let (ox, oz) = (hx - BOWL.x, hz - BOWL.z);
                let pr = (ox * ox + oz * oz).sqrt();
                let u = ox * DOOR.0 + oz * DOOR.1;
                let v = oz * DOOR.0 - ox * DOOR.1;
                let (sx, sz) = (hx - SQUARE.x, hz - SQUARE.z);
                let plaza = (sx * sx + sz * sz).sqrt();
                let skip = s.next(0.0, 1.0) > 0.8;
                if pr > 1.25 || pr < 0.34 || (u > 0.0 && abs(v) < 0.1) || plaza < 0.2 || skip {
                    continue;
                }
                let q = point(p.x - hx, p.y - self.fl, p.z - hz).turned(s.next(0.0, TAU));
                let (w, d, h) = (s.next(0.016, 0.026), s.next(0.02, 0.032), s.next(0.02, 0.032));
                let rh = s.next(0.012, 0.022);
                let body = cuboid(q - point(0.0, h * 0.5 - 0.0075, 0.0), point(w, h * 0.5 + 0.0075, d));
                let doorway = cuboid(q - point(0.0, 0.006, d), point(0.004, 0.0105, 0.003));
                let walls = cut(body, doorway);
                let e = w + 0.004;
                let n = (rh * rh + e * e).sqrt();
                let y = q.y - h;
                let roof = max(max((abs(q.x) * rh + y * e - e * rh) / n, -y - 0.004), abs(q.z) - d - 0.005);
                let wall_mat = if s.below(5) < 2 { Material::Stone } else { Material::Wood };
                let roof_mat = if s.below(3) == 0 { Material::Leaf } else { Material::Moss };
                best = best.or(Answer::new(walls, wall_mat)).or(Answer::new(roof, roof_mat));
                // A chimney through one slope of the roof, rising a little above the ridge.
                if s.below(3) == 0 {
                    let side = w * 0.5;
                    let chimney = cuboid(q - point(side, h + rh * 0.5 + 0.002, -d * 0.4), point(0.0025, rh * 0.5 + 0.006, 0.0025));
                    best = best.or(Answer::new(chimney, Material::Stone));
                }
            }
        }
        best
    }

    /// The hearth: a ring of boulders about a bed of ash and two crossed logs.
    fn hearth(&self, p: Point, view: &View) -> Answer {
        let q = p - point(HEARTH.x, self.hearth_y, HEARTH.z);
        let bound = cylinder(q, 6.3, 2.6);
        if bound > 0.5 {
            return Answer::new(bound, Material::Stone);
        }
        let mut stones = 1000.0;
        for k in 0..7 {
            stones = min(stones, sphere(q - self.stones[k], self.radii[k]));
        }
        let stone = Answer::new(rough(stones, p, view, 0.8, 0.05, self.seed ^ 0x5e, 0.7), Material::Stone);
        let logs = min(
            capsule(q, point(-2.0, 0.4, -0.8), point(2.0, 0.4, 0.8), 0.45),
            capsule(q, point(-1.2, 0.4, 1.8), point(1.4, 0.9, -1.6), 0.4),
        );
        let wood = Answer::new(self.wood(logs, p, view, self.seed ^ 0x6a), Material::Wood);
        let ash = Answer::new(cylinder(q, 2.6, 0.06), Material::Soil);
        stone.or(wood).or(ash)
    }
}
