//! The world graph (`WORLD_PLAN.md`, phase F): what each placed object comes from, rests on, and
//! belongs to, as typed relations between nodes.
//!
//! Nodes are the terrain (the base areas with their sculpting), placed elements by id, players by
//! id, the terrain's edits by their place in the edit log, and water bodies by id. A relation is `kind from → to`, at most one of each kind between two nodes, kept in an
//! ordered map, so every machine walks the graph alike and a save writes it the same each time:
//!
//! | Kind | From → to | Written by | Data |
//! |---|---|---|---|
//! | `derived_from` | a piece → the element it was cut from | `Cut`, and the support check (a part come away, a trunk broken at a notch) | the piece's own side of the cut |
//! | `rests_on` | an element → what holds it up (terrain or an element) | `Place`, `Drop`, `Cut`, and the support check (where what falls comes to rest, and what the world changed under), from real contacts | — |
//! | `owned_by` | an element → a player | `Place`; a cut piece inherits its original's owner, not the cutter | — |
//! | `attached_to` | an element → what it is fixed to (terrain or an element): held up by it | `Place` fitted to an opening | — |
//! | `fills` | an element → what the opening it fills is in | `Place` fitted to an opening | where the opening was measured |
//! | `carved_into` | a terrain edit → what it carved (the terrain, elements it cut) | `Sculpt` (a carve) | — |
//! | `flooded_by` | a water body → the carve that let it in | `Sculpt` (a carve that floods) | — |
//! | `uprooted` | an element rooted by its shape → the terrain its roots no longer hold in | the support check, when nothing of its roots touches the terrain (dug round, or carved away) | — |
//!
//! The graph changes only by `World::apply`: each change is a `GraphDelta` (relations added and
//! removed), reported in the action's `Summary` and kept with its undo, which reverts it. Saved as
//! `rel <kind> <from> <to> [data]` lines in `plateau-world 2`, where `3` is element 3, `terrain`
//! the terrain, `@name` a player, `edit:4` the terrain's fifth edit, and `water:2` water body 2.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use wasmtime::{bail, format_err};

use crate::Result;
use crate::cut::ClipPlane;

/// Something a relation joins.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Node {
    /// The base areas with the sculpting over them.
    Terrain,
    /// A placed element, by id (one still in the world, or, as the original of a piece, one that was).
    Element(u32),
    /// A player, by id.
    Player(String),
    /// One of the terrain's edits, by its place in the edit log (from 0).
    Edit(u32),
    /// A water body, by id.
    Water(u32),
}

impl Node {
    pub fn element(self) -> Option<u32> {
        match self {
            Node::Element(id) => Some(id),
            _ => None,
        }
    }

    /// As a save writes it: `3`, `terrain`, `@name`.
    pub fn to_token(&self) -> String {
        match self {
            Node::Terrain => "terrain".into(),
            Node::Element(id) => id.to_string(),
            Node::Player(p) => format!("@{p}"),
            Node::Edit(i) => format!("edit:{i}"),
            Node::Water(i) => format!("water:{i}"),
        }
    }

    pub fn from_token(s: &str) -> Option<Node> {
        if s == "terrain" {
            Some(Node::Terrain)
        } else if let Some(p) = s.strip_prefix('@') {
            valid_player(p).then(|| Node::Player(p.to_string()))
        } else if let Some(i) = s.strip_prefix("edit:") {
            i.parse().ok().map(Node::Edit)
        } else if let Some(i) = s.strip_prefix("water:") {
            i.parse().ok().map(Node::Water)
        } else {
            s.parse().ok().map(Node::Element)
        }
    }
}

impl std::fmt::Display for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Node::Terrain => f.write_str("the terrain"),
            Node::Element(id) => write!(f, "#{id}"),
            Node::Player(p) => write!(f, "{p}"),
            Node::Edit(i) => write!(f, "edit {i}"),
            Node::Water(i) => write!(f, "water body {i}"),
        }
    }
}

fn valid_player(p: &str) -> bool {
    !p.is_empty() && !p.chars().any(char::is_whitespace)
}

/// What kind of relation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    DerivedFrom,
    RestsOn,
    OwnedBy,
    AttachedTo,
    Fills,
    CarvedInto,
    FloodedBy,
    Uprooted,
}

impl Kind {
    pub const ALL: [Kind; 8] = [Kind::DerivedFrom, Kind::RestsOn, Kind::OwnedBy, Kind::AttachedTo, Kind::Fills, Kind::CarvedInto, Kind::FloodedBy, Kind::Uprooted];

