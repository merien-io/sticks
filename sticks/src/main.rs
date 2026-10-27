#![allow(dead_code)]
#![allow(unused_variables)]

use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use clap::{Parser as ClapParser, Subcommand};
use rusqlite::Connection;
use serde::Deserialize;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

// ==========================================
// CONFIGURATION
// ==========================================
#[derive(Debug, Deserialize)]
struct Config {
    daemon: DaemonConfig,
    site: Vec<SiteConfig>,
}

#[derive(Debug, Deserialize)]
struct DaemonConfig { interface: String, port: u16 }

#[derive(Debug, Deserialize)]
struct SiteConfig {
    #[serde(default)] hostname: String,
    public_dir: String,
    database: DatabaseConfig,
}

#[derive(Debug, Deserialize)]
struct DatabaseConfig { driver: String, path: String }

// ==========================================
// LEXER
// ==========================================
#[derive(Debug, Clone, PartialEq)]
pub enum RawToken { Text(String), Block(String) }

pub fn scan(input: &str) -> Result<Vec<RawToken>, String> {
    let mut tokens = Vec::new();
    let mut chars = input.char_indices().peekable();
    let bytes = input.as_bytes();
    let mut text_buf = String::new();
    
    while let Some(&(i, ch)) = chars.peek() {
        if ch == '|' && i + 1 < bytes.len() && bytes[i + 1] == b'|' {
            if !text_buf.is_empty() {
                tokens.push(RawToken::Text(text_buf.clone()));
                text_buf.clear();
            }
            chars.next(); chars.next();
            
            let mut block_buf = String::new();
            let mut closed = false;
            while let Some(&(_, c)) = chars.peek() {
                if c == '|' { chars.next(); closed = true; break; } 
                else { block_buf.push(c); chars.next(); }
            }
            if !closed { return Err(format!("Unterminated block starting at byte {}", i)); }
            tokens.push(RawToken::Block(block_buf));
        } else {
            text_buf.push(ch);
            chars.next();
        }
    }
    if !text_buf.is_empty() { tokens.push(RawToken::Text(text_buf)); }
    Ok(tokens)
}

// ==========================================
// AST & PARSER
// ==========================================
#[derive(Debug, Clone)]
pub enum Node {
    Text(String),
    Print(String),
    If { cond: String, body: String, else_node: Option<Box<Node>> },
    For { var: String, iter: String, body: String, else_body: Option<String> },
    Handler { method: String, stmts: String },
    Statements(String),
}

pub fn parse(tokens: Vec<RawToken>) -> Result<Vec<Node>, String> {
    let mut ast = Vec::new();
    let mut i = 0;
    
    while i < tokens.len() {
        match &tokens[i] {
            RawToken::Text(t) => ast.push(Node::Text(t.clone())),
            RawToken::Block(b) => {
                let trimmed = b.trim();
                
                if trimmed.starts_with("if ") {
                    let head_end = trimmed.find(':').ok_or("Expected ':' in 'if' block")?;
                    let head = &trimmed[3..head_end].trim();
                    let body = &trimmed[head_end + 1..];
                    
                    let mut else_node = None;
                    if i + 1 < tokens.len() {
                        if let RawToken::Block(next_b) = &tokens[i + 1] {
                            let nb_trim = next_b.trim();
                            if nb_trim.starts_with("else:") {
                                else_node = Some(Box::new(Node::Text(nb_trim[5..].to_string())));
                                i += 1;
                            }
                        }
                    }
                    ast.push(Node::If { cond: head.to_string(), body: body.to_string(), else_node });
                } 
                else if trimmed.starts_with("for ") {
                    let head_end = trimmed.find(':').ok_or("Expected ':' in 'for' block")?;
                    let head = &trimmed[4..head_end].trim();
                    let body = &trimmed[head_end + 1..];
                    
                    let parts: Vec<&str> = head.split_whitespace().collect();
                    if parts.len() < 4 || parts[2] != "in" { return Err("Invalid for loop syntax".to_string()); }
                    
                    let mut else_body = None;
                    if i + 1 < tokens.len() {
                        if let RawToken::Block(next_b) = &tokens[i + 1] {
                            let nb_trim = next_b.trim();
                            if nb_trim.starts_with("else:") {
                                else_body = Some(nb_trim[5..].to_string());
                                i += 1;
                            }
                        }
                    }
                    ast.push(Node::For { var: parts[1].to_string(), iter: parts[3].to_string(), body: body.to_string(), else_body });
                }
                else if trimmed.starts_with("on ") {
                    let head_end = trimmed.find(':').ok_or("Expected ':' in 'on' block")?;
                    let method = &trimmed[3..head_end].trim();
                    let body_str = &trimmed[head_end + 1..];
                    ast.push(Node::Handler { method: method.to_string(), stmts: body_str.to_string() });
                }
                else {
                    if trimmed.contains(';') || trimmed.starts_with("type ") {
                        ast.push(Node::Statements(trimmed.to_string()));
                    } else {
                        ast.push(Node::Print(trimmed.to_string()));
                    }
                }
            }
        }
        i += 1;
    }
    Ok(ast)
}

