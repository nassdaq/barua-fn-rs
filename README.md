# barua-fn

The host, from inside a [Barua Function](https://barua.tz/functions): the request, your database, and the shape of your answer.

```toml
[dependencies]
barua-fn = { git = "https://github.com/nassdaq/barua-fn-rs" }
serde_json = "1"
```

```rust
use barua_fn::{execute, header, query, request, status};
use serde_json::json;

let req = request();                       // method, path, query, content_type, headers
match (req.method.as_str(), req.segments().as_slice()) {
    ("GET", ["visits"]) => {
        let rows = query("select who, at from visits order by at desc limit $1", &[json!(10)])?;
        println!("{}", serde_json::to_string(&rows)?);
    }
    ("POST", ["visits"]) => {
        execute("insert into visits (who) values ($1)", &[json!(req.query_param("who").unwrap_or_default())])?;
        status(201);
        println!("{{\"ok\":true}}");
    }
    _ => {
        status(404);
        header("content-type", "application/json");
        println!("{{\"error\":\"not found\"}}");
    }
}
```

- **The body is stdin**, byte for byte. The rest of the request is in `request()`: `method`, `path` (under the function's URL), `query` with `query_param()`, `content_type`, `headers` (lower-cased, never the caller's credentials), `source`.
- **The database** is the `DATABASE_URL` set on the function's page: `query`, `execute`, `query_one`, `query_as::<T>`. Parameters are `$1`… bound to JSON values; rows are JSON objects keyed by column (`timestamptz` → ISO string, `int8` → string, `bytea` → base64). A transaction is one string, `"begin; …; commit;"`, rolled back if any statement fails. 4 s per statement, 4 MB per answer.
- **The answer** is what you print. Exit 0 → 200 (JSON if it looks like JSON, else text); `status(n)` chooses the status; `header(name, value)` sets `content-type`, `cache-control`, `location`, `content-disposition`, `content-language`, `etag`, `vary` or any `x-…`. A non-zero exit → 500 with stderr as the message.

Outside a function (a native build of the same crate) the database calls return `Error` and `status`/`header` do nothing, so one crate can be both the server and the function.

The whole contract, drawn: https://barua.app/docs/functions/inside/#contract
