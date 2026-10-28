use crate::dbaccess;
use crate::interpreter::{parse, scan, Context, Node, Val};
use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Debug, Deserialize)]
pub struct Config {
    pub daemon: DaemonConfig,
    pub site: Vec<SiteConfig>,
}

#[derive(Debug, Deserialize)]
pub struct DaemonConfig {
    pub interface: String,
    pub port: u16,
}

#[derive(Debug, Deserialize)]
pub struct SiteConfig {
    #[serde(default)]
    #[allow(dead_code)]
    pub hostname: String,
    pub public_dir: String,
    pub database: DatabaseConfig,
}

#[derive(Debug, Deserialize)]
pub struct DatabaseConfig {
    #[allow(dead_code)]
    pub driver: String,
    pub path: String,
}

pub struct AppState {
    pub db: Arc<Mutex<Connection>>,
    pub canonical_public_dir: PathBuf,
}

pub fn check_templates(public_dir: &Path) -> Result<(), String> {
    if !public_dir.exists() {
        return Err(format!("Public directory does not exist: {public_dir:?}"));
    }
    for entry in walkdir(public_dir)? {
        if entry.extension().map_or(false, |e| e.eq_ignore_ascii_case("html")) {
            let content = fs::read_to_string(&entry)
                .map_err(|e| format!("Failed to read {:?}: {e}", entry))?;
            let tokens = scan(&content)
                .map_err(|e| format!("Scan error in {:?}: {e}", entry))?;
            let _ = parse(tokens)
                .map_err(|e| format!("Parse error in {:?}: {e}", entry))?;
        }
    }
    Ok(())
}

fn walkdir(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let entries = fs::read_dir(dir).map_err(|e| e.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let p = entry.path();
        if p.is_dir() {
            files.extend(walkdir(&p)?);
        } else {
            files.push(p);
        }
    }
    Ok(files)
}

fn resolve_path_safely(root_canon: &Path, url_path: &str) -> Option<PathBuf> {
    let decoded = percent_encoding::percent_decode_str(url_path)
        .decode_utf8()
        .ok()?;

    let mut candidate = root_canon.to_path_buf();
    for seg in decoded.split('/').filter(|s| !s.is_empty()) {
        if seg.starts_with('.') || seg.contains(['\\', '\0', ':']) {
            return None;
        }
        candidate.push(seg);
    }

    if let Ok(canon) = candidate.canonicalize() {
        if canon.starts_with(root_canon) {
            return Some(canon);
        }
    }
    None
}

pub async fn run_server(config_path_str: &str) -> Result<(), Box<dyn std::error::Error>> {
    let config_path = PathBuf::from(config_path_str);
    let mut base_dir = config_path.parent().unwrap_or(Path::new("")).to_path_buf();
    if base_dir.as_os_str().is_empty() {
        base_dir = PathBuf::from(".");
    }

    let config_content = fs::read_to_string(&config_path)?;
    let cfg: Config = toml::from_str(&config_content)?;
    let site = cfg
        .site
        .first()
        .ok_or("Configuration must contain at least one [[site]]")?;

    let canonical_public = base_dir.join(&site.public_dir).canonicalize()?;
    check_templates(&canonical_public)
        .map_err(|e| format!("Template validation error at boot: {e}"))?;

    let db_path = base_dir.join(&site.database.path);
    if let Some(p) = db_path.parent() {
        fs::create_dir_all(p)?;
    }

    let conn = Connection::open(&db_path)?;
    dbaccess::init_db(&conn, &base_dir)?;

    let state = Arc::new(AppState {
        db: Arc::new(Mutex::new(conn)),
        canonical_public_dir: canonical_public,
    });

    let addr = format!("{}:{}", cfg.daemon.interface, cfg.daemon.port);
    println!("🚀 Sticks Server online: http://{}", addr);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, Router::new().fallback(any(handle_request)).with_state(state)).await?;
    Ok(())
}

