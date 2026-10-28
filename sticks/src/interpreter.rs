use crate::apiaccess;
use crate::dbaccess;
use rusqlite::Connection;
use serde_json::{Map, Number, Value};
use std::ops::ControlFlow;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone, PartialEq)]
pub enum RawToken {
    Text(String),
    Block(String),
}

pub fn scan(input: &str) -> Result<Vec<RawToken>, String> {
    let mut tokens = Vec::new();
    let mut chars = input.char_indices().peekable();
    let bytes = input.as_bytes();
    let mut text_buf = String::new();

    while let Some(&(i, ch)) = chars.peek() {
        if ch == '|' && i + 1 < bytes.len() && bytes[i + 1] == b'|' {
            if !text_buf.is_empty() {
                tokens.push(RawToken::Text(std::mem::take(&mut text_buf)));
            }
            chars.next();
            chars.next();

            let mut block_buf = String::new();
            let mut closed = false;
            while let Some(&(_, c)) = chars.peek() {
                if c == '|' {
                    chars.next();
                    closed = true;
                    break;
                } else {
                    block_buf.push(c);
                    chars.next();
                }
            }
            if !closed {
                return Err(format!("Unterminated block starting at byte index {i}"));
            }
            tokens.push(RawToken::Block(block_buf));
        } else {
            text_buf.push(ch);
            chars.next();
        }
    }
    if !text_buf.is_empty() {
        tokens.push(RawToken::Text(text_buf));
    }
    Ok(tokens)
}

#[derive(Debug, Clone)]
pub enum Node {
    Text(String),
    Print(String),
    If {
        cond: String,
        body: String,
        else_body: Option<String>,
    },
    For {
        var: String,
        iter: String,
        body: String,
        else_body: Option<String>,
    },
    Handler {
        method: String,
        stmts: String,
    },
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
                    let head = trimmed[3..head_end].trim();
                    let body = &trimmed[head_end + 1..];

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
                    ast.push(Node::If {
                        cond: head.to_string(),
                        body: body.to_string(),
                        else_body,
                    });
                } else if trimmed.starts_with("for ") {
                    let head_end = trimmed.find(':').ok_or("Expected ':' in 'for' block")?;
                    let head = trimmed[4..head_end].trim();
                    let body = &trimmed[head_end + 1..];

                    let parts: Vec<&str> = head.split_whitespace().collect();
                    let (var, iter) = if parts.len() == 3 && parts[1] == "in" {
                        (parts[0], parts[2])
                    } else if parts.len() >= 4 && parts[2] == "in" {
                        (parts[1], parts[3])
                    } else {
                        return Err(format!("Invalid for loop signature: '{head}'"));
                    };

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
                    ast.push(Node::For {
                        var: var.to_string(),
                        iter: iter.to_string(),
                        body: body.to_string(),
                        else_body,
                    });
                } else if trimmed.starts_with("else:") {
                    return Err("Orphan 'else:' block encountered".to_string());
                } else if trimmed.starts_with("on ") {
                    let head_end = trimmed.find(':').ok_or("Expected ':' in 'on' block")?;
                    let method = trimmed[3..head_end].trim();
                    let body_str = &trimmed[head_end + 1..];
                    ast.push(Node::Handler {
                        method: method.to_string(),
                        stmts: body_str.to_string(),
                    });
                } else if trimmed.contains(';')
                    || trimmed.starts_with("type ")
                    || trimmed.starts_with("input ")
                {
                    ast.push(Node::Statements(trimmed.to_string()));
                } else {
                    ast.push(Node::Print(trimmed.to_string()));
                }
            }
        }
        i += 1;
    }
    Ok(ast)
}

#[derive(Clone, Debug)]
pub enum Val {
    Null,
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Json(Value),
}

