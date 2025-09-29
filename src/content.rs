use std::{
    fmt::Display,
    io::{Read, Write},
    ops::{Add, AddAssign, Sub, SubAssign},
};

use csv::{ReaderBuilder, WriterBuilder};

use crate::MoveDirection;

#[derive(Clone, Debug, Default)]
pub(crate) struct CsvTable {
    pub(crate) delimiter: Option<u8>,
    rows: Vec<Vec<Option<String>>>,
}

impl CsvTable {
    pub(crate) fn load(read: impl Read, delimiter: Option<u8>) -> color_eyre::Result<Self> {
        let mut builder = ReaderBuilder::new();
        builder.has_headers(false);
        if let Some(delimiter) = delimiter {
            builder.delimiter(delimiter);
        }
        let mut reader = builder.from_reader(read);
        let mut rows: Vec<Vec<Option<String>>> = Vec::new();

        for result in reader.records() {
            let record = result?;
            rows.push(
                record
                    .iter()
                    .map(|s| (!s.is_empty()).then(|| s.to_owned()))
                    .collect(),
            );
        }
        Ok(Self { delimiter, rows })
    }

    pub(crate) fn get(&self, location: CellLocation) -> Option<&str> {
        self.rows.get(location.row)?.get(location.col)?.as_deref()
    }

    #[must_use]
    pub(crate) fn set(&mut self, location: CellLocation, value: Option<String>) -> Option<String> {
        let CellLocation { row, col } = location;
        // Ensure, that columns and rows exist
        if self.rows.len() <= row {
            self.rows.resize_with(row + 1, Vec::new);
        }
        let row = &mut self.rows[row];

        if row.len() <= col {
            row.resize(col + 1, None);
        }

        let old_value = row[col].take();
        let value = value.filter(|value| !value.is_empty());

        // We can just set the cell, because we ensured, that it exists
        row[col] = value;
        old_value
    }

    #[allow(unused)]
    pub(crate) fn get_rect(&self, rect: CellRect) -> Vec<Option<&str>> {
        let CellRect {
            top_left_cell_location,
            col_count,
            row_count,
        } = rect;
        let mut result = Vec::with_capacity(col_count * row_count);

        for row_offset in 0..row_count {
            let row_index = top_left_cell_location.row + row_offset;
            let row = self.rows.get(row_index);

            for col_offset in 0..col_count {
                let col_index = top_left_cell_location.col + col_offset;
                let value = row
                    .and_then(|r| r.get(col_index))
                    .and_then(|cell| cell.as_deref());
                result.push(value);
            }
        }
        result
    }

    pub(crate) fn get_rect_cloned(&self, rect: CellRect) -> Vec<Option<String>> {
        let CellRect {
            top_left_cell_location,
            col_count,
            row_count,
        } = rect;
        let mut result = Vec::with_capacity(col_count * row_count);

        for row_offset in 0..row_count {
            let row_index = top_left_cell_location.row + row_offset;
            let row = self.rows.get(row_index);

            for col_offset in 0..col_count {
                let col_index = top_left_cell_location.col + col_offset;
                let value = row.and_then(|r| r.get(col_index)).cloned().flatten();
                result.push(value);
            }
        }
        result
    }

    #[must_use]
    pub(crate) fn set_rect(
        &mut self,
        rect: CellRect,
        new_values: impl IntoIterator<Item = Option<String>>,
    ) -> Vec<Option<String>> {
        let CellRect {
            top_left_cell_location,
            col_count,
            row_count,
        } = rect;

        let mut old_values = Vec::with_capacity(rect.col_count * rect.row_count);

        // Ensure enough rows
        let required_rows = top_left_cell_location.row + row_count;
        if self.rows.len() < required_rows {
            self.rows.resize_with(required_rows, Vec::new);
        }

        let mut values_iter = new_values.into_iter();

        for row_offset in 0..row_count {
            let row_index = top_left_cell_location.row + row_offset;
            let row = &mut self.rows[row_index];

            // Ensure enough columns in this row
            let required_cols = top_left_cell_location.col + col_count;
            if row.len() < required_cols {
                row.resize(required_cols, None);
            }

            for col_offset in 0..col_count {
                let col_index = top_left_cell_location.col + col_offset;

                let new_value = values_iter
                    .next()
                    .expect("iteration count must match new_values.len()");
                let old_value = row[col_index].take();
                let new_value = new_value.filter(|v| !v.is_empty());

                row[col_index] = new_value;
                old_values.push(old_value);
            }
        }

        old_values
    }
    #[allow(unused)]
    pub(crate) fn delete(&mut self, cell_location: CellLocation) -> Option<String> {
        self.set(cell_location, None)
    }

