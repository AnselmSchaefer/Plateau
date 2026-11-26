//! Undo and redo (`WORLD_PLAN.md`, phase E): what each change did, kept so it can be undone, across
//! sculpting, promotion, placing, chopping and moving.

use crate::compose::{PlacedElement, PlacementTransform};
use crate::edit::Edit;
use crate::graph::GraphDelta;

/// One change, as undo and redo need it.
#[derive(Clone, Debug)]
pub enum WorldAction {
    /// A stamp on the terrain, and the scene elements it promoted (id, edit index).
    SdfEdit { edit: Edit, promoted_ids: Vec<(u32, usize)> },
    /// A stamp on a placed element, in its frame, and whether it promoted it (at that edit index).
    ElementSdfEdit { id: u32, edit: Edit, newly_promoted: Option<usize> },
    PlaceElement(PlacedElement),
    RemoveElement(PlacedElement),
    /// An element chopped into a lower and an upper piece.
    CutElement { original: PlacedElement, lower: PlacedElement, upper: PlacedElement },
    RelocateElement { id: u32, before: PlacementTransform, after: PlacementTransform },
}

/// A change kept for undo: what it did to the world, and to the graph.
#[derive(Clone, Debug)]
pub struct Done {
    pub action: WorldAction,
    pub graph: GraphDelta,
}

/// Changes to undo, most recent last, and those undone, to redo.
#[derive(Clone, Debug, Default)]
pub struct History {
    pub undo: Vec<Done>,
    pub redo: Vec<Done>,
    /// Changes pushed, ever: so `World::apply` knows whether the action it applied pushed one, to
    /// give it the action's graph delta.
    pub pushed: u64,
}

impl History {
    /// A new change: undoable, and what was undone is no longer redone. Its graph delta is given
    /// once the action is done (`World::apply`).
    pub fn push(&mut self, action: WorldAction) {
        self.undo.push(Done { action, graph: GraphDelta::default() });
        self.redo.clear();
        self.pushed += 1;
    }
}