impl PartialEq for Val {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Val::Null, Val::Null) => true,
            (Val::Null, Val::Json(Value::Null)) | (Val::Json(Value::Null), Val::Null) => true,
            (Val::Bool(a), Val::Bool(b)) => a == b,
            (Val::Bool(a), Val::Json(Value::Bool(b))) | (Val::Json(Value::Bool(b)), Val::Bool(a)) => {
                a == b
            }
            (Val::String(a), Val::String(b)) => a == b,
            (Val::String(a), Val::Json(Value::String(b)))
            | (Val::Json(Value::String(b)), Val::String(a)) => a == b,
            (Val::Int(a), Val::Int(b)) => a == b,
            (Val::Int(a), Val::Json(Value::Number(b)))
            | (Val::Json(Value::Number(b)), Val::Int(a)) => {
                b.as_i64().map_or(false, |val| val == *a)
            }
            (Val::Float(a), Val::Float(b)) => (a - b).abs() < f64::EPSILON,
            (Val::Int(a), Val::Float(b)) | (Val::Float(b), Val::Int(a)) => (*a as f64 - b).abs() < f64::EPSILON,
            (Val::Json(a), Val::Json(b)) => a == b,
            _ => false,
        }
    }
}

impl PartialOrd for Val {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        let n1 = self.to_f64();
        let n2 = other.to_f64();
        if let (Some(a), Some(b)) = (n1, n2) {
            return a.partial_cmp(&b);
        }
        None
    }
}

impl Val {
    pub fn to_f64(&self) -> Option<f64> {
        match self {
            Val::Int(i) => Some(*i as f64),
            Val::Float(f) => Some(*f),
            Val::Json(Value::Number(n)) => n.as_f64(),
            _ => None,
        }
    }

    pub fn is_truthy(&self) -> bool {
        match self {
            Val::Bool(b) => *b,
            Val::Json(Value::Bool(b)) => *b,
            Val::Int(i) => *i != 0,
            Val::Json(Value::Number(n)) => n.as_i64().map_or(false, |i| i != 0),
            Val::String(s) => !s.is_empty(),
            Val::Json(Value::String(s)) => !s.is_empty(),
            Val::Json(Value::Array(a)) => !a.is_empty(),
            Val::Json(Value::Object(o)) => !o.is_empty(),
            Val::Null | Val::Json(Value::Null) | Val::Float(_) => false,
        }
    }

    pub fn to_plain_string(&self) -> String {
        match self {
            Val::Null => String::new(),
            Val::String(s) => s.clone(),
            Val::Int(i) => i.to_string(),
            Val::Float(f) => f.to_string(),
            Val::Bool(b) => b.to_string(),
            Val::Json(j) => match j {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                Value::Null => String::new(),
                _ => j.to_string(),
            },
        }
    }

    pub fn to_escaped_html(&self) -> String {
        escape_html(&self.to_plain_string())
    }

    pub fn to_json_value(&self) -> Value {
        match self {
            Val::Null => Value::Null,
            Val::String(s) => Value::String(s.clone()),
            Val::Int(i) => Value::Number((*i).into()),
            Val::Float(f) => Number::from_f64(*f).map(Value::Number).unwrap_or(Value::Null),
            Val::Bool(b) => Value::Bool(*b),
            Val::Json(j) => j.clone(),
        }
    }
}

pub fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

pub struct Context {
    pub vars: HashMap<String, Val>,
    pub cookies: HashMap<String, String>,
    pub set_cookies: Vec<String>,
    pub db: Arc<Mutex<Connection>>,
    pub redirect: Option<String>,
    pub allow_effects: bool,
}

