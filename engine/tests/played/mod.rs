//! An area of `areas/` played natively, as a `Region`: what its module stands for in the tests
//! (POC_PLAN.md, "Tests"), its answers made the engine's own.

use plateau_engine::{Answer, Borders, Description, Edge, Material, Place, Region, Result, Size};
use plateau_sdk::native::Played;

pub struct Native(pub Box<dyn Played>);

/// The area of `areas/<name>/` played natively, with its seed.
pub fn area(name: &str, seed: u64) -> Native {
    let (_, played) = areas_played::AREAS.iter().find(|(n, _)| *n == name).expect("an area of areas/");
    Native(played(seed))
}

impl Region for Native {
    fn sample(&mut self, points: &[[f32; 3]], size: f32, out: &mut Vec<Answer>) -> Result<()> {
        let points: Vec<_> = points.iter().map(|&[x, y, z]| plateau_sdk::point(x, y, z)).collect();
        let mut answers = Vec::new();
        self.0.sample(&points, size, &mut answers);
        out.clear();
        out.extend(answers.iter().map(|a| Answer { distance: a.distance, material: Material::from_number(a.material as u8).unwrap() }));
        Ok(())
    }

    fn describe(&mut self) -> Result<Description> {
        let d = self.0.describe();
        let edge = |e: &plateau_sdk::Edge| Edge { ground: e.ground.clone(), water: e.water };
        let b = &d.borders;
        Ok(Description {
            name: d.name.clone(),
            places: d
                .places
                .iter()
                .map(|p| Place { name: p.name.clone(), size: Size::ALL[p.size as usize], at: [p.at.x, p.at.y, p.at.z] })
                .collect(),
            borders: Borders { north: edge(&b.north), east: edge(&b.east), south: edge(&b.south), west: edge(&b.west) },
        })
    }
}
