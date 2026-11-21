//! Pictures of the walled garden (POC_PLAN.md, step 4), played natively: its fixed views and plans
//! as its description gives them, and its reach maps showing the cave as the tiny's alone and the
//! seat as giants' alone ("Done when"). The pictures themselves are made by `examples/look.rs`.

mod played;

use plateau_engine::picture::{Plan, map, plan, plans, views};
use plateau_engine::reach::World;
use plateau_engine::{Region, START, Size};

const SEED: u64 = 7;

fn garden() -> played::Native {
    played::area("garden", SEED)
}

#[test]
fn the_garden_s_views_and_plans_follow_from_what_it_describes() {
    let mut g = garden();
    let d = g.describe().unwrap();
    let names: Vec<String> = views(&mut g, &d).unwrap().into_iter().map(|(n, _)| n).collect();
    // A giant over the area, each size at the start, each place seen from somewhere clear of it.
    assert_eq!(
        names,
        ["giant-over-the-area", "tiny-at-the-start", "person-at-the-start", "giant-at-the-start", "tiny-the-cave-behind-the-crack", "giant-the-giant-s-seat"]
    );
    let names: Vec<String> = plans(&d).into_iter().map(|(n, _, _)| n).collect();
    assert_eq!(names, ["plan", "plan-tiny-the-cave-behind-the-crack", "plan-giant-the-giant-s-seat"]);
}

#[test]
fn the_cave_s_plan_is_cut_through_the_wall_above_it_and_shows_its_floor() {
    let d = garden().describe().unwrap();
    let (_, cave, _) = plans(&d).into_iter().find(|(n, _, _)| n.contains("cave")).unwrap();
    let side = 64;
    let pixels = plan(&|| Ok(garden()), &cave, side).unwrap();
    let at = |x: f32, z: f32| {
        let (i, j) = cave.pixel(x, z, side).unwrap();
        pixels[j * side + i]
    };
    // At the place: the cave's earthen floor, lit, under the wall cut away. Just south of the cave,
    // in the wall: stone, cut, flat and dark.
    let floor = at(cave.middle[0], cave.middle[1]);
    let wall = at(54.0, 30.6);
    assert!(floor[0] > floor[2] + 20, "the floor is earth, brown: {floor:?}");
    assert!(wall[0].abs_diff(wall[2]) < 12 && wall[0] < floor[0], "the wall is cut stone, grey and darker: {wall:?}");
}

#[test]
fn the_reach_maps_show_the_cave_as_the_tiny_s_alone_and_the_seat_as_giants_alone() {
    let mut g = garden();
    let d = g.describe().unwrap();
    let mut world = World::new(&mut g, &d.places);
    let reaches = world.reaches().unwrap();
    let side = 256;
    let all = plans(&d);
    let bits = |s: Size| 1u8 << s as u8;
    let pixel = |p: &Plan, x: f32, z: f32| {
        let (i, j) = p.pixel(x, z, side).unwrap();
        j * side + i
    };

    // About the cave, cut away above it: what is opened at the place, and nowhere else about it
    // that a person stands, is the tiny's.
    let (_, cave, band) = all.iter().find(|(n, _, _)| n.contains("cave")).unwrap();
    let m = map(&world, &reaches, cave, *band, side);
    let at = pixel(cave, cave.middle[0], cave.middle[1]);
    assert_eq!(m.opens[at], bits(Size::Tiny), "the cave is opened by the tiny alone");
    assert_eq!(m.all[at], bits(Size::Tiny), "and only the tiny stand in it");
    let tiny_only = (0..side * side).filter(|&k| m.opens[k] == bits(Size::Tiny)).count();
    assert!(tiny_only > side * side / 20, "the cave fills a good part of its plan: {tiny_only} pixels");

    // The whole garden from above: the seat is the giants', the start anyone's, the wall's top the
    // giants'.
    let (_, whole, band) = &all[0];
    let m = map(&world, &reaches, whole, *band, side);
    let seat = d.places.iter().find(|p| p.size == Size::Giant).unwrap().at;
    assert_eq!(m.opens[pixel(whole, seat[0], seat[2])], bits(Size::Giant), "the seat is opened by giants alone");
    let start = pixel(whole, START[0] as f32, START[1] as f32);
    assert!(m.all[start] != 0 && m.opens[start] == 0, "anyone stands at the start");
    assert_eq!(m.opens[pixel(whole, 54.0, 40.0)], bits(Size::Giant), "only a giant steps onto the wall");
}