// ==========================================
// RUNTIME EVALUATOR
// ==========================================
#[derive(Clone, Debug)]
pub enum Val {
    Null,
    String(String),
    Int(i64),
    Bool(bool),
    Json(serde_json::Value),
}

impl PartialEq for Val {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Val::Bool(a), Val::Bool(b)) => a == b,
            (Val::Bool(a), Val::Json(serde_json::Value::Bool(b))) => a == b,
            (Val::Json(serde_json::Value::Bool(a)), Val::Bool(b)) => a == b,
            (Val::String(a), Val::String(b)) => a == b,
            (Val::String(a), Val::Json(serde_json::Value::String(b))) => a == b,
            (Val::Json(serde_json::Value::String(a)), Val::String(b)) => a == b,
            (Val::Int(a), Val::Int(b)) => a == b,
            (Val::Null, Val::Null) => true,
            _ => false,
        }
    }
}

impl Val {
    pub fn to_escaped_string(&self) -> String {
        match self {
            Val::Null => "".to_string(),
            Val::String(s) => s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;"),
            Val::Int(i) => i.to_string(),
            Val::Bool(b) => b.to_string(),
            Val::Json(j) => {
                if let serde_json::Value::String(s) = j {
                    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
                } else {
                    j.to_string().replace('<', "&lt;")
                }
            }
        }
    }
}

pub struct Context {
    pub vars: HashMap<String, Val>,
    pub cookies: HashMap<String, String>,
    pub set_cookies: Vec<String>,
    pub db: Arc<Mutex<Connection>>,
    pub req_method: String,
    pub redirect: Option<String>,
    pub status: StatusCode,
}

impl Context {
    pub fn execute_statements(&mut self, script: &str) {
        let mut rest = script.trim();
        while !rest.is_empty() {
            rest = rest.trim();
            if rest.is_empty() { break; }
            
            if rest.starts_with("type ") || rest.starts_with("input ") {
                if let Some(semi) = rest.find(';') {
                    rest = &rest[semi + 1..];
                    continue;
                } else {
                    break;
                }
            }

            // Parse: if <cond> { <body> }
            if rest.starts_with("if ") {
                if let Some(brace_open) = rest.find('{') {
                    let cond_str = rest[3..brace_open].trim();
                    let mut depth = 0;
                    let mut brace_close = None;
                    for (idx, ch) in rest[brace_open..].char_indices() {
                        if ch == '{' { depth += 1; }
                        else if ch == '}' {
                            depth -= 1;
                            if depth == 0 {
                                brace_close = Some(brace_open + idx);
                                break;
                            }
                        }
                    }
                    
                    if let Some(close_idx) = brace_close {
                        let body_str = &rest[brace_open + 1..close_idx];
                        rest = &rest[close_idx + 1..];

                        let cond_val = self.eval_expr(cond_str);
                        if matches!(cond_val, Val::Bool(true)) {
                            self.execute_statements(body_str);
                        }
                        continue;
                    }
                }
            }

            let (stmt, remainder) = match rest.find(';') {
                Some(idx) => (&rest[..idx], &rest[idx + 1..]),
                None => (rest, ""),
            };
            rest = remainder;
            let s = stmt.trim();
            if s.is_empty() { continue; }

            let is_assignment = if let Some((lhs, _)) = s.split_once('=') {
                let lhs = lhs.trim();
                !lhs.contains('(') && !lhs.contains(')') && !lhs.contains('"') && !lhs.contains('\'') &&
                !lhs.contains('>') && !lhs.contains('<') && !lhs.contains('!') &&
                (lhs.starts_with('$') || (lhs.contains(' ') && lhs.split_whitespace().last().unwrap_or("").starts_with('$')))
            } else {
                false
            };

            if is_assignment {
                let (left, right) = s.split_once('=').unwrap();
                let left = left.trim();
                let right = right.trim();
                let var_name = if left.contains(' ') {
                    left.split_whitespace().last().unwrap()
                } else {
                    left
                };
                let val = self.eval_expr(right);
                self.vars.insert(var_name.to_string(), val);
            } else {
                self.eval_expr(s);
            }
        }
    }

