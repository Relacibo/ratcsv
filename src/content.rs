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

    pub(crate) fn set(&mut self, location: CellLocation, value: Option<String>) {
        let CellLocation { row, col } = location;
        // Ensure, that columns and rows exist
        if self.rows.len() <= row {
            self.rows.resize_with(row + 1, Vec::new);
        }

        let row = &mut self.rows[row];

        if row.len() <= col {
            row.resize(col + 1, None);
        }

        let value = value.filter(|value| !value.is_empty());

        // We can just set the cell, because we ensured, that it exists
        row[col] = value;
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

    pub(crate) fn in_rect(self, corner_a: CellLocation, corner_b: CellLocation) -> bool {
        let CellLocation {
            row: row_a,
            col: col_a,
        } = corner_a;
        let CellLocation {
            row: row_b,
            col: col_b,
        } = corner_b;

        let (row_start, row_end) = if row_a < row_b {
            (row_a, row_b)
        } else {
            (row_b, row_a)
        };

        let (col_start, col_end) = if col_a < col_b {
            (col_a, col_b)
        } else {
            (col_b, col_a)
        };

        self.row >= row_start && self.row <= row_end && self.col >= col_start && self.col <= col_end
    }

    pub(crate) fn rect_iter(self, opposite: CellLocation) -> impl Iterator<Item = CellLocation> {
        let CellLocation {
            row: row_a,
            col: col_a,
        } = self;
        let CellLocation {
            row: row_b,
            col: col_b,
        } = opposite;

        let (row_start, row_end) = if row_a < row_b {
            (row_a, row_b)
        } else {
            (row_b, row_a)
        };

        let (col_start, col_end) = if col_a < col_b {
            (col_a, col_b)
        } else {
            (col_b, col_a)
        };

        (row_start..=row_end)
            .flat_map(move |r| (col_start..=col_end).map(move |c| CellLocation { row: r, col: c }))
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
