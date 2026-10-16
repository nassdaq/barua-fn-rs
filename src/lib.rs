//! The host, from inside a Barua Function.
//!
//! A function has no network of its own. What it has is a host, and the host
//! has your database and the request. Three things live here:
//!
//! - [`request`]: the HTTP request this call is answering - method, path,
//!   query, content type, headers - read from the environment the host sets
//!   for the call.
//! - [`query`], [`execute`], [`query_as`], [`query_one`]: your database, over
//!   the `DATABASE_URL` set on the function's page at barua.tz.
//! - [`status`] and [`header`]: the HTTP status and headers of the answer.
//!   Without them a function that exits 0 answers 200, with a content type
//!   guessed from what it printed.
//!
//! ```no_run
//! use barua_fn::{execute, query, request, status};
//! use serde_json::json;
//!
//! let req = request();
//! if req.path != "/visits" { status(404); println!("{{\"error\":\"not found\"}}"); return Ok(()); }
//! execute("create table if not exists visits (who text, at timestamptz default now())", &[])?;
//! execute("insert into visits (who) values ($1)", &[json!(req.query_param("who").unwrap_or_default())])?;
//! let rows = query("select count(*)::int as n from visits", &[])?;
//! println!("{}", rows[0]["n"]);
//! # Ok::<(), barua_fn::Error>(())
//! ```
//!
//! Outside Barua Functions (a native build of the same crate) the database
//! calls return [`Error`] and [`status`]/[`header`] do nothing, so the same
//! code can keep a real client and a real server for the native build.
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::BTreeMap;
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
        /// Sets the HTTP status of this call's answer.
        pub fn status(code: i32) -> i32;
        /// Sets a response header; 0 when accepted.
        pub fn header(name_ptr: *const u8, name_len: i32, value_ptr: *const u8, value_len: i32) -> i32;
    }
}

// ---- the request ------------------------------------------------------------------

/// The HTTP request this call is answering.
///
/// The body is on stdin; the rest is here. A call made from the console, or
/// straight at the runtime, has `method` `POST`, `path` `/` and no headers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Request {
    /// `GET`, `POST`, `PUT`, `PATCH`, `DELETE`…
    pub method: String,
    /// The path after the function's URL: `/`, `/books/45`.
    pub path: String,
    /// The raw query string without the `?`: `q=upendo&limit=5`.
    pub query: String,
    /// The request's `Content-Type`, or empty.
    pub content_type: String,
    /// Where the call came from: `url`, `console`, `api`.
    pub source: String,
    /// The function's id on the runtime.
    pub function: String,
    /// The caller's headers, lower-cased; never `authorization` or `cookie`.
    pub headers: BTreeMap<String, String>,
}

impl Request {
    /// The path split on `/`, without empties: `/books/45` → `["books", "45"]`.
    pub fn segments(&self) -> Vec<&str> {
        self.path.split('/').filter(|s| !s.is_empty()).collect()
    }

    /// Every query parameter, decoded, in order. Repeated keys repeat.
    pub fn query_params(&self) -> Vec<(String, String)> {
        self.query
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|p| match p.split_once('=') {
                Some((k, v)) => (decode(k), decode(v)),
                None => (decode(p), String::new()),
            })
            .collect()
    }

    /// The first value of a query parameter, decoded.
    pub fn query_param(&self, key: &str) -> Option<String> {
        self.query_params().into_iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// A request header, by lower-case name.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_ascii_lowercase()).map(String::as_str)
    }
}

/// `%20` and `+` undone.
fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() + 0 && i + 2 <= bytes.len() - 1 + 1 => {
                let hex = |b: u8| (b as char).to_digit(16);
                if let (Some(h), Some(l)) = (bytes.get(i + 1).and_then(|b| hex(*b)), bytes.get(i + 2).and_then(|b| hex(*b))) {
                    out.push((h * 16 + l) as u8);
                    i += 2;
                } else {
                    out.push(b'%');
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The request this call is answering.
pub fn request() -> Request {
    let var = |k: &str| std::env::var(k).unwrap_or_default();
    let headers: BTreeMap<String, String> = serde_json::from_str(&var("BARUA_HEADERS")).unwrap_or_default();
    Request {
        method: {
            let m = var("BARUA_METHOD");
            if m.is_empty() { "POST".to_string() } else { m }
        },
        path: {
            let p = var("BARUA_PATH");
            if p.is_empty() { "/".to_string() } else { p }
        },
        query: var("BARUA_QUERY"),
        content_type: var("BARUA_CONTENT_TYPE"),
        source: var("BARUA_SOURCE"),
        function: var("BARUA_FUNCTION"),
        headers,
    }
}

// ---- the answer ------------------------------------------------------------------

/// The HTTP status of this call's answer. Without it, 200 when the function exits 0.
pub fn status(code: u16) {
    #[cfg(target_os = "wasi")]
    unsafe {
        sys::status(code as i32);
    }
    #[cfg(not(target_os = "wasi"))]
    let _ = code;
}

/// A response header: `content-type`, `cache-control`, `location`,
/// `content-disposition`, `content-language`, `etag`, `vary`, or any `x-…`.
/// Returns whether the host accepted it (false for other names, or past 16).
pub fn header(name: &str, value: &str) -> bool {
    #[cfg(target_os = "wasi")]
    {
        unsafe { sys::header(name.as_ptr(), name.len() as i32, value.as_ptr(), value.len() as i32) == 0 }
    }
    #[cfg(not(target_os = "wasi"))]
    {
        let _ = (name, value);
        false
    }
}

// ---- the database ----------------------------------------------------------------

/// Run one statement with `$1`, `$2`… parameters and get its rows.
///
/// Any statement works - `SELECT`, `INSERT … RETURNING`, DDL - and more than
/// one statement in a string is fine when there are no parameters, which is
/// also how a transaction is written: `"begin; …; commit;"`.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_strings_decode() {
        let r = Request { query: "q=upendo%20wa&limit=5&flag&name=Asha+Juma".into(), ..Default::default() };
        assert_eq!(r.query_param("q").as_deref(), Some("upendo wa"));
        assert_eq!(r.query_param("limit").as_deref(), Some("5"));
        assert_eq!(r.query_param("flag").as_deref(), Some(""));
        assert_eq!(r.query_param("name").as_deref(), Some("Asha Juma"));
        assert_eq!(r.query_param("none"), None);
    }

    #[test]
    fn paths_split() {
        let r = Request { path: "/books/45/".into(), ..Default::default() };
        assert_eq!(r.segments(), vec!["books", "45"]);
    }
}
