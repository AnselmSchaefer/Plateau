//! Fitting again (`WORLD_PLAN.md`, phase I): an element that fills an opening (`fills`, with where
//! the opening was measured) is held to it again when a change reaches it. The opening is measured
//! anew; an element placed with its site's params is given the new site's, where the new site is,
//! and made again (undone with the change); then it is held to the opening (`fit::cavity_fit`) and,
//! if it no longer fits, whoever made the change is told why.

use crate::edit::Edit;
use crate::fit::cavity_fit;
use crate::graph::{Data, Kind, Node, Opening};
use crate::scene::Combined;
use crate::site::{SiteContext, SiteProbe};

use super::World;
use super::history::WorldAction;

/// How far beyond an element a change may be and still reach the opening it fills, in heights of
/// whoever measured it.
const REACH: f32 = 0.5;

impl World {
    /// Each element filling an opening `edit` reached, fitted to it again: what to tell whoever made
    /// the change (those that no longer fit, and why).
    pub(super) fn refit_openings(&mut self, edit: &Edit) -> Vec<String> {
        let (lo, hi) = edit.influence_bounds();
        let fills: Vec<(u32, Opening)> = self
            .graph
            .relations()
            .filter_map(|r| match (r.kind, r.from, r.data) {
                (Kind::Fills, Node::Element(id), Data::Opening(o)) => Some((id, o)),
                _ => None,
            })
            .collect();
        let mut told = Vec::new();
        for (id, o) in fills {
            let Some((blo, bhi)) = self.world_box(id) else { continue };
            let pad = REACH * o.size;
            if !(0..3).all(|a| hi[a] >= blo[a] - pad && lo[a] <= bhi[a] + pad) {
                continue;
            }
            let Some(before) = self.find(id).cloned() else { continue };
            let Ok(element) = self.catalog.get_or_load(&before.module) else { continue };
            let name = self.element_name(id);
            // Measured, and judged, as if it were not there (in the SDF, if a stroke promoted it, as
            // in the scene): only what it is fitted to.
            let mut without = self.scene.clone();
            without.remove_layer(id);
            let bare = self.composed_without(&[id]);
            let kept = std::mem::replace(&mut self.region.log, bare);
            let probe = SiteProbe { target: o.target, normal: o.normal, from: None, size: o.size };
            let measured = SiteContext::measure(&mut Combined { region: &mut self.region, world: &without }, &probe);
            let site = match measured {
                Ok(site) if site.cavity.is_some() => site,
                Ok(_) => {
                    self.region.log = kept;
                    told.push(format!("{name} fills no opening now: there is none where it was"));
                    continue;
                }
                Err(e) => {
                    self.region.log = kept;
                    told.push(format!("{name}: its opening could not be measured again: {e:#}"));
                    continue;
                }
            };
            // Made for its site's params, it is made again for the site as it is now, where it is
            // now: whole, as it was made (what a stroke carved off it went with the old opening).
            let mut after = before.clone();
            if !before.params.is_empty() && before.clips.is_empty() {
                after.params = site.params();
                after.set_transform(site.frame);
                after.promoted = None;
                after.local_edits.clear();
            }
            let verdict = cavity_fit(&mut self.region, &without, &site, &element, after.seed, &after.params, after.transform());
            self.region.log = kept;
            if after != before {
                self.restore_placed(after.clone());
                self.history.push(WorldAction::Refit { before, after });
            }
            match verdict {
                Ok(Ok(_)) => {}
                Ok(Err(why)) => told.push(format!("{name} no longer fits its opening: {why}")),
                Err(e) => told.push(format!("{name}: it could not be held to its opening: {e:#}")),
            }
        }
        told
    }
}
