/// Removes items annotated with the exact `#[cfg(test)]` attribute while
/// preserving production source on either side of them.
pub fn without_cfg_test_items(source: &str) -> String {
    let mut production = String::with_capacity(source.len());
    let mut cursor = 0;

    while let Some(marker) = next_cfg_test_marker(source, cursor) {
        production.push_str(&source[cursor..marker]);
        cursor = cfg_test_item_end(source, marker);
    }

    production.push_str(&source[cursor..]);
    production
}

fn next_cfg_test_marker(source: &str, mut index: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let marker = b"#[cfg(test)]";

    while index < bytes.len() {
        if bytes.get(index..index + marker.len()) == Some(marker) {
            return Some(index);
        }
        if let Some(end) = non_code_end(source, index) {
            index = end;
        } else {
            index += 1;
        }
    }

    None
}

fn cfg_test_item_end(source: &str, marker: usize) -> usize {
    let bytes = source.as_bytes();
    let mut index = marker + "#[cfg(test)]".len();
    let mut parentheses = 0usize;
    let mut brackets = 0usize;
    let mut angles = 0usize;

    while index < bytes.len() {
        if let Some(end) = non_code_end(source, index) {
            index = end;
            continue;
        }

        match bytes[index] {
            b'(' => parentheses += 1,
            b')' => parentheses = parentheses.saturating_sub(1),
            b'[' => brackets += 1,
            b']' => brackets = brackets.saturating_sub(1),
            b'<' => angles += 1,
            b'>' => angles = angles.saturating_sub(1),
            b'{' if parentheses == 0 && brackets == 0 && angles == 0 => {
                return braced_item_end(source, index);
            }
            b';' | b',' if parentheses == 0 && brackets == 0 && angles == 0 => {
                return index + 1;
            }
            _ => {}
        }
        index += 1;
    }

    bytes.len()
}

fn braced_item_end(source: &str, opening: usize) -> usize {
    let bytes = source.as_bytes();
    let mut depth = 0usize;
    let mut index = opening;

    while index < bytes.len() {
        if let Some(end) = non_code_end(source, index) {
            index = end;
            continue;
        }

        match bytes[index] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return index + 1;
                }
            }
            _ => {}
        }
        index += 1;
    }

    bytes.len()
}

/// Returns the byte immediately after a comment or literal starting at `index`.
fn non_code_end(source: &str, index: usize) -> Option<usize> {
    let bytes = source.as_bytes();

    if bytes.get(index..index + 2) == Some(b"//") {
        return Some(
            source[index..]
                .find('\n')
                .map_or(bytes.len(), |newline| index + newline + 1),
        );
    }
    if bytes.get(index..index + 2) == Some(b"/*") {
        return Some(block_comment_end(bytes, index));
    }
    if let Some(end) = raw_string_end(source, index) {
        return Some(end);
    }
    if bytes.get(index) == Some(&b'"') {
        return Some(quoted_end(bytes, index, b'"'));
    }
    if bytes.get(index) == Some(&b'\'') {
        return char_literal_end(source, index);
    }

    None
}

fn block_comment_end(bytes: &[u8], mut index: usize) -> usize {
    let mut depth = 0usize;

    while index < bytes.len() {
        if bytes.get(index..index + 2) == Some(b"/*") {
            depth += 1;
            index += 2;
        } else if bytes.get(index..index + 2) == Some(b"*/") {
            depth -= 1;
            index += 2;
            if depth == 0 {
                return index;
            }
        } else {
            index += 1;
        }
    }

    bytes.len()
}

fn raw_string_end(source: &str, index: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    if bytes.get(index) != Some(&b'r') {
        return None;
    }

    let mut opening_quote = index + 1;
    while bytes.get(opening_quote) == Some(&b'#') {
        opening_quote += 1;
    }
    if bytes.get(opening_quote) != Some(&b'"') {
        return None;
    }

    let hashes = opening_quote - index - 1;
    let mut cursor = opening_quote + 1;
    while let Some(relative_quote) = source[cursor..].find('"') {
        let quote = cursor + relative_quote;
        let suffix_start = quote + 1;
        let suffix_end = suffix_start + hashes;
        if suffix_end <= bytes.len()
            && bytes[suffix_start..suffix_end]
                .iter()
                .all(|byte| *byte == b'#')
        {
            return Some(suffix_end);
        }
        cursor = quote + 1;
    }

    Some(bytes.len())
}

fn quoted_end(bytes: &[u8], mut index: usize, quote: u8) -> usize {
    index += 1;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index = (index + 2).min(bytes.len());
        } else if bytes[index] == quote {
            return index + 1;
        } else {
            index += 1;
        }
    }
    bytes.len()
}

fn char_literal_end(source: &str, index: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let content_start = index + 1;
    if bytes.get(content_start) == Some(&b'\\') {
        let end = quoted_end(bytes, index, b'\'');
        return (end < bytes.len() || bytes.last() == Some(&b'\'')).then_some(end);
    }

    let character = source.get(content_start..)?.chars().next()?;
    let closing = content_start + character.len_utf8();
    (bytes.get(closing) == Some(&b'\'')).then_some(closing + 1)
}