impl Context {
    pub fn execute_statements(&mut self, script: &str) -> ControlFlow<()> {
        let mut rest = script.trim();
        while !rest.is_empty() {
            if self.redirect.is_some() {
                return ControlFlow::Break(());
            }

            rest = rest.trim();
            if rest.is_empty() {
                break;
            }

            if rest.starts_with("//") {
                if let Some(newline_pos) = rest.find('\n') {
                    rest = &rest[newline_pos + 1..];
                    continue;
                } else {
                    break;
                }
            }

            if rest.starts_with("type ") || rest.starts_with("input ") {
                if let Some(brace_open) = rest.find('{') {
                    if let Some(brace_close) = rest[brace_open..].find('}') {
                        rest = rest[brace_open + brace_close + 1..]
                            .trim_start_matches(';')
                            .trim();
                        continue;
                    }
                }
                if let Some(semi) = rest.find(';') {
                    rest = &rest[semi + 1..];
                    continue;
                }
                break;
            }

            if rest.starts_with("if ") {
                if let Some(brace_open) = rest.find('{') {
                    let cond_str = rest[3..brace_open].trim();
                    let mut depth = 0;
                    let mut brace_close = None;
                    for (idx, ch) in rest[brace_open..].char_indices() {
                        if ch == '{' {
                            depth += 1;
                        } else if ch == '}' {
                            depth -= 1;
                            if depth == 0 {
                                brace_close = Some(brace_open + idx);
                                break;
                            }
                        }
                    }

                    if let Some(close_idx) = brace_close {
                        let body_str = &rest[brace_open + 1..close_idx];
                        let cond_val = self.eval_expr(cond_str);
                        let is_true = cond_val.is_truthy();

                        rest = rest[close_idx + 1..].trim();
                        if is_true {
                            if self.execute_statements(body_str).is_break() {
                                return ControlFlow::Break(());
                            }
                            if rest.starts_with("else") {
                                if let Some(el_open) = rest.find('{') {
                                    let mut d = 0;
                                    for (i, c) in rest[el_open..].char_indices() {
                                        if c == '{' {
                                            d += 1;
                                        } else if c == '}' {
                                            d -= 1;
                                            if d == 0 {
                                                rest = rest[el_open + i + 1..].trim();
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        } else if rest.starts_with("else") {
                            let after_else = rest[4..].trim();
                            if let Some(el_open) = after_else.find('{') {
                                let mut d = 0;
                                let mut el_close = None;
                                for (i, c) in after_else[el_open..].char_indices() {
                                    if c == '{' {
                                        d += 1;
                                    } else if c == '}' {
                                        d -= 1;
                                        if d == 0 {
                                            el_close = Some(el_open + i);
                                            break;
                                        }
                                    }
                                }
                                if let Some(ci) = el_close {
                                    let el_body = &after_else[el_open + 1..ci];
                                    rest = after_else[ci + 1..].trim();
                                    if self.execute_statements(el_body).is_break() {
                                        return ControlFlow::Break(());
                                    }
                                }
                            }
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
            if s.is_empty() {
                continue;
            }

            if let Some((lhs, rhs)) = s.split_once('=') {
                let lhs = lhs.trim();
                let var_name = lhs.split_whitespace().last().unwrap_or(lhs);
                if var_name.starts_with('$') {
                    let val = self.eval_expr(rhs);
                    self.vars.insert(var_name.to_string(), val);
                    continue;
                }
            }
            self.eval_expr(s);
        }
        ControlFlow::Continue(())
    }

    pub fn eval_expr(&mut self, expr: &str) -> Val {
        let expr = expr.trim();
        if expr == "null" {
            return Val::Null;
        }
        if expr == "true" {
            return Val::Bool(true);
        }
        if expr == "false" {
            return Val::Bool(false);
        }
        if expr == "[]" {
            return Val::Json(Value::Array(vec![]));
        }
        if let Ok(i) = expr.parse::<i64>() {
            return Val::Int(i);
        }
        if let Ok(f) = expr.parse::<f64>() {
            return Val::Float(f);
        }

        if expr.len() >= 2 && expr.starts_with('"') && expr.ends_with('"') {
            return Val::String(expr[1..expr.len() - 1].to_string());
        }

        // len(x)
        if expr.starts_with("len(") && expr.ends_with(')') {
            let inner = &expr[4..expr.len() - 1].trim();
            let v = self.eval_expr(inner);
            let length = match v {
                Val::Json(Value::Array(a)) => a.len(),
                Val::Json(Value::Object(o)) => o.len(),
                Val::Json(Value::String(s)) => s.len(),
                Val::String(s) => s.len(),
                _ => 0,
            };
            return Val::Int(length as i64);
        }

        if let Some((lhs, rhs)) = expr.split_once(" as ") {
            let val = self.eval_expr(lhs);
            let target_type = rhs.trim();
            return match target_type {
                "String" => Val::String(val.to_plain_string()),
                "Int" => match val {
                    Val::Int(i) => Val::Int(i),
                    Val::Float(f) => Val::Int(f as i64),
                    Val::Json(Value::Number(n)) => Val::Int(n.as_i64().unwrap_or(0)),
                    Val::String(s) => Val::Int(s.parse().unwrap_or(0)),
                    _ => Val::Int(0),
                },
                "Bool" => Val::Bool(val.is_truthy()),
                "Json" => match val {
                    Val::Json(Value::String(s)) => {
                        serde_json::from_str::<Value>(&s).map(Val::Json).unwrap_or(Val::Json(Value::String(s)))
                    }
                    Val::Json(j) => Val::Json(j),
                    Val::String(s) => {
                        serde_json::from_str::<Value>(&s).map(Val::Json).unwrap_or(Val::Null)
                    }
                    _ => Val::Null,
                },
                _ => val,
            };
        }

        if expr.starts_with("http.get(") && expr.ends_with(')') {
            let inner = &expr[9..expr.len() - 1];
            let args = split_args(inner);
            if !args.is_empty() {
                let url = self.eval_expr(&args[0]).to_plain_string();
                let mut header_list = Vec::new();

                if args.len() > 1 {
                    if let Val::Json(Value::Object(opts)) = self.eval_expr(&args[1]) {
                        if let Some(Value::Object(hmap)) = opts.get("headers") {
                            for (k, v) in hmap {
                                if let Some(s) = v.as_str() {
                                    header_list.push((k.clone(), s.to_string()));
                                }
                            }
                        }
                    }
                }

                let borrowed_headers: Vec<(&str, &str)> = header_list
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect();

                match apiaccess::http_get(&url, &borrowed_headers) {
                    Ok(json_val) => {
                        let mut map = Map::new();
                        map.insert("ok".to_string(), Value::Bool(true));
                        map.insert("value".to_string(), json_val);
                        map.insert("error".to_string(), Value::Null);
                        map.insert("status".to_string(), Value::Null);
                        return Val::Json(Value::Object(map));
                    }
                    Err(e) => {
                        let mut map = Map::new();
                        map.insert("ok".to_string(), Value::Bool(false));
                        map.insert("value".to_string(), Value::Null);
                        map.insert("status".to_string(), Value::Null);
                        
                        let mut err = Map::new();
                        err.insert("code".to_string(), Value::String("http_error".to_string()));
                        err.insert("message".to_string(), Value::String(e));
                        
                        map.insert("error".to_string(), Value::Object(err));
                        return Val::Json(Value::Object(map));
                    }
                }
            }
        }

        if expr.starts_with("session.set(") && expr.ends_with(')') {
            if !self.allow_effects {
                return Val::Null;
            }
            let inner = &expr[12..expr.len() - 1];
            let args = split_args(inner);
            if args.len() >= 2 {
                let key = self.eval_expr(&args[0]).to_plain_string();
                let val = self.eval_expr(&args[1]).to_plain_string();
                if !key.contains(';') && !val.contains(';') {
                    self.cookies.insert(key.clone(), val.clone());
                    self.set_cookies
                        .push(format!("{key}={val}; Path=/; HttpOnly; SameSite=Lax"));
                }
            }
            return Val::Null;
        }

        if expr.starts_with("session.clear(") && expr.ends_with(')') {
            if !self.allow_effects {
                return Val::Null;
            }
            for (k, _) in self.cookies.drain() {
                self.set_cookies
                    .push(format!("{k}=; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT; HttpOnly; SameSite=Lax"));
            }
            return Val::Null;
        }

        if expr.starts_with("session.get(") && expr.ends_with(')') {
            let inner = &expr[12..expr.len() - 1];
            let key = self.eval_expr(inner).to_plain_string();
            if let Some(val) = self.cookies.get(&key) {
                return Val::String(val.clone());
            }
            return Val::Null;
        }

        if let Some(tail) = expr.strip_prefix("not ") {
            let v = self.eval_expr(tail);
            return Val::Bool(!v.is_truthy());
        }

        // Relational comparisons
        if let Some((left, right)) = expr.split_once(" == ") {
            return Val::Bool(self.eval_expr(left) == self.eval_expr(right));
        }
        if let Some((left, right)) = expr.split_once(" != ") {
            return Val::Bool(self.eval_expr(left) != self.eval_expr(right));
        }
        if let Some((left, right)) = expr.split_once(" >= ") {
            return Val::Bool(self.eval_expr(left) >= self.eval_expr(right));
        }
        if let Some((left, right)) = expr.split_once(" <= ") {
            return Val::Bool(self.eval_expr(left) <= self.eval_expr(right));
        }
        if let Some((left, right)) = expr.split_once(" > ") {
            return Val::Bool(self.eval_expr(left) > self.eval_expr(right));
        }
        if let Some((left, right)) = expr.split_once(" < ") {
            return Val::Bool(self.eval_expr(left) < self.eval_expr(right));
        }

        if expr.starts_with("redirect(") && expr.ends_with(')') {
            if !self.allow_effects {
                return Val::Null;
            }
            let inner = expr[9..expr.len() - 1].trim();
            let target = self.eval_expr(inner).to_plain_string();
            let sanitized = if target.starts_with('/') && !target.starts_with("//") {
                target
            } else {
                "/".to_string()
            };
            self.redirect = Some(sanitized);
            return Val::Null;
        }

        if expr.starts_with("db.exec(") && expr.ends_with(')') {
            if !self.allow_effects {
                return Val::Null;
            }
            let args = split_args(&expr[8..expr.len() - 1]);
            if !args.is_empty() {
                let sql = self.eval_expr(&args[0]).to_plain_string();
                let params = if args.len() > 1 {
                    self.parse_params(&args[1])
                } else {
                    vec![]
                };
                let _ = dbaccess::execute_sql(&self.db, &sql, &params);
            }
            return Val::Null;
        }

        if expr.starts_with("db.all(") && expr.ends_with(')') {
            let args = split_args(&expr[7..expr.len() - 1]);
            if !args.is_empty() {
                let sql = self.eval_expr(&args[0]).to_plain_string();
                let params = if args.len() > 1 {
                    self.parse_params(&args[1])
                } else {
                    vec![]
                };
                return Val::Json(dbaccess::query_all(&self.db, &sql, &params));
            }
            return Val::Null;
        }

        if expr.starts_with('{') && expr.ends_with('}') {
            let body = expr[1..expr.len() - 1].trim();
            let mut map = Map::new();
            if !body.is_empty() {
                for pair in split_args(body) {
                    if let Some((k, v)) = pair.split_once(':') {
                        let k = k.trim().trim_matches('"');
                        let val = self.eval_expr(v);
                        map.insert(k.to_string(), val.to_json_value());
                    }
                }
            }
            return Val::Json(Value::Object(map));
        }

        if expr.starts_with('$') {
            let clean = expr.trim_start_matches('$');
            let parts: Vec<&str> = clean.split('.').collect();
            let mut root_part = parts[0];
            let mut root_idx = None;

            if let Some(start) = root_part.find('[') {
                if let Some(end) = root_part.find(']') {
                    root_idx = root_part[start + 1..end].parse::<usize>().ok();
                    root_part = &root_part[..start];
                }
            }

            let root_name = format!("${root_part}");
            let mut current = self.vars.get(&root_name).cloned().unwrap_or(Val::Null);

            if let Some(idx) = root_idx {
                current = match current {
                    Val::Json(Value::Array(ref arr)) => {
                        arr.get(idx).map(|v| Val::Json(v.clone())).unwrap_or(Val::Null)
                    }
                    _ => Val::Null,
                };
            }

            for part in &parts[1..] {
                let mut p = *part;
                let mut array_idx = None;
                if let Some(start) = p.find('[') {
                    if let Some(end) = p.find(']') {
                        array_idx = p[start + 1..end].parse::<usize>().ok();
                        p = &p[..start];
                    }
                }

                current = match current {
                    Val::Json(Value::Object(map)) => match map.get(p) {
                        Some(v) => Val::Json(v.clone()),
                        None => Val::Null,
                    },
                    _ => Val::Null,
                };

                if let Some(idx) = array_idx {
                    current = match current {
                        Val::Json(Value::Array(arr)) => {
                            arr.get(idx).map(|v| Val::Json(v.clone())).unwrap_or(Val::Null)
                        }
                        _ => Val::Null,
                    };
                }
            }
            return current;
        }

        Val::Null
    }

    fn parse_params(&mut self, list: &str) -> Vec<rusqlite::types::Value> {
        let clean = list.trim().trim_start_matches('[').trim_end_matches(']');
        if clean.is_empty() {
            return vec![];
        }
        let mut out = Vec::new();
        for p in split_args(clean) {
            match self.eval_expr(&p) {
                Val::Int(i) => out.push(rusqlite::types::Value::Integer(i)),
                Val::Float(f) => out.push(rusqlite::types::Value::Real(f)),
                Val::Bool(b) => out.push(rusqlite::types::Value::Integer(if b { 1 } else { 0 })),
                Val::String(s) => out.push(rusqlite::types::Value::Text(s)),
                Val::Json(j) => match j {
                    Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            out.push(rusqlite::types::Value::Integer(i));
                        } else if let Some(f) = n.as_f64() {
                            out.push(rusqlite::types::Value::Real(f));
                        } else {
                            out.push(rusqlite::types::Value::Null);
                        }
                    }
                    Value::String(s) => out.push(rusqlite::types::Value::Text(s)),
                    Value::Bool(b) => {
                        out.push(rusqlite::types::Value::Integer(if b { 1 } else { 0 }))
                    }
                    Value::Null => out.push(rusqlite::types::Value::Null),
                    _ => out.push(rusqlite::types::Value::Text(j.to_string())),
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
                    if next_ch == '$' {
                        chars.next();
                        out.push('$');
                    } else if next_ch == '{' {
                        chars.next();
                        let mut expr = String::new();
                        while let Some(&(_, c)) = chars.peek() {
                            if c == '}' {
                                chars.next();
                                break;
                            }
                            expr.push(c);
                            chars.next();
                        }
                        if let Some(tail) = expr.strip_prefix("url ") {
                            let val = self.eval_expr(tail.trim()).to_plain_string();
                            out.push_str(&percent_encoding::utf8_percent_encode(
                                &val,
                                percent_encoding::NON_ALPHANUMERIC,
                            ).to_string());
                        } else {
                            let resolved = self.eval_expr(expr.trim()).to_escaped_html();
                            out.push_str(&resolved);
                        }
                    } else if next_ch.is_alphabetic() || next_ch == '_' {
                        let mut var_expr = String::from("$");

                        while let Some(&(_, c)) = chars.peek() {
                            if c.is_alphanumeric() || c == '_' {
                                var_expr.push(c);
                                chars.next();
                            } else if c == '.' {
                                let mut lookahead = chars.clone();
                                lookahead.next();
                                if let Some(&(_, ahead_c)) = lookahead.peek() {
                                    if ahead_c.is_alphabetic() || ahead_c == '_' {
                                        var_expr.push('.');
                                        chars.next(); // consume '.'
                                        continue;
                                    }
                                }
                                break;
                            } else if c == '[' {
                                let mut bracket_expr = String::new();
                                let mut lookahead = chars.clone();
                                lookahead.next(); // skip '['
                                let mut closed = false;
                                while let Some(&(_, bc)) = lookahead.peek() {
                                    if bc == ']' {
                                        bracket_expr.push(']');
                                        lookahead.next();
                                        closed = true;
                                        break;
                                    } else if bc.is_ascii_digit() {
                                        bracket_expr.push(bc);
                                        lookahead.next();
                                    } else {
                                        break;
                                    }
                                }
                                if closed {
                                    var_expr.push('[');
                                    var_expr.push_str(&bracket_expr);
                                    chars = lookahead;
                                } else {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                        out.push_str(&self.eval_expr(&var_expr).to_escaped_html());
                    } else {
                        out.push('$');
                    }
                } else {
                    out.push('$');
                }
            } else {
                out.push(ch);
                chars.next();
            }
        }
        out
    }
}  

pub fn split_args(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut curr = String::new();
    let mut depth: usize = 0;
    let mut in_q = false;
    let mut prev = ' ';
    for c in input.chars() {
        if c == '"' && prev != '\\' {
            in_q = !in_q;
        }
        if !in_q {
            if c == '[' || c == '(' || c == '{' {
                depth += 1;
            }
            if c == ']' || c == ')' || c == '}' {
                depth = depth.saturating_sub(1);
            }
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
    if !curr.trim().is_empty() {
        args.push(curr.trim().to_string());
    }
    args
}