    #[allow(unused)]
    pub(crate) fn delete_rect(&mut self, rect: CellRect) -> Vec<Option<String>> {
        self.set_rect(rect, std::iter::repeat(None))
    }

    pub(crate) fn fill_rect(
        &mut self,
        rect: CellRect,
        value: Option<String>,
    ) -> Vec<Option<String>> {
        self.set_rect(rect, std::iter::repeat(value))
    }

    pub(crate) fn normalize(&mut self) {
        // Finde die letzte gesetzte Zeile und Spalte
        let mut last_row = 0;
        let mut last_col = 0;

        for (r_idx, row) in self.rows.iter().enumerate() {
            for (c_idx, cell) in row.iter().enumerate() {
                if cell.is_some() {
                    last_row = last_row.max(r_idx);
                    last_col = last_col.max(c_idx);
                }
            }
        }

        // shorten rows-Vec
        self.rows.truncate(last_row + 1);

        // shorten or lengthen each row
        for row in &mut self.rows {
            row.resize(last_col + 1, None);
        }
    }

    pub(crate) fn normalize_and_save(&mut self, write: &mut impl Write) -> color_eyre::Result<()> {
        self.normalize();
        let mut builder = WriterBuilder::new();
        if let Some(delimiter) = self.delimiter {
            builder.delimiter(delimiter);
        }
        let mut wtr = builder.from_writer(write);

        for row in &self.rows {
            let record: Vec<&str> = row
                .iter()
                .map(|c| c.as_deref().unwrap_or_default())
                .collect();
            wtr.write_record(&record)?;
        }

        wtr.flush()?;
        Ok(())
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.rows
            .iter()
            .all(|row| row.iter().all(|cell| cell.is_none()))
    }
}