async fn handle_request(State(state): State<Arc<AppState>>, req: Request<Body>) -> Response {
    let raw_path = req.uri().path();
    let sanitized_path = if raw_path == "/" { "/index.html" } else { raw_path };

    let file_path = match resolve_path_safely(&state.canonical_public_dir, sanitized_path) {
        Some(p) => p,
        None => return (StatusCode::NOT_FOUND, "404 - Not Found").into_response(),
    };

    let is_html = file_path
        .extension()
        .map_or(false, |ext| ext.eq_ignore_ascii_case("html"));

    if !is_html && file_path.exists() {
        if let Ok(bytes) = fs::read(&file_path) {
            let mime = mime_guess::from_path(&file_path).first_or_octet_stream();
            return Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime.as_ref())
                .header("X-Content-Type-Options", "nosniff")
                .body(Body::from(bytes))
                .unwrap();
        }
    }

    let template = match fs::read_to_string(&file_path) {
        Ok(t) => t,
        Err(_) => return (StatusCode::NOT_FOUND, "404 - Not Found").into_response(),
    };

    let ast = match scan(&template).and_then(parse) {
        Ok(a) => a,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Parse Error: {e}"),
            )
                .into_response()
        }
    };

    let (parts, body) = req.into_parts();
    let method = parts.method.to_string();

    let query_map: HashMap<String, String> = parts
        .uri
        .query()
        .and_then(|q| serde_urlencoded::from_str(q).ok())
        .unwrap_or_default();

    let mut cookie_map = HashMap::new();
    if let Some(cookie_hdr) = parts.headers.get(header::COOKIE).and_then(|v| v.to_str().ok()) {
        for cookie in cookie_hdr.split(';') {
            if let Some((k, v)) = cookie.trim().split_once('=') {
                cookie_map.insert(k.to_string(), v.to_string());
            }
        }
    }

    let bytes = match axum::body::to_bytes(body, 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return (StatusCode::BAD_REQUEST, "Payload too large").into_response(),
    };

    let mut body_map = Map::new();
    if let Ok(json_val) = serde_json::from_slice::<Value>(&bytes) {
        if let Value::Object(obj) = json_val {
            body_map = obj;
        }
    } else if let Ok(form_data) = serde_urlencoded::from_bytes::<Vec<(String, String)>>(&bytes) {
        for (k, v) in form_data {
            body_map.insert(k, Value::String(v));
        }
    }

    let mut req_obj = Map::new();
    req_obj.insert(
        "query".to_string(),
        Value::Object(query_map.into_iter().map(|(k, v)| (k, Value::String(v))).collect()),
    );
    req_obj.insert("body".to_string(), Value::Object(body_map));
    req_obj.insert("method".to_string(), Value::String(method.clone()));

    let mut ctx = Context {
        vars: HashMap::new(),
        cookies: cookie_map,
        set_cookies: Vec::new(),
        db: Arc::clone(&state.db),
        redirect: None,
        allow_effects: true,
    };
    ctx.vars.insert("$req".to_string(), Val::Json(Value::Object(req_obj)));

    let mut view_start_idx = ast.len();

    // Effect Zone Pass
    for (idx, node) in ast.iter().enumerate() {
        match node {
            Node::Statements(stmts) => {
                if ctx.execute_statements(stmts).is_break() {
                    break;
                }
            }
            Node::Handler { method: m, stmts } => {
                if *m == method {
                    if ctx.execute_statements(stmts).is_break() {
                        break;
                    }
                }
            }
            Node::Text(t) if t.trim().is_empty() => {}
            _ => {
                view_start_idx = idx;
                break;
            }
        }
    }

    if let Some(target) = ctx.redirect.take() {
        let mut builder = Response::builder()
            .status(StatusCode::SEE_OTHER)
            .header(header::LOCATION, target);
        for sc in ctx.set_cookies {
            builder = builder.header(header::SET_COOKIE, sc);
        }
        return builder.body(Body::empty()).unwrap();
    }

    ctx.allow_effects = false;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<String, std::io::Error>>(32);
    let view_nodes = ast[view_start_idx..].to_vec();
    let set_cookies = ctx.set_cookies.clone();

    tokio::task::spawn_blocking(move || {
        for node in view_nodes {
            match node {
                Node::Text(t) => {
                    if tx.blocking_send(Ok(t)).is_err() {
                        break;
                    }
                }
                Node::Print(expr) => {
                    let out = ctx.eval_expr(&expr).to_escaped_html();
                    if tx.blocking_send(Ok(out)).is_err() {
                        break;
                    }
                }
                Node::If { cond, body, else_body } => {
                    if ctx.eval_expr(&cond).is_truthy() {
                        let out = ctx.interpolate(&body);
                        if tx.blocking_send(Ok(out)).is_err() {
                            break;
                        }
                    } else if let Some(el) = else_body {
                        let out = ctx.interpolate(&el);
                        if tx.blocking_send(Ok(out)).is_err() {
                            break;
                        }
                    }
                }
                Node::For { var, iter, body, else_body } => {
                    let mut executed = false;
                    let target = ctx.eval_expr(&iter);
                    if let Val::Json(Value::Array(arr)) = target {
                        let original_binding = ctx.vars.get(&var).cloned();
                        for item in arr {
                            executed = true;
                            ctx.vars.insert(var.clone(), Val::Json(item));
                            let out = ctx.interpolate(&body);
                            if tx.blocking_send(Ok(out)).is_err() {
                                return;
                            }
                        }
                        if let Some(prev) = original_binding {
                            ctx.vars.insert(var, prev);
                        } else {
                            ctx.vars.remove(&var);
                        }
                    }
                    if !executed {
                        if let Some(el) = else_body {
                            let out = ctx.interpolate(&el);
                            if tx.blocking_send(Ok(out)).is_err() {
                                break;
                            }
                        }
                    }
                }
                Node::Statements(stmts) => {
                    let _ = ctx.execute_statements(&stmts);
                }
                Node::Handler { .. } => {}
            }
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header("X-Content-Type-Options", "nosniff");

    for sc in set_cookies {
        builder = builder.header(header::SET_COOKIE, sc);
    }

    builder.body(Body::from_stream(stream)).unwrap()
}