    pub fn eval_expr(&mut self, expr: &str) -> Val {
        let expr = expr.trim();
        if expr == "null" { return Val::Null; }
        if expr == "true" { return Val::Bool(true); }
        if expr == "false" { return Val::Bool(false); }
        if let Ok(i) = expr.parse::<i64>() { return Val::Int(i); }
        
        if expr.len() >= 2 && expr.starts_with('"') && expr.ends_with('"') {
            return Val::String(expr[1..expr.len() - 1].to_string());
        }

        // Sessions & Cookies
        if expr.starts_with("session.set(") && expr.ends_with(')') && expr.len() >= 13 {
            let inner = &expr[12..expr.len() - 1];
            let args = split_args(inner);
            if args.len() >= 2 {
                let key = args[0].trim().trim_matches('"');
                let val = self.eval_expr(&args[1]).to_escaped_string();
                self.cookies.insert(key.to_string(), val.clone());
                self.set_cookies.push(format!("{key}={val}; Path=/; SameSite=Lax"));
            }
            return Val::Null;
        }

        if expr.starts_with("session.get(") && expr.ends_with(')') && expr.len() >= 13 {
            let inner = &expr[12..expr.len() - 1];
            let key = inner.trim().trim_matches('"');
            if let Some(val) = self.cookies.get(key) {
                return Val::String(val.clone());
            }
            return Val::Null;
        }

        if expr == "session.clear()" {
            for (k, _) in &self.cookies {
                self.set_cookies.push(format!("{k}=; Path=/; Max-Age=0; SameSite=Lax"));
            }
            self.cookies.clear();
            return Val::Null;
        }

        // Equality & Operations
        if let Some((left, right)) = expr.split_once(" == ") { return Val::Bool(self.eval_expr(left) == self.eval_expr(right)); }
        if let Some((left, right)) = expr.split_once(" != ") { return Val::Bool(self.eval_expr(left) != self.eval_expr(right)); }
        if let Some((left, right)) = expr.split_once(" + ") {
            let l = self.eval_expr(left).to_escaped_string();
            let r = self.eval_expr(right).to_escaped_string();
            return Val::String(l + &r);
        }

        if expr.starts_with("len(") && expr.ends_with(')') && expr.len() >= 5 {
            let inner = expr[4..expr.len() - 1].trim();
            if let Val::Json(serde_json::Value::Array(arr)) = self.eval_expr(inner) { return Val::Int(arr.len() as i64); }
            return Val::Int(0);
        }

        if expr.starts_with("redirect(") && expr.ends_with(')') && expr.len() >= 10 {
            let inner = expr[9..expr.len() - 1].trim();
            self.redirect = Some(self.eval_expr(inner).to_escaped_string());
            return Val::Null;
        }

        if expr.starts_with("db.exec(") && expr.ends_with(')') && expr.len() >= 9 {
            let args = split_args(&expr[8..expr.len() - 1]);
            if args.len() >= 2 {
                let sql = args[0].trim().trim_matches('"');
                let params = self.parse_params(&args[1]);
                self.execute_sql(sql, &params);
            }
            return Val::Null;
        }

        if expr.starts_with("db.all(") && expr.ends_with(')') && expr.len() >= 8 {
            let args = split_args(&expr[7..expr.len() - 1]);
            if args.len() >= 2 {
                let sql = args[0].trim().trim_matches('"');
                let params = self.parse_params(&args[1]);
                return self.query_all(sql, &params);
            }
            return Val::Null;
        }

        if expr.starts_with('$') {
            let parts: Vec<&str> = expr.split('.').collect();
            let mut val = self.vars.get(parts[0]).cloned().unwrap_or(Val::Null);
            for part in &parts[1..] {
                if let Val::Json(j) = val {
                    val = match j.get(*part) {
                        Some(v) => Val::Json(v.clone()),
                        None => Val::Null,
                    };
                } else { return Val::Null; }
            }
            return val;
        }
        Val::Null
    }

