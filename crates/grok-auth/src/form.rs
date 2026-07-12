//! application/x-www-form-urlencoded helpers (no extra deps).

/// Encode a single form value (RFC 3986 unreserved set left as-is).
#[must_use]
pub fn urlencoding_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Build a form body from key/value pairs.
#[must_use]
pub fn form_body(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", urlencoding_encode(k), urlencoding_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_space_and_keeps_alnum() {
        assert_eq!(urlencoding_encode("ab c"), "ab%20c");
        assert_eq!(urlencoding_encode("ok_1"), "ok_1");
    }

    #[test]
    fn form_body_joins() {
        assert_eq!(form_body(&[("a", "1"), ("b", "x y")]), "a=1&b=x%20y");
    }
}