    pub fn name(self) -> &'static str {
        match self {
            Kind::DerivedFrom => "derived_from",
            Kind::RestsOn => "rests_on",
            Kind::OwnedBy => "owned_by",
            Kind::AttachedTo => "attached_to",
            Kind::Fills => "fills",
            Kind::CarvedInto => "carved_into",
            Kind::FloodedBy => "flooded_by",
            Kind::Uprooted => "uprooted",
        }
    }

    /// Whether an element is held up by being the `from` of this kind.
    pub fn holds_up(self) -> bool {
        matches!(self, Kind::RestsOn | Kind::AttachedTo)
    }

    pub fn from_name(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == s)
    }

    /// Whether `from → to` makes sense for this kind.
    fn joins(self, from: &Node, to: &Node) -> bool {
        match self {
            Kind::DerivedFrom => matches!((from, to), (Node::Element(a), Node::Element(b)) if a != b),
            Kind::RestsOn | Kind::AttachedTo | Kind::Fills => {
                matches!((from, to), (Node::Element(a), Node::Terrain) | (Node::Element(a), Node::Element(_)) if Node::Element(*a) != *to)
            }
            Kind::OwnedBy => matches!((from, to), (Node::Element(_), Node::Player(_))),
            Kind::CarvedInto => matches!((from, to), (Node::Edit(_), Node::Terrain | Node::Element(_))),
            Kind::FloodedBy => matches!((from, to), (Node::Water(_), Node::Edit(_))),
            Kind::Uprooted => matches!((from, to), (Node::Element(_), Node::Terrain)),
        }
    }
}

/// What a relation carries beside its ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Data {
    #[default]
    None,
    /// The plane a piece was cut along, facing away from the piece (the clip it carries).
    Cut(ClipPlane),
    /// Where an opening was measured: enough to measure it again (`site::SiteProbe`, unseen).
    Opening(Opening),
}

/// An opening as it was pointed at when an element was fitted to it: a point of its wall, the way
/// out of that wall, and the height of whoever measured it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Opening {
    pub target: [f32; 3],
    pub normal: [f32; 3],
    pub size: f32,
}

impl Opening {
    pub fn validate(&self) -> Result<()> {
        if !self.target.iter().chain(&self.normal).all(|c| c.is_finite()) || !(self.size.is_finite() && self.size > 0.0) {
            bail!("an opening must be finite, with a size above nought, got {self:?}");
        }
        Ok(())
    }
}

/// One relation: `kind from → to`, with its data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Relation {
    pub kind: Kind,
    pub from: Node,
    pub to: Node,
    #[serde(default, skip_serializing_if = "is_none_data")]
    pub data: Data,
}

fn is_none_data(d: &Data) -> bool {
    matches!(d, Data::None)
}

impl Relation {
    pub fn new(kind: Kind, from: Node, to: Node) -> Relation {
        Relation { kind, from, to, data: Data::None }
    }

    pub fn derived_from(piece: u32, original: u32, cut: ClipPlane) -> Relation {
        Relation { kind: Kind::DerivedFrom, from: Node::Element(piece), to: Node::Element(original), data: Data::Cut(cut) }
    }

    pub fn rests_on(a: u32, b: Node) -> Relation {
        Relation::new(Kind::RestsOn, Node::Element(a), b)
    }

    pub fn owned_by(id: u32, player: &str) -> Relation {
        Relation::new(Kind::OwnedBy, Node::Element(id), Node::Player(player.to_string()))
    }

    pub fn attached_to(a: u32, host: Node) -> Relation {
        Relation::new(Kind::AttachedTo, Node::Element(a), host)
    }

    pub fn fills(a: u32, host: Node, opening: Opening) -> Relation {
        Relation { kind: Kind::Fills, from: Node::Element(a), to: host, data: Data::Opening(opening) }
    }

    pub fn carved_into(edit: u32, what: Node) -> Relation {
        Relation::new(Kind::CarvedInto, Node::Edit(edit), what)
    }

    pub fn flooded_by(water: u32, edit: u32) -> Relation {
        Relation::new(Kind::FloodedBy, Node::Water(water), Node::Edit(edit))
    }

    pub fn uprooted(id: u32) -> Relation {
        Relation::new(Kind::Uprooted, Node::Element(id), Node::Terrain)
    }

    fn key(&self) -> (Kind, Node, Node) {
        (self.kind, self.from.clone(), self.to.clone())
    }