    fn execute_sql(&self, sql: &str, params: &[rusqlite::types::Value]) {
        let conn = self.db.lock().unwrap();
        let _ = conn.execute(sql, rusqlite::params_from_iter(params.iter()));
    }

    fn query_all(&self, sql: &str, params: &[rusqlite::types::Value]) -> Val {
        let conn = self.db.lock().unwrap();
        let mut stmt = match conn.prepare(sql) {
            Ok(s) => s,
            Err(_) => {
                let mut err = serde_json::Map::new();
                err.insert("ok".to_string(), serde_json::Value::Bool(false));
                return Val::Json(serde_json::Value::Object(err));
            }
        };

        let col_names: Vec<String> = stmt.column_names().into_iter().map(String::from).collect();
        let mut results = Vec::new();
        if let Ok(mut rows) = stmt.query(rusqlite::params_from_iter(params.iter())) {
            while let Ok(Some(row)) = rows.next() {
                let mut map = serde_json::Map::new();
                for (i, col) in col_names.iter().enumerate() {
                    let val: rusqlite::types::Value = row.get(i).unwrap_or(rusqlite::types::Value::Null);
                    let jval = match val {
                        rusqlite::types::Value::Integer(i) => serde_json::Value::Number(i.into()),
                        rusqlite::types::Value::Text(s) => serde_json::Value::String(s),
                        _ => serde_json::Value::Null,
                    };
                    map.insert(col.clone(), jval);
                }
                results.push(serde_json::Value::Object(map));
            }
        }
        drop(stmt);
        drop(conn);

        let mut res = serde_json::Map::new();
        res.insert("ok".to_string(), serde_json::Value::Bool(true));
        res.insert("value".to_string(), serde_json::Value::Array(results));
        Val::Json(serde_json::Value::Object(res))
    }

    fn parse_params(&mut self, list: &str) -> Vec<rusqlite::types::Value> {
        let clean = list.trim().trim_start_matches('[').trim_end_matches(']');
        if clean.is_empty() { return vec![]; }
        let mut out = Vec::new();
        for p in split_args(clean) { 
            match self.eval_expr(&p) {
                Val::Int(i) => out.push(rusqlite::types::Value::Integer(i)),
                Val::Bool(b) => out.push(rusqlite::types::Value::Integer(if b { 1 } else { 0 })),
                Val::String(s) => out.push(rusqlite::types::Value::Text(s)),
                Val::Json(j) => {
                    match j {
                        serde_json::Value::Number(n) => {
                            if let Some(i) = n.as_i64() { out.push(rusqlite::types::Value::Integer(i)); }
                            else { out.push(rusqlite::types::Value::Null); }
                        }
                        serde_json::Value::String(s) => out.push(rusqlite::types::Value::Text(s)),
                        serde_json::Value::Bool(b) => out.push(rusqlite::types::Value::Integer(if b { 1 } else { 0 })),
                        serde_json::Value::Null => out.push(rusqlite::types::Value::Null),
                        _ => out.push(rusqlite::types::Value::Text(j.to_string())),
                    }
                },
                Val::Null => out.push(rusqlite::types::Value::Null),
            }
        }
        out
    }
    
    pub fn interpolate(&mut self, text: &str) -> String {
        let mut out = String::new();
        let mut chars = text.char_indices().peekable();
        while let Some(&(_, ch)) = chars.peek() {
            if ch == '$' {
                chars.next();
                if let Some(&(_, next_ch)) = chars.peek() {
                    if next_ch.is_alphabetic() || next_ch == '_' {
                        let mut var_name = String::from("$");
                        while let Some(&(_, c)) = chars.peek() {
                            if c.is_alphanumeric() || c == '_' || c == '.' { var_name.push(c); chars.next(); } 
                            else { break; }
                        }
                        out.push_str(&self.eval_expr(&var_name).to_escaped_string());
                    } else { out.push('$'); }
                } else { out.push('$'); }
            } else { out.push(ch); chars.next(); }
        }
        out
    }
}

