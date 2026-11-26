//! Undo and redo (`WORLD_PLAN.md`, phase E): what each change did, kept so it can be undone, across
//! sculpting, promotion, placing, chopping and moving.

use crate::compose::{PlacedElement, PlacementTransform};
use crate::edit::Edit;

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

/// Changes to undo, most recent last, and those undone, to redo.
#[derive(Clone, Debug, Default)]
pub struct History {
    pub undo: Vec<WorldAction>,
    pub redo: Vec<WorldAction>,
}

impl History {
    /// A new change: undoable, and what was undone is no longer redone.
    pub fn push(&mut self, action: WorldAction) {
        self.undo.push(action);
        self.redo.clear();
    }
}
