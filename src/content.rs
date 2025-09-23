use std::{
    fmt::Display,
    io,
    ops::{Add, AddAssign, Sub, SubAssign},
    path::PathBuf,
};

use color_eyre::eyre::eyre;
use csv::{ReaderBuilder, WriterBuilder};

use crate::MoveDirection;

#[derive(Clone, Debug, Default)]
pub(crate) struct CsvTable {
    pub(crate) delimiter: Option<u8>,
    rows: Vec<Vec<Option<String>>>,
    pub(crate) file: Option<PathBuf>,
}

impl CsvTable {
    pub(crate) fn load_from_file(file: PathBuf, delimiter: Option<u8>) -> color_eyre::Result<Self> {
        let mut builder = ReaderBuilder::new();
        builder.has_headers(false);
        if let Some(delimiter) = delimiter {
            builder.delimiter(delimiter);
        }
        let mut reader = builder.from_path(&file)?;
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
        Ok(Self {
            delimiter,
            file: Some(file),
            rows,
        })
    }

    pub(crate) fn from_stdin(delimiter: Option<u8>) -> color_eyre::Result<Self> {
        let mut builder = ReaderBuilder::new();
        builder.has_headers(false);
        if let Some(delimiter) = delimiter {
            builder.delimiter(delimiter);
        }
        let mut reader = builder.from_reader(io::stdin());
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
        Ok(Self {
            delimiter,
            file: None,
            rows,
        })
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

    pub(crate) fn normalize_and_save(&mut self) -> color_eyre::Result<()> {
        let Some(file) = self.file.clone() else {
            return Err(eyre!("There is no file to write to!"));
        };
        self.normalize();
        let mut builder = WriterBuilder::new();
        if let Some(delimiter) = self.delimiter {
            builder.delimiter(delimiter);
        }
        let mut wtr = builder.from_path(file)?;

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CellLocation {
    pub(crate) row: usize,
    pub(crate) col: usize,
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
