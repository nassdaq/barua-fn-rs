# barua-fn

Call your database from inside a [Barua Function](https://barua.tz/functions).

```toml
[dependencies]
barua-fn = { git = "https://github.com/nassdaq/barua-fn-rs" }
serde_json = "1"
```

```rust
use barua_fn::{execute, query};
use serde_json::json;

execute("create table if not exists visits (who text, at timestamptz default now())", &[])?;
execute("insert into visits (who) values ($1)", &[json!("Asha")])?;
let rows = query("select count(*)::int as n from visits", &[])?;
```

Set `DATABASE_URL` in the function's environment on its page. The host keeps the
connection; your code just asks. Outside a function every call returns an error,
so a crate that is also a native server keeps its own client there.
