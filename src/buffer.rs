use std::{
    borrow::Cow,
    fs::{self, File},
    hash::{Hash, Hasher},
    io::stdin,
    path::PathBuf,
};

use ahash::AHasher;
use color_eyre::eyre::{bail, eyre};

use crate::{
    CsvTableWidgetStyle, MoveDirection, Selection,
    content::{CellLocation, CellLocationDelta, CellRect, CsvTable},
    undo::{UndoStack, Undoee},
};

#[derive(Debug, Clone)]
pub(crate) struct CsvBuffer {
    pub(crate) visible_cols: usize,
    pub(crate) visible_rows: usize,
    pub(crate) cell_height_wanted: u16,
    pub(crate) cell_width_wanted: u16,
    pub(crate) cell_height: u16,
    pub(crate) cell_width: u16,
    pub(crate) style: CsvTableWidgetStyle,
    pub(crate) top_left_cell_location: CellLocation,
    pub(crate) csv_table: CsvTable,
    pub(crate) selection: Selection,
    pub(crate) selection_yanked: Option<Selection>,
    pub(crate) file: Option<PathBuf>,
    pub(crate) undo_stack: UndoStack<CsvTable>,
    saved_hash: Option<u64>,
}

impl Default for CsvBuffer {
    fn default() -> Self {
        let csv_table = CsvTable::default();
        Self {
            visible_cols: 5,
            visible_rows: 20,
            cell_height_wanted: 1,
            cell_width_wanted: 25,
            cell_height: 0,
            cell_width: 0,
            style: Default::default(),
            top_left_cell_location: Default::default(),
            saved_hash: None,
            csv_table,
            selection: Default::default(),
            selection_yanked: Default::default(),
            file: None,
            undo_stack: UndoStack::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum LoadOption {
    File(PathBuf),
    Stdin,
}

impl CsvBuffer {
    pub(crate) fn load(load_option: LoadOption, delimiter: Option<u8>) -> color_eyre::Result<Self> {
        let (csv_table, file, saved_hash) = match load_option {
            LoadOption::File(path_buf) => {
                let file = File::open(&path_buf)?;
                let csv_table = CsvTable::load(file, delimiter)?;
                let hash = hash_table(&csv_table);
                (csv_table, Some(path_buf), Some(hash))
            }
            LoadOption::Stdin => {
                let stdin = stdin();
                (CsvTable::load(stdin, delimiter)?, None, None)
            }
        };
        let res = Self {
            saved_hash,
            csv_table,
            file,
            ..Default::default()
        };
        Ok(res)
    }

    pub(crate) fn save(
        &mut self,
        file_name: Option<PathBuf>,
        create_new_file: bool,
    ) -> color_eyre::Result<PathBuf> {
        let Some(file_path) = file_name
            .map(Cow::Owned)
            .or_else(|| self.file.as_deref().map(Cow::Borrowed))
        else {
            bail!("Need file name!");
        };

        if !file_path.exists() {
            if create_new_file {
                let parent = file_path
                    .parent()
                    .ok_or_else(|| eyre!("File path invalid!"))?;
                fs::create_dir_all(parent)?;
            } else {
                bail!("File does not exist!");
            }
        }
        let mut file = File::create(&file_path)?;
        self.csv_table.normalize_and_save(&mut file)?;
        self.saved_hash = Some(hash_table(&self.csv_table));
        let file_path = file_path.into_owned();
        self.file = Some(file_path.clone());
        Ok(file_path)
    }

    pub(crate) fn is_dirty(&self) -> bool {
        let Some(saved_hash) = self.saved_hash else {
            return !self.is_empty();
        };

        hash_table(&self.csv_table) != saved_hash
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.csv_table.is_empty()
    }

    pub(crate) fn move_selection(&mut self, direction: MoveDirection, n: usize) {
        self.selection.primary += CellLocationDelta::from_direction(direction, n);
        self.ensure_selection_in_view();
    }

    pub(crate) fn move_selection_to(&mut self, location: CellLocation) {
        self.selection.primary = location;
        self.ensure_selection_in_view();
    }

    pub(crate) fn move_view(&mut self, direction: MoveDirection, n: usize) {
        self.top_left_cell_location += CellLocationDelta::from_direction(direction, n);
    }

    #[expect(unused)]
    pub(crate) fn move_view_to(&mut self, location: CellLocation) {
        self.top_left_cell_location = location;
    }

    pub(crate) fn ensure_selection_in_view(&mut self) {
        let sel = self.selection.primary;

        let col_buffer = (self.visible_cols as f32 * 0.1).max(1.0) as usize;
        let row_buffer = (self.visible_rows as f32 * 0.1).max(1.0) as usize;

        if sel.col < self.top_left_cell_location.col + col_buffer {
            self.top_left_cell_location.col = sel.col.saturating_sub(col_buffer);
        } else if sel.col >= self.top_left_cell_location.col + self.visible_cols - col_buffer {
            self.top_left_cell_location.col = sel.col + col_buffer - self.visible_cols + 1;
        }

        if sel.row < self.top_left_cell_location.row + row_buffer {
            self.top_left_cell_location.row = sel.row.saturating_sub(row_buffer);
        } else if sel.row >= self.top_left_cell_location.row + self.visible_rows - row_buffer {
            self.top_left_cell_location.row = sel.row + row_buffer - self.visible_rows + 1;
        }
    }

    pub(crate) fn center_primary_selection(&mut self) {
        self.top_left_cell_location = self.selection.primary
            - CellLocationDelta {
                x: (self.visible_cols / 2) as isize,
                y: (self.visible_rows / 2) as isize,
            }
    }

    pub(crate) fn recalculate_dimensions(&mut self, available_cols: u16, available_rows: u16) {
        self.visible_rows = (available_rows / self.cell_height_wanted) as usize;
        if self.visible_rows == 0 {
            self.visible_rows = if available_rows == 0 { 0 } else { 1 };
            self.cell_height = available_rows;
        } else {
            self.cell_height = self.cell_height_wanted + available_rows % self.cell_height_wanted;
        }

        self.visible_cols = (available_cols / self.cell_width_wanted) as usize;
        if self.visible_cols == 0 {
            self.visible_cols = if available_cols == 0 { 0 } else { 1 };
            self.cell_width = available_cols;
        } else {
            self.cell_width = self.cell_width_wanted + available_cols % self.cell_width_wanted;
        }
    }

    pub(crate) fn undo(&mut self) {
        self.undo_stack.undo(&mut self.csv_table);
    }

    pub(crate) fn redo(&mut self) {
        self.undo_stack.redo(&mut self.csv_table);
    }
}

impl Undoee for CsvTable {
    type UndoAction = UndoAction;
    type RedoAction = RedoAction;

    fn undo(&mut self, action: Self::UndoAction) -> Self::RedoAction {
        match action {
            UndoAction::ChangeCells {
                mode,
                rect,
                from_values,
            } => {
                let to_values = self.set_rect(rect, from_values);
                if mode == UndoChangeCellMode::Delete {
                    return RedoAction::DeleteCells { rect };
                }
                RedoAction::EditCells { rect, to_values }
            }
            UndoAction::ChangeCell {
                mode,
                cell_location,
                from_value,
            } => {
                let to_value = self.set(cell_location, from_value);
                if mode == UndoChangeCellMode::Delete {
                    return RedoAction::DeleteCell { cell_location };
                }
                RedoAction::EditCell {
                    cell_location,
                    to_value,
                }
            }
        }
    }

    fn redo(&mut self, action: Self::RedoAction) -> Self::UndoAction {
        match action {
            RedoAction::EditCells { to_values, rect } => {
                let from_values = self.set_rect(rect, to_values);
                UndoAction::ChangeCells {
                    mode: UndoChangeCellMode::Edit,
                    rect,
                    from_values,
                }
            }
            RedoAction::EditCell {
                cell_location,
                to_value,
            } => {
                let from_value = self.set(cell_location, to_value);
                UndoAction::ChangeCell {
                    mode: UndoChangeCellMode::Edit,
                    cell_location,
                    from_value,
                }
            }
            RedoAction::DeleteCells { rect } => {
                let from_values = self.delete_rect(rect);
                UndoAction::ChangeCells {
                    mode: UndoChangeCellMode::Edit,
                    rect,
                    from_values,
                }
            }
            RedoAction::DeleteCell { cell_location } => {
                let from_value = self.delete(cell_location);
                UndoAction::ChangeCell {
                    mode: UndoChangeCellMode::Edit,
                    cell_location,
                    from_value,
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum UndoAction {
    ChangeCells {
        mode: UndoChangeCellMode,
        rect: CellRect,
        from_values: Vec<Option<String>>,
    },
    ChangeCell {
        mode: UndoChangeCellMode,
        cell_location: CellLocation,
        from_value: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum UndoChangeCellMode {
    Edit,
    Delete,
}

#[derive(Debug, Clone)]
pub(crate) enum RedoAction {
    EditCells {
        rect: CellRect,
        to_values: Vec<Option<String>>,
    },
    EditCell {
        cell_location: CellLocation,
        to_value: Option<String>,
    },
    DeleteCells {
        rect: CellRect,
    },
    DeleteCell {
        cell_location: CellLocation,
    },
}

fn hash_table(table: &CsvTable) -> u64 {
    let mut hasher = AHasher::default();
    table.hash(&mut hasher);
    hasher.finish()
}
