use std::{collections::VecDeque, marker::PhantomData};

const MAX_UNDO_COUNT: usize = 512;
const UNDO_STACK_STARTING_CAPACITY: usize = MAX_UNDO_COUNT / 2;
const REDO_STACK_STARTING_CAPACITY: usize = MAX_UNDO_COUNT / 4;

#[derive(Debug, Clone)]
pub(crate) struct UndoStack<U: Undoee> {
    undo: VecDeque<U::UndoAction>,
    redo: VecDeque<U::RedoAction>,
    _marker: PhantomData<U>,
}

pub(crate) trait Undoee {
    type UndoAction;
    type RedoAction;
    fn undo(&mut self, action: Self::UndoAction) -> Self::RedoAction;
    fn redo(&mut self, action: Self::RedoAction) -> Self::UndoAction;
}

impl<U: Undoee> UndoStack<U> {
    pub(crate) fn new() -> Self {
        Self {
            undo: VecDeque::with_capacity(UNDO_STACK_STARTING_CAPACITY),
            redo: VecDeque::with_capacity(REDO_STACK_STARTING_CAPACITY),
            _marker: Default::default(),
        }
    }

    pub(crate) fn push(&mut self, action: U::UndoAction) {
        if self.undo.len() == MAX_UNDO_COUNT {
            self.undo.pop_front();
        }
        self.undo.push_back(action);
        self.redo.clear();
    }

    pub(crate) fn undo(&mut self, unduee: &mut U) {
        if let Some(undo) = self.undo.pop_back() {
            let redo = unduee.undo(undo);
            self.redo.push_back(redo);
        }
    }

    pub(crate) fn redo(&mut self, unduee: &mut U) {
        if let Some(redo) = self.redo.pop_back() {
            let undo = unduee.redo(redo);
            self.undo.push_back(undo);
        }
    }
}