impl std::hash::Hash for CsvTable {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.delimiter.hash(state);
        for (row_idx, row) in self.rows.iter().enumerate() {
            for (col_idx, cell) in row.iter().enumerate() {
                if let Some(value) = cell {
                    row_idx.hash(state);
                    col_idx.hash(state);
                    value.hash(state);
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CellRect {
    pub(crate) top_left_cell_location: CellLocation,
    pub(crate) col_count: usize,
    pub(crate) row_count: usize,
}

impl CellRect {
    pub(crate) fn from_opposite_cell_locations(
        corner: CellLocation,
        corner_opposite: CellLocation,
    ) -> CellRect {
        let CellLocation { row, col } = corner;
        let CellLocation {
            row: row_opposite,
            col: col_opposite,
        } = corner_opposite;

        let (top_row, bottom_row) = if row < row_opposite {
            (row, row_opposite)
        } else {
            (row_opposite, row)
        };
        let (left_col, right_col) = if col < col_opposite {
            (col, col_opposite)
        } else {
            (col_opposite, col)
        };

        CellRect {
            top_left_cell_location: CellLocation {
                row: top_row,
                col: left_col,
            },
            col_count: right_col - left_col + 1,
            row_count: bottom_row - top_row + 1,
        }
    }

    pub(crate) fn contains(&self, location: CellLocation) -> bool {
        let top_row = self.top_left_cell_location.row;
        let left_col = self.top_left_cell_location.col;

        let bottom_row = top_row + self.row_count.saturating_sub(1);
        let right_col = left_col + self.col_count.saturating_sub(1);

        (top_row..=bottom_row).contains(&location.row)
            && (left_col..=right_col).contains(&location.col)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CellLocation {
    pub(crate) row: usize,
    pub(crate) col: usize,
}

impl CellLocation {
    pub(crate) fn col_index_to_id(mut col: usize) -> String {
        let mut col_str = String::new();

        loop {
            let rem = col % 26;
            col_str.insert(0, (b'A' + rem as u8) as char);
            if col < 26 {
                break;
            }
            col = col / 26 - 1;
        }
        col_str
    }

    pub(crate) fn row_index_to_id(row: usize) -> String {
        (row + 1).to_string()
    }

    pub(crate) fn get_column_count(self, opposite: CellLocation) -> usize {
        self.col.abs_diff(opposite.col) + 1
    }
}

impl Display for CellLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let CellLocation { row, mut col } = *self;
        let mut col_str = String::new();

        loop {
            let rem = col % 26;
            col_str.insert(0, (b'A' + rem as u8) as char);
            if col < 26 {
                break;
            }
            col = col / 26 - 1;
        }
        write!(f, "{}{}", col_str, row + 1)
    }
}

impl Add<CellLocation> for CellLocation {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            row: self.row.saturating_add(rhs.row),
            col: self.col.saturating_add(rhs.col),
        }
    }
}

impl Sub<CellLocation> for CellLocation {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            row: self.row.saturating_sub(rhs.row),
            col: self.col.saturating_sub(rhs.col),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CellLocationDelta {
    pub(crate) x: isize,
    pub(crate) y: isize,
}

impl CellLocationDelta {
    pub(crate) fn from_direction(direction: MoveDirection, n: usize) -> Self {
        let n = n as isize;
        match direction {
            MoveDirection::Left => Self { x: -n, y: 0 },
            MoveDirection::Down => Self { x: 0, y: n },
            MoveDirection::Up => Self { x: 0, y: -n },
            MoveDirection::Right => Self { x: n, y: 0 },
        }
    }
}

impl Add<CellLocationDelta> for CellLocation {
    type Output = CellLocation;

    fn add(self, rhs: CellLocationDelta) -> Self::Output {
        let col = if rhs.x < 0 {
            self.col.saturating_sub(-rhs.x as usize)
        } else {
            self.col.saturating_add(rhs.x as usize)
        };
        let row = if rhs.y < 0 {
            self.row.saturating_sub(-rhs.y as usize)
        } else {
            self.row.saturating_add(rhs.y as usize)
        };
        Self { col, row }
    }
}

impl Sub<CellLocationDelta> for CellLocation {
    type Output = CellLocation;

    fn sub(self, rhs: CellLocationDelta) -> Self::Output {
        let col = if rhs.x < 0 {
            self.col.saturating_add(-rhs.x as usize)
        } else {
            self.col.saturating_sub(rhs.x as usize)
        };
        let row = if rhs.y < 0 {
            self.row.saturating_add(-rhs.y as usize)
        } else {
            self.row.saturating_sub(rhs.y as usize)
        };
        Self { col, row }
    }
}

impl AddAssign<CellLocationDelta> for CellLocation {
    fn add_assign(&mut self, rhs: CellLocationDelta) {
        if rhs.x < 0 {
            self.col = self.col.saturating_sub(-rhs.x as usize);
        } else {
            self.col = self.col.saturating_add(rhs.x as usize);
        };
        if rhs.y < 0 {
            self.row = self.row.saturating_sub(-rhs.y as usize);
        } else {
            self.row = self.row.saturating_add(rhs.y as usize);
        };
    }
}

impl SubAssign<CellLocationDelta> for CellLocation {
    fn sub_assign(&mut self, rhs: CellLocationDelta) {
        if rhs.x < 0 {
            self.col = self.col.saturating_add(-rhs.x as usize);
        } else {
            self.col = self.col.saturating_sub(rhs.x as usize);
        };
        if rhs.y < 0 {
            self.row = self.row.saturating_add(-rhs.y as usize);
        } else {
            self.row = self.row.saturating_sub(rhs.y as usize);
        };
    }
}

impl Add<CellLocationDelta> for CellLocationDelta {
    type Output = CellLocationDelta;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            x: self.x.saturating_add(rhs.x),
            y: self.y.saturating_add(rhs.y),
        }
    }
}

impl Sub<CellLocationDelta> for CellLocationDelta {
    type Output = CellLocationDelta;

    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            x: self.x.saturating_sub(rhs.x),
            y: self.y.saturating_sub(rhs.y),
        }
    }
}
