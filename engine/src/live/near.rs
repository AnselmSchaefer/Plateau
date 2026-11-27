//! What is round about a site, from the world graph (`WORLD_PLAN.md`, phase F): what the site is in
//! and who owns it, what that was cut from, the carves made there and the water let in through
//! them, whether the opening is filled already, and the elements near by. In words, for whoever
//! writes an element for the site (`SiteContext::near`, `describe_for_prompt`), with places in the
//! site's local metres.

use super::World;
use crate::graph::{Kind, Node, Opening};
use crate::site::SiteContext;

/// How far round a site to look, in the measuring player's heights.
const ROUND: f32 = 6.0;
/// The most elements named near by.
const MOST: usize = 8;

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

fn to_box(p: [f32; 3], lo: [f32; 3], hi: [f32; 3]) -> f32 {
    let d = [0, 1, 2].map(|a| (lo[a] - p[a]).max(0.0).max(p[a] - hi[a]));
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

fn list(ids: &[u32], word: &str) -> String {
    let shown: Vec<String> = ids.iter().take(MOST).map(|i| format!("{word} {i}")).collect();
    let more = if ids.len() > MOST { format!(" and {} more", ids.len() - MOST) } else { String::new() };
    format!("{}{more}", shown.join(", "))
}

impl World {
    /// Where the element `id` is in the world: the box round its solid (or its SDF), if it has one.
    pub(super) fn world_box(&self, id: u32) -> Option<([f32; 3], [f32; 3])> {
        let pl = self.find(id)?;
        if let Some(b) = self.sdf_bounds_of(pl) {
            return Some(pl.transform().world_bounds(b));
        }
        let phys = self.physics_of(id)?;
        let tf = pl.transform();
        let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
        for x in [phys.local_lo[0], phys.local_hi[0]] {
            for y in [phys.local_lo[1], phys.local_hi[1]] {
                for z in [phys.local_lo[2], phys.local_hi[2]] {
                    let p = tf.local_to_world_point([x, y, z]);
                    for a in 0..3 {
                        lo[a] = lo[a].min(p[a]);
                        hi[a] = hi[a].max(p[a]);
                    }
                }
            }
        }
        Some((lo, hi))
    }

    fn node_words(&self, n: &Node) -> String {
        match n {
            Node::Element(id) => format!("{} (#{id})", self.element_name(*id)),
            other => other.to_string(),
        }
    }

    /// The element `id` in words: what it is, who owns it, what it was cut from, how it has been
    /// carved, and what holds it up.
    pub fn element_words(&self, id: u32) -> String {
        let Some(pl) = self.find(id) else { return format!("#{id}, no longer in the world") };
        let mut s = format!("{} (#{id})", self.element_name(id));
        if self.is_sdf_element(pl) {
            s += ", a landform";
        } else if pl.is_promoted() {
            s += ", carved into, so solid as an SDF (it can still be chopped and carried)";
        }
        if let Some(owner) = self.owner_of(id) {
            s += &format!("; owned by {owner}");
        }
        let mut lineage = Vec::new();
        let mut at = id;
        while let Some((orig, _)) = self.graph.derived_from(at) {
            lineage.push(orig);
            if lineage.len() > 16 {
                break;
            }
            at = orig;
        }
        if !lineage.is_empty() {
            let chain: Vec<String> = lineage.iter().map(|o| format!("#{o}")).collect();
            s += &format!("; a piece cut from {}", chain.join(", itself cut from "));
        }
        let own = pl.local_edits.len();
        let terrain = self.graph.carves_into(&Node::Element(id));
        if own > 0 || !terrain.is_empty() {
            let mut parts = Vec::new();
            if own > 0 {
                parts.push(format!("{own} times in its own frame (that carving goes where it goes)"));
            }
            if !terrain.is_empty() {
                parts.push(format!("by {}", list(&terrain, "terrain edit")));
            }
            s += &format!("; carved {}", parts.join(" and "));
        }
        let held: Vec<String> = self
            .graph
            .relations()
            .filter(|r| r.from == Node::Element(id) && r.kind.holds_up())
            .map(|r| format!("{} {}", if r.kind == Kind::RestsOn { "rests on" } else { "is attached to" }, self.node_words(&r.to)))
            .collect();
        match self.footing(id) {
            Some(super::Footing::Rooted) => s += "; rooted where it stands",
            Some(super::Footing::Floating) => s += "; afloat",
            _ => {}
        }
        if !held.is_empty() {
            s += &format!("; {}", held.join(", "));
        }
        s
    }

    /// The scene element whose solid mesh `p` is on (within a tenth of `size`), the nearest.
    fn scene_surface_at(&self, p: [f32; 3], size: f32) -> Option<u32> {
        let reach = (0.1 * size).max(1e-4);
        let lo = [p[0] - reach, p[1] - reach, p[2] - reach];
        let hi = [p[0] + reach, p[1] + reach, p[2] + reach];
        let mut best: Option<(f32, u32)> = None;
        for owner in self.scene.solid_placed_overlapping(lo, hi) {
            if let Some(d) = self.scene.distance_to_solids_of(owner, p, reach)
                && best.is_none_or(|(b, _)| d < b)
            {
                best = Some((d, owner));
            }
        }
        best.map(|(_, id)| id)
    }

    /// What is round about `site` in the world, in words (see the module).
    pub fn neighbourhood(&mut self, site: &SiteContext) -> Vec<String> {
        let size = site.size;
        let frame = site.frame;
        let local = |m: f32| m / frame.scale;
        let mut out = Vec::new();

        // What the site is in, and whether its opening is filled already.
        let opening = Opening { target: site.target, normal: site.normal, size };
        let host = self.host_at(opening);
        // What is pointed at may be a scene element's own surface (a door set into a trunk) with
        // something else solid behind it: say what it is on, then what that is in.
        let on = self.scene_surface_at(site.target, size);
        if let Some(id) = on {
            out.push(format!("The site is on {}.", self.element_words(id)));
        }
        match &host {
            Some(Node::Element(id)) if on == Some(*id) => {}
            Some(Node::Element(id)) => out.push(format!("{} {}.", if on.is_some() { "That is in" } else { "The site is in" }, self.element_words(*id))),
            Some(Node::Terrain) => out.push(format!(
                "{} the terrain: the area's own ground, with what has been sculpted there.",
                if on.is_some() { "That is in" } else { "The site is in" }
            )),
            _ => {}
        }
        // Filled already: by whatever was fitted to an opening measured here (pointed at now, it may
        // be the door itself that is seen).
        let fills: Vec<_> = self.graph.relations().filter(|r| r.kind == Kind::Fills).collect();
        for r in fills {
            let Some(id) = r.from.clone().element() else { continue };
            let on_it = self.world_box(id).is_some_and(|(lo, hi)| to_box(site.target, lo, hi) <= size);
            if let crate::graph::Data::Opening(o) = r.data
                && (on_it || dist(o.target, site.target) < 2.0 * size.max(o.size))
            {
                out.push(format!("This opening is filled already, by {}, fitted into {}.", self.node_words(&Node::Element(id)), self.node_words(&r.to)));
            }
        }

        // The carves made round about, and the water let in through them.
        let round = ROUND * size;
        let carves: Vec<u32> = self
            .edits
            .edits()
            .iter()
            .enumerate()
            .filter(|(_, e)| e.op == crate::edit::Op::Carve && dist(e.center, site.target) <= round + e.shape.radius())
            .map(|(i, _)| i as u32)
            .collect();
        if !carves.is_empty() {
            let sizes: Vec<f32> = carves.iter().map(|&i| self.edits.edits()[i as usize].stamp_size).collect();
            let (lo, hi) = sizes.iter().fold((f32::INFINITY, 0.0f32), |(lo, hi), &s| (lo.min(s), hi.max(s)));
            let made = if hi <= lo * 1.01 { crate::laws::length(lo) } else { format!("{} to {}", crate::laws::length(lo), crate::laws::length(hi)) };
            let mut into: Vec<String> = Vec::new();
            for &i in &carves {
                for n in self.graph.carved_by(i) {
                    let w = self.node_words(&n);
                    if !into.contains(&w) {
                        into.push(w);
                    }
                }
            }
            let into = if into.is_empty() { String::new() } else { format!(", into {}", into.join(", ")) };
            out.push(format!(
                "{} carve{} within {:.2} m of it, made by someone {made} tall{into} ({}).",
                carves.len(),
                if carves.len() == 1 { "" } else { "s" },
                local(round),
                list(&carves, "terrain edit")
            ));
        }
        for body in self.water.bodies() {
            let through: Vec<u32> = self.graph.flooded_through(body.id).into_iter().filter(|i| carves.contains(i)).collect();
            if !through.is_empty() {
                out.push(format!(
                    "Water came into the carves here (water body {}, its surface at local y = {:.3}), let in through {}.",
                    body.id,
                    local(body.level - frame.pos[1]),
                    list(&through, "terrain edit")
                ));
            }
        }

        // The elements near by, nearest first.
        let host_id = host.as_ref().and_then(|h| h.clone().element());
        let mut near: Vec<(f32, u32)> = self
            .placed
            .iter()
            .filter(|pl| Some(pl.id) != host_id && Some(pl.id) != on)
            .filter_map(|pl| self.world_box(pl.id).map(|(lo, hi)| (to_box(site.target, lo, hi), pl.id)))
            .filter(|(d, _)| *d <= round)
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        for &(d, id) in near.iter().take(MOST) {
            let p = frame.world_to_local_point(self.find(id).map_or(site.target, |pl| pl.pos));
            out.push(format!("Near by: {}, {:.2} m off, standing at local ({:.2}, {:.2}, {:.2}).", self.element_words(id), local(d), p[0], p[1], p[2]));
        }
        if near.len() > MOST {
            out.push(format!("And {} more elements within {:.2} m.", near.len() - MOST, local(round)));
        }
        out
    }
}
