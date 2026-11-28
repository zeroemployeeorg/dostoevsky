//! Standard base64 with padding, as Go's `encoding/base64.StdEncoding`, which `encoding/json` uses for `[]byte`.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode(input: &[u8]) -> String {
    let mut out = Vec::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        out.push(ALPHABET[(b[0] >> 2) as usize]);
        out.push(ALPHABET[(((b[0] & 3) << 4) | (b[1] >> 4)) as usize]);
        out.push(if chunk.len() > 1 {
            ALPHABET[(((b[1] & 15) << 2) | (b[2] >> 6)) as usize]
        } else {
            b'='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(b[2] & 63) as usize]
        } else {
            b'='
        });
    }
    String::from_utf8(out).expect("base64 output is ASCII")
}

fn sextet(c: u8) -> Option<u8> {
    match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Decodes as Go's non-strict `StdEncoding.DecodeString`: carriage returns and line feeds are skipped, padding is
/// required, and unused trailing bits are not checked.
pub fn decode(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    decode_into(input.as_bytes(), &mut out).then_some(out)
}

/// As [`decode`], into a buffer the caller keeps: `out` is cleared first. False means the input is not base64, and
/// then `out` holds nothing to rely on.
pub fn decode_into(input: &[u8], out: &mut Vec<u8>) -> bool {
    out.clear();
    let mut quad = [0u8; 4];
    let (mut have, mut padded) = (0, false);
    for &c in input {
        if c == b'\r' || c == b'\n' {
            continue;
        }
        if padded && have == 0 {
            return false; // something follows the padded group
        }
        quad[have] = c;
        have += 1;
        if have < 4 {
            continue;
        }
        have = 0;
        let pad = quad.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 {
            return false;
        }
        padded = pad > 0;
        let mut v = [0u8; 4];
        for (k, &c) in quad[..4 - pad].iter().enumerate() {
            match sextet(c) {
                Some(x) => v[k] = x,
                None => return false,
            }
        }
        out.push((v[0] << 2) | (v[1] >> 4));
        if pad < 2 {
            out.push((v[1] << 4) | (v[2] >> 2));
        }
        if pad < 1 {
            out.push((v[2] << 6) | v[3]);
        }
    }
    have == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors_round_trip() {
        for (plain, coded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(encode(plain.as_bytes()), coded);
            assert_eq!(decode(coded).unwrap(), plain.as_bytes());
        }
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(decode(&encode(&all)).unwrap(), all);
    }

    #[test]
    fn refuses_what_go_refuses_and_skips_newlines() {
        assert_eq!(decode("Zm9v\nYmFy\r\n").unwrap(), b"foobar");
        for bad in ["Zg", "Zg=", "Z===", "Zm9v=", "Zg==Zg==", "Zm9*"] {
            assert!(decode(bad).is_none(), "{bad}");
        }
    }
}
