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

pub fn split_once_outside_quotes<'a>(input: &'a str, delimiter: &str) -> Option<(&'a str, &'a str)> {
    let mut in_quotes = false;
    let mut depth: usize = 0; // Fix applied here
    let mut prev = ' ';
    
    let delim_bytes = delimiter.as_bytes();
    if delim_bytes.is_empty() {
        return None;
    }

    let bytes = input.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let c = bytes[i] as char;
        if c == '"' && prev != '\\' {
            in_quotes = !in_quotes;
        }
        if !in_quotes {
            if c == '(' || c == '[' || c == '{' {
                depth += 1;
            } else if c == ')' || c == ']' || c == '}' {
                depth = depth.saturating_sub(1);
            } else if depth == 0 {
                if i + delim_bytes.len() <= bytes.len() {
                    if &bytes[i..i + delim_bytes.len()] == delim_bytes {
                        let left = &input[..i];
                        let right = &input[i + delim_bytes.len()..];
                        return Some((left, right));
                    }
                }
            }
        }
        prev = c;
        i += 1;
    }
    None
}