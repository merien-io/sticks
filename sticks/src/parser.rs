use crate::scanner::RawToken;

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