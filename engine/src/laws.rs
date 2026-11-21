//! The laws an area must keep (CONCEPT.md, "Laws"), searched for rather than proved. Step 1 has the
//! first; step 3 brings the rest.

use crate::wasm::{Compiled, Limits};
use crate::{Answer, Region, Result, SIZES};

/// **NaNs unseen**, the law that stands in for NaN canonicalization (spike/RESULTS.md): the area
/// gives the same answers, bit for bit, with every NaN made the one NaN and with the machine's own.
/// A NaN's bits differ from one processor to the next (on x86 the machine's NaN is not the one NaN,
/// so the difference shows here); an area whose answers depend on them would make a different world
/// on another machine. On failure, says the first point and size where the answers differ.
pub fn nans_unseen(module: &[u8], seed: u64, points: &[[f32; 3]]) -> Result<std::result::Result<(), String>> {
    let mut one = Compiled::new(module, Limits { canonical_nans: true, ..Limits::default() })?.instantiate(seed)?;
    let mut own = Compiled::new(module, Limits::default())?.instantiate(seed)?;
    let (mut a, mut b) = (Vec::new(), Vec::new());
    for &(size, height) in &SIZES {
        for batch in points.chunks(4096) {
            one.sample(batch, height, &mut a)?;
            own.sample(batch, height, &mut b)?;
            if let Some(i) = (0..batch.len()).find(|&i| !same(a[i], b[i])) {
                let [x, y, z] = batch[i];
                return Ok(Err(format!(
                    "the area looks at a NaN's bits: at ({x}, {y}, {z}) for a {} ({height} m) it answers {:?} with every NaN made \
                     one and {:?} with this machine's own",
                    size.name(),
                    a[i],
                    b[i]
                )));
            }
        }
    }
    Ok(Ok(()))
}

/// The same answer, bit for bit.
pub fn same(a: Answer, b: Answer) -> bool {
    a.distance.to_bits() == b.distance.to_bits() && a.material == b.material
}
