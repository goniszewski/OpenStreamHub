//! Bounded JSON for the untrusted profile entry.
//!
//! The official JSON is mostly image bytes stored as decimal arrays
//! (`{"type":"Buffer","data":[…]}`). Numbers inside arrays are counted and
//! dropped instead of stored, and over-long strings (base64 images) are
//! replaced by `Skipped`, so those payloads are never materialised. Depth,
//! key length, array length and the total node count are capped; any hit
//! rejects the file. The mapper never needs a number inside an array.

use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use std::{cell::Cell, fmt};

pub const MAX_DEPTH: usize = 64;
pub const MAX_STRING_BYTES: usize = 64 * 1024;
pub const MAX_ARRAY_ITEMS: usize = 100_000;
pub const MAX_KEY_BYTES: usize = 1024;
pub const MAX_NODES: usize = 500_000;

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    /// `numbers` counts the numeric elements that were dropped.
    Array {
        items: Vec<Node>,
        numbers: usize,
    },
    /// Key order is kept; a repeated key resolves to its last value, as in
    /// `JSON.parse`.
    Object(Vec<(String, Node)>),
    /// A string longer than `MAX_STRING_BYTES`, kept only as a marker.
    Skipped,
}

static NULL: Node = Node::Null;

impl Node {
    pub fn get(&self, key: &str) -> &Node {
        match self {
            Self::Object(entries) => entries
                .iter()
                .rev()
                .find(|(name, _)| name == key)
                .map_or(&NULL, |(_, value)| value),
            _ => &NULL,
        }
    }

    pub fn str(&self) -> Option<&str> {
        match self {
            Self::String(text) => Some(text),
            _ => None,
        }
    }

    /// A non-negative integer from a number or a decimal string.
    pub fn index(&self) -> Option<u64> {
        match self {
            Self::Number(value) if value.fract() == 0.0 && *value >= 0.0 && *value < 1e15 => {
                Some(*value as u64)
            }
            Self::String(text) if !text.is_empty() && text.len() < 16 => text.parse().ok(),
            _ => None,
        }
    }

    pub fn items(&self) -> &[Node] {
        match self {
            Self::Array { items, .. } => items,
            _ => &[],
        }
    }

    /// Numeric elements dropped from an array (0 for anything else).
    pub fn numbers(&self) -> usize {
        match self {
            Self::Array { numbers, .. } => *numbers,
            _ => 0,
        }
    }

    pub fn entries(&self) -> &[(String, Node)] {
        match self {
            Self::Object(entries) => entries,
            _ => &[],
        }
    }

    /// True for anything a user would see as "set": not null, empty or zero.
    pub fn is_present(&self) -> bool {
        match self {
            Self::Null => false,
            Self::Bool(value) => *value,
            Self::Number(value) => *value != 0.0,
            Self::String(text) => !text.trim().is_empty(),
            Self::Array { items, numbers } => !items.is_empty() || *numbers > 0,
            Self::Object(entries) => entries.iter().any(|(_, value)| value.is_present()),
            Self::Skipped => true,
        }
    }
}

struct Budget {
    nodes: Cell<usize>,
}

impl Budget {
    fn take<E: de::Error>(&self) -> Result<(), E> {
        let nodes = self.nodes.get() + 1;
        if nodes > MAX_NODES {
            return Err(E::custom("the profile JSON has too many values"));
        }
        self.nodes.set(nodes);
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Seed<'a> {
    depth: usize,
    budget: &'a Budget,
}

impl<'de> DeserializeSeed<'de> for Seed<'_> {
    type Value = Node;
    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<Node, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl Seed<'_> {
    fn child<E: de::Error>(self) -> Result<Self, E> {
        if self.depth + 1 > MAX_DEPTH {
            return Err(E::custom("the profile JSON is nested too deeply"));
        }
        Ok(Self {
            depth: self.depth + 1,
            budget: self.budget,
        })
    }

    fn text<E: de::Error>(self, value: &str) -> Result<Node, E> {
        self.budget.take()?;
        Ok(if value.len() > MAX_STRING_BYTES {
            Node::Skipped
        } else {
            Node::String(value.to_owned())
        })
    }
}