fn split_args(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut curr = String::new();
    let mut depth: usize = 0;
    let mut in_q = false;
    let mut prev = ' ';
    for c in input.chars() {
        if c == '"' && prev != '\\' { in_q = !in_q; }
        if !in_q {
            if c == '[' || c == '(' { depth += 1; }
            if c == ']' || c == ')' { depth = depth.saturating_sub(1); }
            if c == ',' && depth == 0 {
                args.push(curr.trim().to_string());
                curr.clear();
                prev = c;
                continue;
            }
        }
        curr.push(c);
        prev = c;
    }
    if !curr.trim().is_empty() { args.push(curr.trim().to_string()); }
    args
}

fn url_decode(input: &str) -> String {
    let mut res = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(std::str::from_utf8(&bytes[i+1..i+3]).unwrap_or(""), 16) {
                res.push(b); i += 3; continue;
            }
        }
        if bytes[i] == b'+' { res.push(b' '); } else { res.push(bytes[i]); }
        i += 1;
    }
    String::from_utf8_lossy(&res).to_string()
}

// ==========================================
// SERVER
// ==========================================
struct AppState { db: Arc<Mutex<Connection>>, public_dir: PathBuf }

#[derive(ClapParser, Debug)]
#[command(name = "sticks", version = "1.0", about = "Put some sticks in your HTML.")]
struct Cli { #[command(subcommand)] command: Command }

#[derive(Subcommand, Debug)]
enum Command { Serve { #[arg(default_value = "sticks.toml")] config: String }, Check { #[arg(default_value = "sticks.toml")] config: String } }

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Check { .. } => { println!("✅ Checked."); std::process::exit(0); }
        Command::Serve { config } => {
            let config_path = PathBuf::from(&config);
            let mut base_dir = config_path.parent().unwrap_or(Path::new("")).to_path_buf();
            if base_dir.as_os_str().is_empty() { base_dir = PathBuf::from("."); }

            let config_content = fs::read_to_string(&config_path).unwrap();
            let cfg: Config = toml::from_str(&config_content).unwrap();
            let site = &cfg.site[0];

            let db_path = base_dir.join(&site.database.path);
            if let Some(p) = db_path.parent() { fs::create_dir_all(p).unwrap(); }
            let conn = Connection::open(&db_path).unwrap();
            
            conn.execute(
                "CREATE TABLE IF NOT EXISTS tasks (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    username TEXT NOT NULL,
                    name TEXT NOT NULL,
                    done INTEGER NOT NULL DEFAULT 0
                );",
                [],
            ).unwrap();

            let state = Arc::new(AppState { db: Arc::new(Mutex::new(conn)), public_dir: base_dir.join(&site.public_dir) });
            
            let addr = format!("{}:{}", cfg.daemon.interface, cfg.daemon.port);
            println!("🚀 Sticks 1.0 Server online: http://{}", addr);
            let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
            axum::serve(listener, Router::new().fallback(any(handle_request)).with_state(state)).await.unwrap();
        }
    }
}