    pub fn validate(&self) -> Result<()> {
        if !self.kind.joins(&self.from, &self.to) {
            bail!("a `{}` relation cannot join {} to {}", self.kind.name(), self.from, self.to);
        }
        match (self.kind, self.data) {
            (Kind::DerivedFrom, Data::Cut(c)) => c.validate(),
            (Kind::DerivedFrom, _) => bail!("a `derived_from` relation needs its cut"),
            (Kind::Fills, Data::Opening(o)) => o.validate(),
            (Kind::Fills, _) => bail!("a `fills` relation needs its opening"),
            (_, Data::None) => Ok(()),
            (k, _) => bail!("a `{}` relation carries no data", k.name()),
        }
    }

    /// The save's record: `rel <kind> <from> <to> [data]`.
    pub fn to_record(&self) -> String {
        let mut s = format!("rel {} {} {}", self.kind.name(), self.from.to_token(), self.to.to_token());
        match self.data {
            Data::None => {}
            Data::Cut(c) => s.push_str(&format!(" clip {} {} {} {}", c.normal[0], c.normal[1], c.normal[2], c.offset)),
            Data::Opening(o) => {
                let [t, n] = [o.target, o.normal];
                s.push_str(&format!(" opening {} {} {} normal {} {} {} size {}", t[0], t[1], t[2], n[0], n[1], n[2], o.size));
            }
        }
        s
    }

    pub fn parse_record(line: &str, line_no: usize) -> Result<Relation> {
        let tok: Vec<&str> = line.split_whitespace().collect();
        if tok.len() < 4 || tok[0] != "rel" {
            bail!("line {line_no}: expected `rel <kind> <from> <to> [data]`, got `{line}`");
        }
        let kind = Kind::from_name(tok[1]).ok_or_else(|| format_err!("line {line_no}: unknown relation `{}`", tok[1]))?;
        let node = |s: &str| Node::from_token(s).ok_or_else(|| format_err!("line {line_no}: `{s}` is not a node (an element id, `terrain`, or `@player`)"));
        let (from, to) = (node(tok[2])?, node(tok[3])?);
        let f = |s: &str| s.parse::<f32>().map_err(|e| format_err!("line {line_no}: bad number `{s}`: {e}"));
        let data = match &tok[4..] {
            [] => Data::None,
            ["clip", nx, ny, nz, off] => Data::Cut(ClipPlane { normal: [f(nx)?, f(ny)?, f(nz)?], offset: f(off)? }),
            ["opening", tx, ty, tz, "normal", nx, ny, nz, "size", size] => {
                Data::Opening(Opening { target: [f(tx)?, f(ty)?, f(tz)?], normal: [f(nx)?, f(ny)?, f(nz)?], size: f(size)? })
            }
            rest => bail!("line {line_no}: unknown relation data `{}`", rest.join(" ")),
        };
        let rel = Relation { kind, from, to, data };
        rel.validate().map_err(|e| format_err!("line {line_no}: {e}"))?;
        Ok(rel)
    }
}

impl std::fmt::Display for Relation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let verb = match self.kind {
            Kind::DerivedFrom => "was cut from",
            Kind::RestsOn => "rests on",
            Kind::OwnedBy => "is owned by",
            Kind::AttachedTo => "is attached to",
            Kind::Fills => "fills an opening in",
            Kind::CarvedInto => "carved into",
            Kind::FloodedBy => "came in through",
            Kind::Uprooted => "no longer has its roots in",
        };
        write!(f, "{} {verb} {}", self.from, self.to)
    }
}

/// What an action did to the graph: relations added and removed, in the order it did them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GraphDelta {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<Relation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<Relation>,
}

impl GraphDelta {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }

    /// Records `rel` added: if it was removed earlier in the same delta, the two cancel.
    fn note_added(&mut self, rel: Relation) {
        if let Some(i) = self.removed.iter().position(|r| *r == rel) {
            self.removed.remove(i);
        } else {
            self.added.push(rel);
        }
    }

    fn note_removed(&mut self, rel: Relation) {
        if let Some(i) = self.added.iter().position(|r| *r == rel) {
            self.added.remove(i);
        } else {
            self.removed.push(rel);
        }
    }
}

/// The world graph: relations by `(kind, from, to)`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Graph {
    relations: BTreeMap<(Kind, Node, Node), Data>,
}

impl Graph {
    pub fn new() -> Graph {
        Graph::default()
    }

