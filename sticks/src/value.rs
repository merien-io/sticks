use serde_json::{Number, Value};

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