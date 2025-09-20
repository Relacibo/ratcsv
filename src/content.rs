use std::{
    ops::{Add, Sub},
    path::PathBuf,
};

use csv::{ReaderBuilder, Writer};

#[derive(Clone, Debug)]
pub(crate) struct CsvTable {
    rows: Vec<Vec<Option<String>>>,
    file: PathBuf,
}

impl CsvTable {
    pub(crate) fn load_from_file(file: PathBuf) -> color_eyre::Result<Self> {
        let mut reader = ReaderBuilder::new().has_headers(false).from_path(&file)?;

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
        Ok(Self { file, rows })
    }

    pub(crate) fn get(&self, location: CellLocation) -> Option<&str> {
        self.rows.get(location.row)?.get(location.col)?.as_deref()
    }

    pub(crate) fn set(&mut self, location: CellLocation, value: String) {
        let CellLocation { row, col } = location;
        // Ensure, that columns and rows exist
        if self.rows.len() <= row {
            self.rows.resize_with(row + 1, Vec::new);
        }

        let row = &mut self.rows[row];

        if row.len() <= col {
            row.resize(col + 1, None);
        }

        let value = (!value.is_empty()).then_some(value);

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
        self.normalize();
        let mut wtr = Writer::from_path(&self.file)?;

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

impl Add<CellLocation> for CellLocation {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            row: self.row + rhs.row,
            col: self.col + rhs.col,
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

impl Add<CellLocationDelta> for CellLocation {
    type Output = CellLocation;

    fn add(self, rhs: CellLocationDelta) -> Self::Output {
        let col = if rhs.x < 0 {
            self.col.saturating_sub(-rhs.x as usize)
        } else {
            self.col + (rhs.x as usize)
        };
        let row = if rhs.y < 0 {
            self.row.saturating_sub(-rhs.y as usize)
        } else {
            self.row + (rhs.y as usize)
        };
        Self { col, row }
    }
}

impl Sub<CellLocationDelta> for CellLocation {
    type Output = CellLocation;

    fn sub(self, rhs: CellLocationDelta) -> Self::Output {
        let col = if rhs.x < 0 {
            self.col + (-rhs.x as usize)
        } else {
            self.col.saturating_sub(rhs.x as usize)
        };
        let row = if rhs.y < 0 {
            self.row + (-rhs.y as usize)
        } else {
            self.row.saturating_sub(rhs.y as usize)
        };
        Self { col, row }
    }
}

impl Add<CellLocationDelta> for CellLocationDelta {
    type Output = CellLocationDelta;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
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
