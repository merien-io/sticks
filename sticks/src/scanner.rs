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