async fn handle_request(State(state): State<Arc<AppState>>, req: Request<Body>) -> Response {
    let path = req.uri().path().to_string();
    let relative = if path == "/" { "index.html" } else { path.trim_start_matches('/') };
    let file_path = state.public_dir.join(relative);
    
    let template = match fs::read_to_string(&file_path) {
        Ok(t) => t, Err(_) => return (StatusCode::NOT_FOUND, "404 - Not Found").into_response(),
    };
    let ast = match parse(scan(&template).unwrap()) {
        Ok(a) => a, Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Parse Error: {}", e)).into_response(),
    };
    let req_method = req.method().to_string();

    // Parse Cookies
    let mut cookies = HashMap::new();
    if let Some(cookie_hdr) = req.headers().get(header::COOKIE).and_then(|v| v.to_str().ok()) {
        for pair in cookie_hdr.split(';') {
            if let Some((k, v)) = pair.trim().split_once('=') {
                cookies.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
    }

    // Parse Query
    let mut req_map = serde_json::Map::new();
    let query_string = req.uri().query().unwrap_or("");
    let mut q_map = serde_json::Map::new();
    for pair in query_string.split('&') {
        if let Some((k, v)) = pair.split_once('=') { q_map.insert(url_decode(k), serde_json::Value::String(url_decode(v))); }
    }
    req_map.insert("query".to_string(), serde_json::Value::Object(q_map));

    // Parse Body
    if req_method == "POST" {
        let bytes = axum::body::to_bytes(req.into_body(), 1024 * 64).await.unwrap();
        let mut form_map = serde_json::Map::new();
        if let Ok(s) = std::str::from_utf8(&bytes) {
            for pair in s.split('&') {
                if let Some((k, v)) = pair.split_once('=') { form_map.insert(url_decode(k), serde_json::Value::String(url_decode(v))); }
            }
        }
        req_map.insert("body".to_string(), serde_json::Value::Object(form_map));
    }

    let mut ctx = Context {
        vars: HashMap::new(),
        cookies,
        set_cookies: Vec::new(),
        db: Arc::clone(&state.db),
        req_method: req_method.clone(),
        redirect: None,
        status: StatusCode::OK,
    };
    ctx.vars.insert("$req".to_string(), Val::Json(serde_json::Value::Object(req_map)));

    println!("[HTTP] {} {}", req_method, path);

    let mut view_start_idx = ast.len();

    // Effect Zone: Statements and Handlers preceding the first HTML output byte
    for (idx, node) in ast.iter().enumerate() {
        match node {
            Node::Statements(stmts) => {
                ctx.execute_statements(stmts);
                if let Some(url) = ctx.redirect.clone() {
                    println!("  └─ Redirecting to: {}", url);
                    let mut resp = Response::builder()
                        .status(StatusCode::SEE_OTHER)
                        .header(header::LOCATION, &url);
                    for cookie in &ctx.set_cookies {
                        resp = resp.header(header::SET_COOKIE, cookie);
                    }
                    return resp.body(Body::empty()).unwrap();
                }
            }
            Node::Handler { method, stmts } => {
                if *method == req_method {
                    ctx.execute_statements(stmts);
                    if let Some(url) = ctx.redirect.clone() {
                        println!("  └─ Redirecting to: {}", url);
                        let mut resp = Response::builder()
                            .status(StatusCode::SEE_OTHER)
                            .header(header::LOCATION, &url);
                        for cookie in &ctx.set_cookies {
                            resp = resp.header(header::SET_COOKIE, cookie);
                        }
                        return resp.body(Body::empty()).unwrap();
                    }
                }
            }
            Node::Text(t) if t.trim().is_empty() => {
                // Ignore whitespace between blocks before the view zone begins
            }
            _ => {
                view_start_idx = idx;
                break;
            }
        }
    }

    let status = ctx.status;
    let set_cookies = ctx.set_cookies.clone();

    // View Zone: Asynchronous streaming of remaining output nodes
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<String, std::io::Error>>(32);
    let view_nodes = ast[view_start_idx..].to_vec();

    tokio::spawn(async move {
        for node in view_nodes {
            match node {
                Node::Text(t) => { let _ = tx.send(Ok(t)).await; }
                Node::Print(expr) => { let _ = tx.send(Ok(ctx.eval_expr(&expr).to_escaped_string())).await; }
                Node::If { cond, body, else_node } => {
                    if matches!(ctx.eval_expr(&cond), Val::Bool(true)) {
                        let out = ctx.interpolate(&body);
                        let _ = tx.send(Ok(out)).await;
                    } else if let Some(el) = else_node {
                        if let Node::Text(t) = *el {
                            let out = ctx.interpolate(&t);
                            let _ = tx.send(Ok(out)).await;
                        }
                    }
                }
                Node::For { var, iter, body, else_body } => {
                    let mut executed = false;
                    if let Val::Json(serde_json::Value::Array(arr)) = ctx.eval_expr(&iter) {
                        for item in arr {
                            executed = true;
                            ctx.vars.insert(var.clone(), Val::Json(item));
                            let out = ctx.interpolate(&body);
                            let _ = tx.send(Ok(out)).await;
                        }
                    }
                    if !executed {
                        if let Some(el) = else_body {
                            let out = ctx.interpolate(&el);
                            let _ = tx.send(Ok(out)).await;
                        }
                    }
                }
                Node::Statements(stmts) => ctx.execute_statements(&stmts),
                Node::Handler { .. } => {}
            }
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    let mut resp = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8");
    for cookie in &set_cookies {
        resp = resp.header(header::SET_COOKIE, cookie);
    }
    resp.body(Body::from_stream(stream)).unwrap()
}