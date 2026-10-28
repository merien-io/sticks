use rusqlite::{params_from_iter, Connection};
use serde_json::{Map, Number, Value};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub fn configure_connection(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(())
}

pub fn init_db(conn: &Connection, base_dir: &Path) -> Result<(), rusqlite::Error> {
    configure_connection(conn)?;

    let schema_path = base_dir.join("data").join("schema.sql");
    if schema_path.exists() {
        if let Ok(sql) = std::fs::read_to_string(&schema_path) {
            conn.execute_batch(&sql)?;
        }
    }
    Ok(())
}

pub fn execute_sql(
    db: &Arc<Mutex<Connection>>,
    sql: &str,
    params: &[rusqlite::types::Value],
) -> Result<usize, String> {
    let conn = db
        .lock()
        .map_err(|e| format!("Database lock poisoned: {e}"))?;
    conn.execute(sql, params_from_iter(params.iter()))
        .map_err(|e| e.to_string())
}

pub fn query_all(
    db: &Arc<Mutex<Connection>>,
    sql: &str,
    params: &[rusqlite::types::Value],
) -> Value {
    let conn = match db.lock() {
        Ok(c) => c,
        Err(e) => {
            let mut err = Map::new();
            err.insert("ok".to_string(), Value::Bool(false));
            err.insert(
                "error".to_string(),
                Value::String(format!("Mutex poisoned: {e}")),
            );
            return Value::Object(err);
        }
    };

    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(e) => {
            let mut err = Map::new();
            err.insert("ok".to_string(), Value::Bool(false));
            err.insert("error".to_string(), Value::String(e.to_string()));
            return Value::Object(err);
        }
    };

    if !stmt.readonly() {
        let mut err = Map::new();
        err.insert("ok".to_string(), Value::Bool(false));
        err.insert(
            "error".to_string(),
            Value::String("Mutating statements are prohibited in db.all queries".to_string()),
        );
        return Value::Object(err);
    }

    let col_names: Vec<String> = stmt.column_names().into_iter().map(String::from).collect();
    let mut results = Vec::new();

    let mut rows = match stmt.query(params_from_iter(params.iter())) {
        Ok(r) => r,
        Err(e) => {
            let mut err = Map::new();
            err.insert("ok".to_string(), Value::Bool(false));
            err.insert("error".to_string(), Value::String(e.to_string()));
            return Value::Object(err);
        }
    };

    while let Ok(Some(row)) = rows.next() {
        let mut map = Map::new();
        for (i, col) in col_names.iter().enumerate() {
            let val = match row.get_ref(i) {
                Ok(rusqlite::types::ValueRef::Null) => Value::Null,
                Ok(rusqlite::types::ValueRef::Integer(v)) => Value::Number(v.into()),
                Ok(rusqlite::types::ValueRef::Real(f)) => {
                    Number::from_f64(f).map(Value::Number).unwrap_or(Value::Null)
                }
                Ok(rusqlite::types::ValueRef::Text(s)) => {
                    Value::String(String::from_utf8_lossy(s).into_owned())
                }
                Ok(rusqlite::types::ValueRef::Blob(_)) => {
                    let mut err = Map::new();
                    err.insert("ok".to_string(), Value::Bool(false));
                    err.insert(
                        "error".to_string(),
                        Value::String("Binary BLOB output not supported".to_string()),
                    );
                    return Value::Object(err);
                }
                Err(e) => {
                    let mut err = Map::new();
                    err.insert("ok".to_string(), Value::Bool(false));
                    err.insert("error".to_string(), Value::String(e.to_string()));
                    return Value::Object(err);
                }
            };
            map.insert(col.clone(), val);
        }
        results.push(Value::Object(map));
    }

    let mut res = Map::new();
    res.insert("ok".to_string(), Value::Bool(true));
    res.insert("value".to_string(), Value::Array(results));
    Value::Object(res)
}