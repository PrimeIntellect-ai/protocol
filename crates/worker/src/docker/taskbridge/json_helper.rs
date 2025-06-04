// Helper function to extract the next complete JSON object from a string
pub fn extract_next_json(input: &[u8]) -> Option<(&str, usize)> {
    // Skip any leading whitespace (including newlines)
    let mut start_pos = 0;
    while start_pos < input.len() && (input[start_pos] <= 32) {
        // ASCII space and below includes all whitespace
        start_pos += 1;
    }

    if start_pos >= input.len() {
        return None; // No content left
    }

    // If we find an opening brace, look for the matching closing brace
    if input[start_pos] == b'{' {
        let mut brace_count = 1;
        let mut pos = start_pos + 1;

        while pos < input.len() && brace_count > 0 {
            match input[pos] {
                b'{' => brace_count += 1,
                b'}' => brace_count -= 1,
                _ => {}
            }
            pos += 1;
        }

        if brace_count == 0 {
            // Found a complete JSON object
            if let Ok(json_str) = std::str::from_utf8(&input[start_pos..pos]) {
                return Some((json_str, pos));
            }
        }
    }

    // Alternatively, look for a newline-terminated JSON object
    if let Some(newline_pos) = input[start_pos..].iter().position(|&c| c == b'\n') {
        let end_pos = start_pos + newline_pos;
        // Check if we have a complete JSON object in this line
        if let Ok(line) = std::str::from_utf8(&input[start_pos..end_pos]) {
            let trimmed = line.trim();
            if trimmed.starts_with('{') && trimmed.ends_with('}') {
                return Some((trimmed, end_pos + 1)); // +1 to consume the newline
            }
        }

        // If not a complete JSON object, skip this line and try the next
        return extract_next_json(&input[end_pos + 1..])
            .map(|(json, len)| (json, len + end_pos + 1));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_input() {
        assert_eq!(extract_next_json(b""), None);
    }

    #[test]
    fn test_whitespace_only() {
        assert_eq!(extract_next_json(b"   \n\t  "), None);
    }

    #[test]
    fn test_simple_json() {
        let input = b"{\"key\": \"value\"}";
        let (json, pos) = extract_next_json(input).unwrap();
        assert_eq!(json, "{\"key\": \"value\"}");
        assert_eq!(pos, input.len());
    }

    #[test]
    fn test_json_with_whitespace() {
        let input = b"  {\"key\": \"value\"}  ";
        let (json, pos) = extract_next_json(input).unwrap();
        assert_eq!(json, "{\"key\": \"value\"}");
        assert_eq!(pos, 18);
    }

    #[test]
    fn test_nested_json() {
        let input = b"{\"outer\": {\"inner\": \"value\"}}";
        let (json, pos) = extract_next_json(input).unwrap();
        assert_eq!(json, "{\"outer\": {\"inner\": \"value\"}}");
        assert_eq!(pos, input.len());
    }

    #[test]
    fn test_multiple_json_objects() {
        let input = b"{\"first\": 1}\n{\"second\": 2}";
        let (json, pos) = extract_next_json(input).unwrap();
        assert_eq!(json, "{\"first\": 1}");
        assert_eq!(pos, 12);
    }

    #[test]
    fn test_incomplete_json() {
        let input = b"{\"key\": \"value\"";
        assert_eq!(extract_next_json(input), None);
    }

    #[test]
    fn test_json_with_newlines() {
        let input = b"{\n  \"key\": \"value\"\n}";
        let (json, pos) = extract_next_json(input).unwrap();
        assert_eq!(json, "{\n  \"key\": \"value\"\n}");
        assert_eq!(pos, input.len());
    }

    #[test]
    fn test_multiple_json_with_whitespace() {
        let input = b"  {\"first\": 1}  \n  {\"second\": 2}  ";
        let (json, pos) = extract_next_json(input).unwrap();
        assert_eq!(json, "{\"first\": 1}");
        assert_eq!(pos, 14);
    }
}