impl<'de> Visitor<'de> for Seed<'_> {
    type Value = Node;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Node, E> {
        self.budget.take()?;
        Ok(Node::Bool(value))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Node, E> {
        Ok(Node::Number(value as f64))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Node, E> {
        Ok(Node::Number(value as f64))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Node, E> {
        Ok(Node::Number(value))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Node, E> {
        self.text(value)
    }

    fn visit_unit<E: de::Error>(self) -> Result<Node, E> {
        self.budget.take()?;
        Ok(Node::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Node, A::Error> {
        let child = self.child()?;
        self.budget.take()?;
        let mut items = Vec::new();
        let mut numbers = 0usize;
        while let Some(item) = seq.next_element_seed(child)? {
            match item {
                Node::Number(_) => numbers += 1,
                item => {
                    if items.len() == MAX_ARRAY_ITEMS {
                        return Err(de::Error::custom("a profile JSON array is too long"));
                    }
                    items.push(item);
                }
            }
        }
        Ok(Node::Array { items, numbers })
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Node, A::Error> {
        let child = self.child()?;
        self.budget.take()?;
        let mut entries = Vec::new();
        while let Some(key) = map.next_key_seed(Key)? {
            let value = map.next_value_seed(child)?;
            if let Node::Number(_) = value {
                // Numbers in objects are kept (they carry options and ids),
                // but still count.
                self.budget.take()?;
            }
            entries.push((key, value));
        }
        Ok(Node::Object(entries))
    }
}

struct Key;

impl<'de> DeserializeSeed<'de> for Key {
    type Value = String;
    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<String, D::Error> {
        deserializer.deserialize_str(self)
    }
}

impl Visitor<'_> for Key {
    type Value = String;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("an object key")
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<String, E> {
        if value.len() > MAX_KEY_BYTES {
            return Err(E::custom("a profile JSON key is too long"));
        }
        Ok(value.to_owned())
    }
}

/// Parses `bytes` within the limits above. Trailing data is rejected.
pub fn parse(bytes: &[u8]) -> Result<Node, String> {
    let budget = Budget {
        nodes: Cell::new(0),
    };
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let node = Seed {
        depth: 0,
        budget: &budget,
    }
    .deserialize(&mut deserializer)
    .and_then(|node| deserializer.end().map(|()| node))
    .map_err(|error| format!("the profile JSON is invalid: {error}"))?;
    Ok(node)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffers_and_long_strings_are_dropped_and_depth_is_capped() {
        let long = "A".repeat(MAX_STRING_BYTES + 1);
        let json = format!(
            r#"{{"img":{{"type":"Buffer","data":[1,2,3,4]}},"icon":"{long}","key":["Control","c"],"n":4,"n":5}}"#
        );
        let node = parse(json.as_bytes()).unwrap();
        assert_eq!(
            node.get("img").get("data"),
            &Node::Array {
                items: vec![],
                numbers: 4
            }
        );
        assert_eq!(node.get("icon"), &Node::Skipped);
        assert_eq!(node.get("key").items().len(), 2);
        assert_eq!(node.get("n").index(), Some(5));
        assert_eq!(node.get("missing"), &Node::Null);

        let deep = "[".repeat(MAX_DEPTH) + &"]".repeat(MAX_DEPTH);
        assert!(parse(deep.as_bytes()).is_ok());
        let deeper = "[".repeat(MAX_DEPTH + 1) + &"]".repeat(MAX_DEPTH + 1);
        assert!(
            parse(deeper.as_bytes())
                .unwrap_err()
                .contains("nested too deeply")
        );
        let objects = "{\"a\":".repeat(MAX_DEPTH + 1) + "1" + &"}".repeat(MAX_DEPTH + 1);
        assert!(
            parse(objects.as_bytes())
                .unwrap_err()
                .contains("nested too deeply")
        );
    }

    #[test]
    fn invalid_trailing_and_oversized_input_is_rejected() {
        for bad in [&b"{"[..], b"{} {}", b"nul", b"{\"a\":}", b"\xff"] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
        let key = format!("{{\"{}\":1}}", "k".repeat(MAX_KEY_BYTES + 1));
        assert!(
            parse(key.as_bytes())
                .unwrap_err()
                .contains("key is too long")
        );
        let many = format!("[{}null]", "null,".repeat(MAX_ARRAY_ITEMS));
        assert!(parse(many.as_bytes()).unwrap_err().contains("too long"));
        let nodes = format!("{{{}}}", vec!["\"a\":[]"; MAX_NODES].join(","));
        assert!(
            parse(nodes.as_bytes())
                .unwrap_err()
                .contains("too many values")
        );
    }
}