    /// The graph of `relations`, each checked; a relation given twice (same kind and ends) is refused.
    pub fn from_relations(relations: impl IntoIterator<Item = Relation>) -> Result<Graph> {
        let mut g = Graph::new();
        for rel in relations {
            rel.validate()?;
            if g.relations.insert(rel.key(), rel.data).is_some() {
                bail!("relation `{}` given twice", rel.to_record());
            }
        }
        Ok(g)
    }

    pub fn len(&self) -> usize {
        self.relations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.relations.is_empty()
    }

    /// Every relation, in order (by kind, then from, then to).
    pub fn relations(&self) -> impl Iterator<Item = Relation> + '_ {
        self.relations.iter().map(|((kind, from, to), data)| Relation { kind: *kind, from: from.clone(), to: to.clone(), data: *data })
    }

    pub fn contains(&self, rel: &Relation) -> bool {
        self.relations.get(&rel.key()) == Some(&rel.data)
    }

    /// Adds `rel` (replacing one of the same kind and ends), noting what changed in `delta`.
    pub fn add(&mut self, rel: Relation, delta: &mut GraphDelta) {
        match self.relations.insert(rel.key(), rel.data) {
            Some(old) if old == rel.data => return,
            Some(old) => delta.note_removed(Relation { data: old, ..rel.clone() }),
            None => {}
        }
        delta.note_added(rel);
    }

    /// Takes `rel` away (by kind and ends), noting it in `delta` if it was there.
    pub fn remove(&mut self, rel: &Relation, delta: &mut GraphDelta) {
        if let Some(data) = self.relations.remove(&rel.key()) {
            delta.note_removed(Relation { data, ..rel.clone() });
        }
    }

    /// Does `delta` again.
    pub fn redo(&mut self, delta: &GraphDelta, out: &mut GraphDelta) {
        for r in &delta.removed {
            self.remove(r, out);
        }
        for r in &delta.added {
            self.add(r.clone(), out);
        }
    }

    /// Undoes `delta`.
    pub fn undo(&mut self, delta: &GraphDelta, out: &mut GraphDelta) {
        for r in delta.added.iter().rev() {
            self.remove(r, out);
        }
        for r in delta.removed.iter().rev() {
            self.add(r.clone(), out);
        }
    }

    /// Relations of `kind` from `node`, in order.
    pub fn from_node<'a>(&'a self, kind: Kind, node: &'a Node) -> impl Iterator<Item = Relation> + 'a {
        self.relations().filter(move |r| r.kind == kind && r.from == *node)
    }

    /// Relations of `kind` to `node`, in order.
    pub fn to_node<'a>(&'a self, kind: Kind, node: &'a Node) -> impl Iterator<Item = Relation> + 'a {
        self.relations().filter(move |r| r.kind == kind && r.to == *node)
    }

    /// Who owns the element `id`, if anyone.
    pub fn owner_of(&self, id: u32) -> Option<&str> {
        let from = Node::Element(id);
        self.relations.range((Kind::OwnedBy, from.clone(), Node::Terrain)..).take_while(|((k, f, _), _)| *k == Kind::OwnedBy && *f == from).find_map(|((_, _, to), _)| match to {
            Node::Player(p) => Some(p.as_str()),
            _ => None,
        })
    }

    /// What the element `id` rests on, in order.
    pub fn supports_of(&self, id: u32) -> Vec<Node> {
        let from = Node::Element(id);
        self.from_node(Kind::RestsOn, &from).map(|r| r.to).collect()
    }

    /// What rests on the element `id`, in order.
    pub fn resting_on(&self, id: u32) -> Vec<u32> {
        let to = Node::Element(id);
        self.to_node(Kind::RestsOn, &to).filter_map(|r| r.from.element()).collect()
    }

    /// The element `piece` was cut from, and the cut.
    pub fn derived_from(&self, piece: u32) -> Option<(u32, ClipPlane)> {
        let from = Node::Element(piece);
        self.from_node(Kind::DerivedFrom, &from).find_map(|r| match (r.to, r.data) {
            (Node::Element(o), Data::Cut(c)) => Some((o, c)),
            _ => None,
        })
    }

    /// The pieces cut from `original`, in order.
    pub fn pieces_of(&self, original: u32) -> Vec<u32> {
        let to = Node::Element(original);
        self.to_node(Kind::DerivedFrom, &to).filter_map(|r| r.from.element()).collect()
    }

    /// The relations an element leaving the world takes with it: all its own, and everything held up
    /// by it or filling it (`rests_on`, `attached_to`, `fills` onto it). What is history stays: pieces
    /// cut from it keep their `derived_from`, edits that carved it their `carved_into`.
    pub fn leaving(&self, id: u32) -> Vec<Relation> {
        let node = Node::Element(id);
        self.relations().filter(|r| r.from == node || (r.to == node && Graph::rests_on_host(r.kind))).collect()
    }

    /// Kinds that need their `to` where it is: they go when it moves or leaves.
    pub fn rests_on_host(kind: Kind) -> bool {
        matches!(kind, Kind::RestsOn | Kind::AttachedTo | Kind::Fills)
    }

    /// What the element `id` is attached to, in order.
    pub fn attached_of(&self, id: u32) -> Vec<Node> {
        let from = Node::Element(id);
        self.from_node(Kind::AttachedTo, &from).map(|r| r.to).collect()
    }

    /// What is attached to, or fills an opening in, `node`, in order.
    pub fn fixed_to(&self, node: &Node) -> Vec<Relation> {
        self.relations().filter(|r| r.to == *node && matches!(r.kind, Kind::AttachedTo | Kind::Fills)).collect()
    }

    /// What the terrain edit `edit` carved into.
    pub fn carved_by(&self, edit: u32) -> Vec<Node> {
        let from = Node::Edit(edit);
        self.from_node(Kind::CarvedInto, &from).map(|r| r.to).collect()
    }

    /// The edits that carved into `node`, in order.
    pub fn carves_into(&self, node: &Node) -> Vec<u32> {
        self.to_node(Kind::CarvedInto, node).filter_map(|r| match r.from {
            Node::Edit(i) => Some(i),
            _ => None,
        }).collect()
    }

    /// The carves that let water body `water` in.
    pub fn flooded_through(&self, water: u32) -> Vec<u32> {
        let from = Node::Water(water);
        self.from_node(Kind::FloodedBy, &from).filter_map(|r| match r.to {
            Node::Edit(i) => Some(i),
            _ => None,
        }).collect()
    }

    /// The save's `rel` records, one a line.
    pub fn write_records(&self, out: &mut String) {
        for r in self.relations() {
            out.push_str(&r.to_record());
            out.push('\n');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_read_back_and_deltas_undo() {
        let plane = ClipPlane { normal: [0.0, 1.0, 0.0], offset: 0.9 };
        let opening = Opening { target: [1.0, 0.5, 2.0], normal: [0.0, 0.0, 1.0], size: 0.017 };
        let rels = [
            Relation::derived_from(3, 1, plane),
            Relation::rests_on(3, Node::Terrain),
            Relation::owned_by(3, "alice"),
            Relation::attached_to(4, Node::Element(3)),
            Relation::fills(4, Node::Element(3), opening),
            Relation::carved_into(0, Node::Element(3)),
            Relation::flooded_by(1, 0),
        ];
        for r in &rels {
            assert_eq!(&Relation::parse_record(&r.to_record(), 1).unwrap(), r);
        }
        let mut g = Graph::new();
        let mut d = GraphDelta::default();
        for r in &rels {
            g.add(r.clone(), &mut d);
        }
        assert_eq!(g.owner_of(3), Some("alice"));
        assert_eq!(g.supports_of(3), vec![Node::Terrain]);
        assert_eq!(g.derived_from(3), Some((1, plane)));
        assert_eq!(g.attached_of(4), vec![Node::Element(3)]);
        assert_eq!(g.carves_into(&Node::Element(3)), vec![0]);
        assert_eq!(g.flooded_through(1), vec![0]);
        assert_eq!(g.leaving(3).len(), 5, "its own three, and the door fixed to it");
        let mut undone = GraphDelta::default();
        g.undo(&d, &mut undone);
        assert!(g.is_empty());
        g.redo(&d, &mut GraphDelta::default());
        assert_eq!(g.len(), rels.len());
        assert!(Relation::parse_record("rel fills 4 3", 1).is_err(), "a fill names its opening");
        assert!(Relation::parse_record("rel flooded_by water:1 3", 1).is_err(), "water comes in through an edit");
        assert!(Relation::parse_record("rel rests_on 3 3", 1).is_err(), "nothing rests on itself");
        assert!(Relation::parse_record("rel owned_by 3 terrain", 1).is_err());
        assert!(Relation::parse_record("rel derived_from 3 1", 1).is_err(), "a piece names its cut");
    }
}
