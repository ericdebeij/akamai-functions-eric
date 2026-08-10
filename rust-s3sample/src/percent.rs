//! Percent-decoding for request paths.
//!
//! Incoming request paths arrive percent-encoded exactly as the client sent them, so
//! they have to be decoded before they can be used as an object key. The encoding
//! direction lives in [`crate::sigv4`], because there it is part of computing the
//! signature and has to follow the SigV4 rules precisely.

/// Converts a single hex digit (as an ASCII byte) to its numeric value,
/// or `None` if it is not a hex digit at all.
fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Decodes `%XX` escapes in a request path back into the real object key.
///
/// Note the difference from a query string: there `+` means a space, but in a path
/// `+` is a literal plus. That is why `form_urlencoded` is not used for decoding.
///
/// This walks the input byte by byte instead of slicing the `&str`. Slicing with
/// `&path[i + 1..i + 3]` would panic if those indices fall inside a multi-byte
/// character (as in `%€`), and a panic in the handler means a 500 for the client.
/// Anything that is not a valid escape is passed through unchanged, so a stray `%`
/// is harmless.
pub(crate) fn percent_decode(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        // A `let` chain: this branch is taken only if we are on a '%' *and* the next
        // two bytes both parse as hex digits. `bytes.get(..)` returns an `Option`, so
        // running off the end of the input is handled without an explicit bounds check.
        if bytes[i] == b'%'
            && let (Some(hi), Some(lo)) = (
                bytes.get(i + 1).copied().and_then(hex_val),
                bytes.get(i + 2).copied().and_then(hex_val),
            )
        {
            // Recombine the two half-bytes into one: high nibble shifted up, low added.
            out.push((hi << 4) | lo);
            i += 3; // Skip the '%' and both hex digits.
            continue;
        }

        out.push(bytes[i]);
        i += 1;
    }

    // The decoded bytes should be valid UTF-8, but a malformed key must not crash us,
    // so invalid sequences are replaced instead of returning an error.
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::percent_decode;

    #[test]
    fn decodes_escapes_and_leaves_the_rest_alone() {
        assert_eq!(percent_decode("a%20b.txt"), "a b.txt");
        assert_eq!(percent_decode("sub/dir/file.txt"), "sub/dir/file.txt");
        assert_eq!(percent_decode("e%C3%A9f.txt"), "eéf.txt"); // Multi-byte UTF-8.
        assert_eq!(percent_decode("a%2fb.txt"), "a/b.txt"); // Lowercase hex.
        assert_eq!(percent_decode("c+d.txt"), "c+d.txt"); // '+' is a literal plus.
    }

    #[test]
    fn malformed_escapes_pass_through_without_panicking() {
        assert_eq!(percent_decode("%.txt"), "%.txt");
        assert_eq!(percent_decode("%zz.txt"), "%zz.txt");
        assert_eq!(percent_decode("%"), "%");
        assert_eq!(percent_decode("%4"), "%4");
        assert_eq!(percent_decode("%€.txt"), "%€.txt"); // Would panic if we sliced.
    }
}
