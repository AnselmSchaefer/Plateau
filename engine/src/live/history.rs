//! Undo and redo (`WORLD_PLAN.md`, phase E): what each change did, kept so it can be undone, across
//! sculpting, promotion, placing, chopping and moving.

use crate::Material;
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
    /// An element fallen (phase G): as it was, and as it came to rest (with the terrain's carves
    /// into it its own, if it was promoted).
    Fell { before: PlacedElement, after: PlacedElement },
    /// An element fitted again to the opening it fills, once a change reached it (phase I): as it
    /// was, and as it was made for the opening as it is now (new params, where the new site is).
    Refit { before: PlacedElement, after: PlacedElement },
    /// Several changes one action made (a cut, then what came apart and fell), in order.
    Many(Vec<WorldAction>),
}

/// A change kept for undo: what it did to the world, to the graph, and to player inventories.
#[derive(Clone, Debug)]
pub struct Done {
    pub action: WorldAction,
    pub graph: GraphDelta,
    /// Signed material mass deltas `(player_id, material, delta_kg)` applied by this action.
    pub inventory: Vec<(String, Material, f32)>,
    /// It settled further what the change before it left unsettled (`Action::Settle`): undone and
    /// redone with that one.
    pub continues: bool,
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
    /// A new change: undoable, and what was undone is no longer redone. Its graph and inventory
    /// deltas are given once the action is done (`World::apply`).
    pub fn push(&mut self, action: WorldAction) {
        self.undo.push(Done { action, graph: GraphDelta::default(), inventory: Vec::new(), continues: false });
        self.redo.clear();
        self.pushed += 1;
    }

    /// The last `n` changes pushed made one, undone and redone together.
    pub fn join_last(&mut self, n: usize) {
        let n = n.min(self.undo.len());
        if n < 2 {
            return;
        }
        let tail = self.undo.split_off(self.undo.len() - n);
        self.undo.push(Done {
            action: WorldAction::Many(tail.into_iter().map(|d| d.action).collect()),
            graph: GraphDelta::default(),
            inventory: Vec::new(),
            continues: false,
        });
    }
}
