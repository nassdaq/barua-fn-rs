//! Call your database from inside a Barua Function.
//!
//! A function has no network of its own. What it has is a host, and the host
//! has your database: set `DATABASE_URL` in the function's environment at
//! barua.tz, and every call below goes to it over a connection the host keeps
//! warm. Rows come back as JSON objects keyed by column.
//!
//! ```no_run
//! use barua_fn::{execute, query};
//! use serde_json::json;
//!
//! execute("create table if not exists visits (who text, at timestamptz default now())", &[])?;
//! execute("insert into visits (who) values ($1)", &[json!("Asha")])?;
//! let rows = query("select count(*)::int as n from visits", &[])?;
//! println!("{}", rows[0]["n"]);
//! # Ok::<(), barua_fn::Error>(())
//! ```
//!
//! Outside Barua Functions (a native build of the same crate) every call
//! returns [`Error`], so the same code can keep a real client for the server.
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::fmt;

/// One row: column name to JSON value.
pub type Row = serde_json::Map<String, Value>;

/// What went wrong, in words meant for the person who wrote the function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

#[cfg(target_os = "wasi")]
mod sys {
    #[link(wasm_import_module = "barua")]
    extern "C" {
        /// Runs the statement; returns the byte length of the JSON answer.
        pub fn query(sql_ptr: *const u8, sql_len: i32, params_ptr: *const u8, params_len: i32) -> i32;
        /// Copies the pending answer into `ptr`; returns the bytes written.
        pub fn read(ptr: *mut u8, len: i32) -> i32;
    }
}

/// Run one statement with `$1`, `$2`… parameters and get its rows.
///
/// Any statement works - `SELECT`, `INSERT … RETURNING`, DDL - and more than
/// one statement in a string is fine when there are no parameters.
pub fn query(sql: &str, params: &[Value]) -> Result<Vec<Row>, Error> {
    #[cfg(target_os = "wasi")]
    {
        let params = serde_json::to_vec(params).map_err(|e| Error(e.to_string()))?;
        let len = unsafe { sys::query(sql.as_ptr(), sql.len() as i32, params.as_ptr(), params.len() as i32) };
        if len <= 0 {
            return Err(Error("the host did not answer".into()));
        }
        let mut buf = vec![0u8; len as usize];
        let got = unsafe { sys::read(buf.as_mut_ptr(), len) };
        buf.truncate(got.max(0) as usize);
        let answer: Value = serde_json::from_slice(&buf).map_err(|e| Error(format!("the host's answer was not JSON: {e}")))?;
        if let Some(err) = answer.get("error").and_then(Value::as_str) {
            return Err(Error(err.to_string()));
        }
        let rows = answer.get("rows").and_then(Value::as_array).cloned().unwrap_or_default();
        Ok(rows
            .into_iter()
            .filter_map(|r| match r {
                Value::Object(m) => Some(m),
                _ => None,
            })
            .collect())
    }
    #[cfg(not(target_os = "wasi"))]
    {
        let _ = (sql, params);
        Err(Error("barua_fn::query only works inside Barua Functions (wasm32-wasip1)".into()))
    }
}

/// Run a statement for its effect: DDL, or an `INSERT`/`UPDATE` without `RETURNING`.
pub fn execute(sql: &str, params: &[Value]) -> Result<(), Error> {
    query(sql, params).map(|_| ())
}

/// Run a statement and deserialize each row into `T`.
pub fn query_as<T: DeserializeOwned>(sql: &str, params: &[Value]) -> Result<Vec<T>, Error> {
    query(sql, params)?
        .into_iter()
        .map(|r| serde_json::from_value(Value::Object(r)).map_err(|e| Error(format!("a row did not fit the type: {e}"))))
        .collect()
}

/// The first row, if any.
pub fn query_one(sql: &str, params: &[Value]) -> Result<Option<Row>, Error> {
    Ok(query(sql, params)?.into_iter().next())
}
