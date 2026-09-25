//! Deterministic JSON encoding — the pre-image for every signature and hash.
//!
//! Two records that are semantically identical but written differently (key
//! order, whitespace) must produce byte-identical encodings, or signatures and
//! content ids would not be reproducible.
//!
//! # Rules
//!
//! - Object keys are sorted by UTF-8 byte order.
//! - No insignificant whitespace.
//! - Strings are escaped per RFC 8259 (delegated to [`serde_json`]).
//! - Numbers are rendered by [`serde_json::Number`]: integers exactly, floats
//!   as shortest round-trip.
//! - Arrays preserve order.
//!
//! # Deviation from RFC 8785
//!
//! JCS specifies ECMAScript number serialization. [`serde_json`] uses `ryu`
//! shortest round-trip, which agrees with JCS for values that round-trip
//! exactly but can differ for some doubles. This is tracked as a spec task;
//! for v1 all in-tree payloads use integers and short decimals.
//!
//! # Safety
//!
//! Nesting is bounded by [`MAX_DEPTH`] because payloads are untrusted network
//! input and naive recursive encoding would allow stack exhaustion.

use serde::Serialize;
use serde_json::Value;

use crate::error::{Error, Result};

/// Maximum object/array nesting depth accepted during canonicalization.
pub const MAX_DEPTH: usize = 128;

/// Encode `value` to its canonical byte representation.
pub fn to_vec(value: &Value) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(estimate_len(value, 0));
    write_value(value, &mut out, 0)?;
    Ok(out)
}

/// Encode `value` to its canonical UTF-8 string representation.
pub fn to_string(value: &Value) -> Result<String> {
    let bytes = to_vec(value)?;
    // Canonical output is built from JSON string escapes, so it is valid UTF-8.
    String::from_utf8(bytes).map_err(|e| Error::MalformedRecord(e.to_string()))
}

/// Serialize any [`Serialize`] type through JSON and canonicalize it.
///
/// This is the normal entry point for signing a typed record struct.
pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let json = serde_json::to_value(value)?;
    to_vec(&json)
}

fn write_value(value: &Value, out: &mut Vec<u8>, depth: usize) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err(Error::MalformedRecord(format!(
            "nesting deeper than {MAX_DEPTH} levels"
        )));
    }

    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Number(n) => out.extend_from_slice(n.to_string().as_bytes()),
        Value::String(s) => {
            let encoded = serde_json::to_string(s)?;
            out.extend_from_slice(encoded.as_bytes());
        }
        Value::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_value(item, out, depth + 1)?;
            }
            out.push(b']');
        }
        Value::Object(map) => {
            out.push(b'{');
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_unstable_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                let encoded_key = serde_json::to_string(key)?;
                out.extend_from_slice(encoded_key.as_bytes());
                out.push(b':');
                // `key` came from `map`, so the lookup cannot fail.
                write_value(&map[key], out, depth + 1)?;
            }
            out.push(b'}');
        }
    }

    Ok(())
}

/// Capacity hint only. Bounded by [`MAX_DEPTH`] so a pathologically deep value
/// cannot exhaust the stack before [`write_value`] rejects it.
fn estimate_len(value: &Value, depth: usize) -> usize {
    if depth > MAX_DEPTH {
        return 0;
    }
    match value {
        Value::Null | Value::Bool(_) => 4,
        Value::Number(n) => n.to_string().len(),
        Value::String(s) => s.len() + 2,
        Value::Array(items) => {
            items
                .iter()
                .map(|v| estimate_len(v, depth + 1))
                .sum::<usize>()
                + items.len()
                + 2
        }
        Value::Object(map) => {
            map.iter()
                .map(|(k, v)| k.len() + estimate_len(v, depth + 1) + 3)
                .sum::<usize>()
                + 2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_order_does_not_change_encoding() {
        let a = json!({"z": 1, "a": 2, "m": {"y": 3, "b": 4}});
        let b = json!({"a": 2, "m": {"b": 4, "y": 3}, "z": 1});
        assert_eq!(to_vec(&a).unwrap(), to_vec(&b).unwrap());
        assert_eq!(to_string(&a).unwrap(), r#"{"a":2,"m":{"b":4,"y":3},"z":1}"#);
    }

    #[test]
    fn array_order_is_preserved() {
        let a = json!([1, 2, 3]);
        let b = json!([3, 2, 1]);
        assert_ne!(to_vec(&a).unwrap(), to_vec(&b).unwrap());
    }

    #[test]
    fn no_whitespace_is_emitted() {
        let v = json!({"a": [1, 2], "b": null});
        let s = to_string(&v).unwrap();
        assert!(!s.contains(' '));
        assert_eq!(s, r#"{"a":[1,2],"b":null}"#);
    }

    #[test]
    fn strings_are_escaped() {
        let v = json!({"k": "a\"b\nc\\d"});
        assert_eq!(to_string(&v).unwrap(), r#"{"k":"a\"b\nc\\d"}"#);
    }

    #[test]
    fn depth_limit_is_enforced() {
        let mut deep = json!({});
        let mut cursor = &mut deep;
        for _ in 0..(MAX_DEPTH + 10) {
            let next = json!({});
            if let Value::Object(map) = cursor {
                map.insert("n".to_string(), next);
            }
            cursor = match cursor {
                Value::Object(map) => map.get_mut("n").unwrap(),
                _ => unreachable!(),
            };
        }
        assert!(matches!(to_vec(&deep), Err(Error::MalformedRecord(_))));
    }

    #[test]
    fn unicode_keys_sort_by_bytes() {
        let v = json!({"é": 1, "z": 2, "a": 3});
        // "a" < "z" < "é" in UTF-8 byte order.
        assert_eq!(to_string(&v).unwrap(), r#"{"a":3,"z":2,"é":1}"#);
    }
}
