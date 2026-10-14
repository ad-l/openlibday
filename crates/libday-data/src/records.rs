//! Named-column access to Jet table rows.

use libday_formats::{Database, Value};

use crate::DataError;

/// All rows of one table, addressed by case-insensitive column name.
pub struct Records {
    pub table: String,
    columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

impl Records {
    /// Reads `table`; a table missing from the database yields no rows.
    pub fn read(db: &Database, table: &str) -> Result<Self, DataError> {
        let err = |e| DataError::Database(table.to_owned(), e);
        if !db.table_names().any(|n| n.eq_ignore_ascii_case(table)) {
            return Ok(Self {
                table: table.to_owned(),
                columns: Vec::new(),
                rows: Vec::new(),
            });
        }
        let def = db.table(table).map_err(err)?;
        let rows = db.rows(&def).map_err(err)?;
        Ok(Self {
            table: table.to_owned(),
            columns: def.columns.into_iter().map(|c| c.name).collect(),
            rows,
        })
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn has(&self, column: &str) -> bool {
        self.index(column).is_some()
    }

    fn index(&self, column: &str) -> Option<usize> {
        self.columns
            .iter()
            .position(|c| c.eq_ignore_ascii_case(column))
    }

    fn invalid(&self, row: usize, column: &str, what: &str) -> DataError {
        DataError::Invalid {
            table: self.table.clone(),
            message: format!("row {row}, column {column}: {what}"),
        }
    }

    pub fn value(&self, row: usize, column: &str) -> Result<&Value, DataError> {
        let index = self
            .index(column)
            .ok_or_else(|| self.invalid(row, column, "missing column"))?;
        Ok(&self.rows[row][index])
    }

    pub fn int(&self, row: usize, column: &str) -> Result<i64, DataError> {
        self.value(row, column)?
            .as_i64()
            .ok_or_else(|| self.invalid(row, column, "expected an integer"))
    }

    /// Integer, or `default` when the column is absent or NULL.
    pub fn int_or(&self, row: usize, column: &str, default: i64) -> Result<i64, DataError> {
        match self.index(column).map(|i| &self.rows[row][i]) {
            None | Some(Value::Null) => Ok(default),
            Some(_) => self.int(row, column),
        }
    }

    /// Number, or `default` when the column is absent or NULL.
    pub fn float_or(&self, row: usize, column: &str, default: f64) -> Result<f64, DataError> {
        match self.index(column).map(|i| &self.rows[row][i]) {
            None | Some(Value::Null) => Ok(default),
            Some(v) => v
                .as_f64()
                .ok_or_else(|| self.invalid(row, column, "expected a number")),
        }
    }

    pub fn flag(&self, row: usize, column: &str) -> Result<bool, DataError> {
        self.value(row, column)?
            .as_bool()
            .ok_or_else(|| self.invalid(row, column, "expected a Boolean"))
    }

    pub fn flag_or(&self, row: usize, column: &str, default: bool) -> Result<bool, DataError> {
        match self.index(column).map(|i| &self.rows[row][i]) {
            None | Some(Value::Null) => Ok(default),
            Some(_) => self.flag(row, column),
        }
    }

    /// Text; NULL reads as empty.
    pub fn text(&self, row: usize, column: &str) -> Result<String, DataError> {
        match self.value(row, column)? {
            Value::Null => Ok(String::new()),
            value => value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| self.invalid(row, column, "expected text")),
        }
    }

    pub fn text_or(&self, row: usize, column: &str) -> Result<String, DataError> {
        if self.has(column) {
            self.text(row, column)
        } else {
            Ok(String::new())
        }
    }
}
