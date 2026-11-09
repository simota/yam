use anyhow::{anyhow, bail, Context, Result};
use serde::de::{
    self, Deserialize, DeserializeSeed, Deserializer, EnumAccess, MapAccess, SeqAccess,
    VariantAccess, Visitor,
};
use serde::ser::{Serialize, Serializer};
use serde_json::Number;
use std::borrow::Borrow;
use std::collections::hash_map::Entry;
use std::collections::hash_map::RandomState;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::hash::BuildHasher;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Document,
    Mapping,
    Sequence,
    Scalar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarType {
    String,
    Number,
    Boolean,
    Null,
}

#[derive(Debug, Clone, PartialEq)]
pub struct YamNode {
    pub kind: NodeKind,
    pub key: Option<String>,
    pub value: Option<String>,
    pub tag: Option<String>,
    pub children: Vec<YamNode>,
    pub depth: usize,
    pub index: usize,
}

impl YamNode {
    pub fn document(child: YamNode) -> Self {
        Self {
            kind: NodeKind::Document,
            key: None,
            value: None,
            tag: None,
            children: vec![child],
            depth: 0,
            index: 0,
        }
    }

    pub fn has_children(&self) -> bool {
        !self.children.is_empty()
    }

    pub fn infer_type(&self) -> ScalarType {
        match self.tag.as_deref() {
            Some("!!null") => ScalarType::Null,
            Some("!!bool") => ScalarType::Boolean,
            Some("!!int" | "!!float") => ScalarType::Number,
            Some("!!str") => ScalarType::String,
            _ => {
                let value = self.value.as_deref().unwrap_or_default();
                match value.to_ascii_lowercase().as_str() {
                    "" | "null" | "~" => ScalarType::Null,
                    "true" | "false" | "yes" | "no" | "on" | "off" => ScalarType::Boolean,
                    _ if is_number(value) => ScalarType::Number,
                    _ => ScalarType::String,
                }
            }
        }
    }

    pub fn is_container(&self) -> bool {
        matches!(
            self.kind,
            NodeKind::Document | NodeKind::Mapping | NodeKind::Sequence
        )
    }
}

pub fn parse_file(path: impl AsRef<Path>) -> Result<YamNode> {
    let path = path.as_ref();
    let input =
        fs::read(path).with_context(|| format!("failed to open file: {}", path.display()))?;
    if is_json_file(path) {
        parse_json(&input)
    } else {
        parse_yaml(&input)
    }
}

pub fn parse_yaml(input: &[u8]) -> Result<YamNode> {
    let YamlRoot(root) = serde_yaml::from_slice(input).context("failed to parse YAML")?;
    Ok(YamNode::document(root))
}

pub fn parse_json(input: &[u8]) -> Result<YamNode> {
    let JsonRoot(root) = serde_json::from_slice(input).context("failed to parse JSON")?;
    Ok(YamNode::document(root))
}

pub fn parse_yaml_str(input: &str) -> Result<YamNode> {
    parse_yaml(input.as_bytes())
}

pub fn parse_path(path: &str) -> Result<Vec<String>> {
    if path.is_empty() || path == "." {
        return Ok(Vec::new());
    }
    if !path.starts_with('.') {
        bail!("path must start with '.': {path}");
    }

    let bytes = path.as_bytes();
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut i = 1;
    while i < bytes.len() {
        match bytes[i] as char {
            '.' => {
                if !current.is_empty() {
                    segments.push(std::mem::take(&mut current));
                }
                i += 1;
            }
            '[' => {
                if !current.is_empty() {
                    segments.push(std::mem::take(&mut current));
                }
                let Some(end_rel) = path[i..].find(']') else {
                    bail!("unclosed bracket in path: {path}");
                };
                let index = &path[i + 1..i + end_rel];
                index
                    .parse::<usize>()
                    .with_context(|| format!("invalid array index: {index}"))?;
                segments.push(index.to_string());
                i += end_rel + 1;
            }
            ch => {
                current.push(ch);
                i += 1;
            }
        }
    }
    if !current.is_empty() {
        segments.push(current);
    }
    Ok(segments)
}

pub fn get_by_path<'a>(root: &'a YamNode, path: &str) -> Result<&'a YamNode> {
    let segments = parse_path(path)?;
    if segments.is_empty() {
        return Ok(root);
    }

    let mut current = if root.kind == NodeKind::Document && !root.children.is_empty() {
        &root.children[0]
    } else {
        root
    };

    for segment in segments {
        current = &current.children[child_position(current, &segment)?];
    }

    Ok(current)
}

/// Like `get_by_path`, but moves the node out of `root` instead of borrowing
/// it, so selecting a subtree does not copy it.
pub fn take_by_path(mut root: YamNode, path: &str) -> Result<YamNode> {
    let segments = parse_path(path)?;
    if segments.is_empty() {
        return Ok(root);
    }

    let mut current = if root.kind == NodeKind::Document && !root.children.is_empty() {
        root.children.swap_remove(0)
    } else {
        root
    };

    for segment in segments {
        let position = child_position(&current, &segment)?;
        current = current.children.swap_remove(position);
    }

    Ok(current)
}

/// Position in `node.children` of the child that path `segment` selects.
fn child_position(node: &YamNode, segment: &str) -> Result<usize> {
    match node.kind {
        NodeKind::Mapping => node
            .children
            .iter()
            .position(|child| child.key.as_deref() == Some(segment))
            .ok_or_else(|| anyhow!("path not found: {segment}")),
        NodeKind::Sequence => {
            let index: usize = segment
                .parse()
                .with_context(|| format!("expected array index, got: {segment}"))?;
            if index < node.children.len() {
                Ok(index)
            } else {
                bail!(
                    "array index out of bounds: {index} (length: {})",
                    node.children.len()
                )
            }
        }
        _ => bail!("cannot traverse into scalar value at: {segment}"),
    }
}

pub fn get_by_segments<'a>(root: &'a YamNode, segments: &[String]) -> Option<&'a YamNode> {
    let mut current = if root.kind == NodeKind::Document && !root.children.is_empty() {
        &root.children[0]
    } else {
        root
    };

    for segment in segments {
        current = match current.kind {
            NodeKind::Mapping => current
                .children
                .iter()
                .find(|child| child.key.as_deref() == Some(segment.as_str()))?,
            NodeKind::Sequence => current.children.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }

    Some(current)
}

pub fn get_by_segments_mut<'a>(
    root: &'a mut YamNode,
    segments: &[String],
) -> Option<&'a mut YamNode> {
    let mut current = if root.kind == NodeKind::Document && !root.children.is_empty() {
        &mut root.children[0]
    } else {
        root
    };

    for segment in segments {
        current = match current.kind {
            NodeKind::Mapping => current
                .children
                .iter_mut()
                .find(|child| child.key.as_deref() == Some(segment.as_str()))?,
            NodeKind::Sequence => current.children.get_mut(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }

    Some(current)
}

pub fn to_json_value(node: &YamNode) -> serde_json::Value {
    match node.kind {
        NodeKind::Document => node
            .children
            .first()
            .map(to_json_value)
            .unwrap_or(serde_json::Value::Null),
        NodeKind::Mapping => {
            let mut map = serde_json::Map::new();
            for child in &node.children {
                if let Some(key) = &child.key {
                    map.insert(key.clone(), to_json_value(child));
                }
            }
            serde_json::Value::Object(map)
        }
        NodeKind::Sequence => {
            serde_json::Value::Array(node.children.iter().map(to_json_value).collect())
        }
        NodeKind::Scalar => scalar_to_json(node),
    }
}

pub fn to_raw_value(node: &YamNode) -> String {
    if node.kind == NodeKind::Document {
        return node.children.first().map(to_raw_value).unwrap_or_default();
    }
    if node.kind == NodeKind::Scalar {
        return node.value.clone().unwrap_or_default();
    }
    serde_json::to_string(&JsonView(node)).unwrap_or_default()
}

/// Writes `to_json_value(node)` as pretty JSON, streaming it from the tree
/// instead of building the JSON value first.
pub fn write_json_pretty(writer: impl io::Write, node: &YamNode) -> serde_json::Result<()> {
    serde_json::to_writer_pretty(writer, &JsonView(node))
}

/// Serializes a node exactly as its `to_json_value` would serialize.
struct JsonView<'a>(&'a YamNode);

impl Serialize for JsonView<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let node = self.0;
        match node.kind {
            NodeKind::Document => match node.children.first() {
                Some(child) => JsonView(child).serialize(serializer),
                None => serializer.serialize_unit(),
            },
            NodeKind::Mapping => match merge_repeated_keys(&node.children) {
                None => serializer.collect_map(
                    node.children
                        .iter()
                        .map(|child| (child.key.as_deref().unwrap_or_default(), JsonView(child))),
                ),
                Some(entries) => serializer.collect_map(
                    entries
                        .into_iter()
                        .map(|(key, child)| (key, JsonView(child))),
                ),
            },
            NodeKind::Sequence => serializer.collect_seq(node.children.iter().map(JsonView)),
            NodeKind::Scalar => match node.infer_type() {
                ScalarType::String => {
                    serializer.serialize_str(node.value.as_deref().unwrap_or_default())
                }
                _ => scalar_to_json(node).serialize(serializer),
            },
        }
    }
}

/// The entries that inserting `children` in order into an order-preserving
/// map leaves, as building a JSON object or YAML mapping from them does: a
/// repeated key keeps its first position and takes its last value, and a
/// child without a key is skipped. `None` when every child has its own key,
/// so the children are the entries.
fn merge_repeated_keys<T: Borrow<YamNode>>(children: &[T]) -> Option<Vec<(&str, &YamNode)>> {
    fn key<T: Borrow<YamNode>>(child: &T) -> Option<&str> {
        child.borrow().key.as_deref()
    }
    let distinct = if children.len() <= 8 {
        children
            .iter()
            .enumerate()
            .all(|(idx, child)| children[..idx].iter().all(|other| key(other) != key(child)))
    } else {
        let mut seen = HashSet::with_capacity(children.len());
        children.iter().all(|child| seen.insert(key(child)))
    };
    if distinct && children.iter().all(|child| key(child).is_some()) {
        return None;
    }

    let mut entries: Vec<(&str, &YamNode)> = Vec::new();
    let mut positions: HashMap<&str, usize> = HashMap::new();
    for child in children {
        let child = child.borrow();
        let Some(key) = child.key.as_deref() else {
            continue;
        };
        match positions.entry(key) {
            Entry::Occupied(position) => entries[*position.get()].1 = child,
            Entry::Vacant(position) => {
                position.insert(entries.len());
                entries.push((key, child));
            }
        }
    }
    Some(entries)
}

pub fn is_json_file(path: impl AsRef<Path>) -> bool {
    path.as_ref()
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("json"))
        .unwrap_or(false)
}

/// The parsed root of a YAML document. Deserializing it builds the `YamNode`
/// tree straight from the parser's events, with the same results and errors
/// as deserializing a `serde_yaml::Value` and converting that, but without
/// allocating the intermediate tree.
struct YamlRoot(YamNode);

impl<'de> Deserialize<'de> for YamlRoot {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        YamlNodeSeed { depth: 0 }
            .deserialize(deserializer)
            .map(Self)
    }
}

/// Builds a node at `depth`; its parent fills in `key` and `index`.
#[derive(Clone, Copy)]
struct YamlNodeSeed {
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for YamlNodeSeed {
    type Value = YamNode;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<YamNode, D::Error> {
        deserializer.deserialize_any(self)
    }
}

/// Mirrors the callbacks of `serde_yaml::Value`'s visitor one for one, so the
/// deserializer resolves every scalar, tag and error exactly as before.
impl<'de> Visitor<'de> for YamlNodeSeed {
    type Value = YamNode;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("any YAML value")
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<YamNode, E> {
        Ok(scalar_node(
            None,
            value.to_string(),
            "!!bool",
            self.depth,
            0,
        ))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<YamNode, E> {
        Ok(yaml_number_node(value.into(), self.depth))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<YamNode, E> {
        Ok(yaml_number_node(value.into(), self.depth))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<YamNode, E> {
        Ok(yaml_number_node(value.into(), self.depth))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<YamNode, E> {
        self.visit_string(value.to_owned())
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<YamNode, E> {
        Ok(scalar_node(None, value, "!!str", self.depth, 0))
    }

    fn visit_unit<E: de::Error>(self) -> Result<YamNode, E> {
        Ok(null_node(self.depth))
    }

    fn visit_none<E: de::Error>(self) -> Result<YamNode, E> {
        Ok(null_node(self.depth))
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<YamNode, D::Error> {
        self.deserialize(deserializer)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<YamNode, A::Error> {
        let seed = YamlNodeSeed {
            depth: self.depth + 1,
        };
        let mut children = Vec::new();
        while let Some(mut child) = seq.next_element_seed(seed)? {
            child.index = children.len();
            children.push(child);
        }
        Ok(container_node(NodeKind::Sequence, children, self.depth))
    }

    /// Rejects duplicate keys like `serde_yaml::Mapping` does: keys are
    /// compared as YAML values, so `1` and `"1"` are distinct keys even though
    /// both are shown as `1`.
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<YamNode, A::Error> {
        let seed = YamlNodeSeed {
            depth: self.depth + 1,
        };
        let mut children: Vec<YamNode> = Vec::new();
        let mut string_keys = KeyIndex::default();
        // Positions and values of the keys that were not plain strings.
        let mut other_ids: Vec<usize> = Vec::new();
        let mut other_keys: HashSet<serde_yaml::Value> = HashSet::new();

        while let Some(key) = map.next_key::<serde_yaml::Value>()? {
            let id = children.len();
            let key = match key {
                serde_yaml::Value::String(key) => {
                    let is_string_key = |id: usize| other_ids.binary_search(&id).is_err();
                    if string_keys.find(&children, &key, is_string_key).is_some() {
                        return Err(de::Error::custom(format_args!(
                            "duplicate entry with key {key:?}"
                        )));
                    }
                    string_keys.insert(&children, &key, id, is_string_key);
                    key
                }
                key => {
                    if let Some(existing) = other_keys.get(&key) {
                        return Err(de::Error::custom(DuplicateKey(existing)));
                    }
                    let shown = yaml_key_to_string(&key);
                    other_keys.insert(key);
                    other_ids.push(id);
                    shown
                }
            };
            let mut child = map.next_value_seed(seed)?;
            child.key = Some(key);
            child.index = id;
            children.push(child);
        }
        Ok(container_node(NodeKind::Mapping, children, self.depth))
    }

    /// A tagged value is shown as its untagged contents.
    fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<YamNode, A::Error> {
        let (TagString, contents) = data.variant_seed(TagString)?;
        contents.newtype_variant_seed(self)
    }
}

/// Accepts a tag the way `serde_yaml::Value` does, rejecting an empty one.
struct TagString;

impl<'de> DeserializeSeed<'de> for TagString {
    type Value = TagString;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_string(self)
    }
}

impl<'de> Visitor<'de> for TagString {
    type Value = TagString;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a YAML tag string")
    }

    fn visit_str<E: de::Error>(self, tag: &str) -> Result<Self, E> {
        if tag.is_empty() {
            return Err(E::custom("empty YAML tag is not allowed"));
        }
        Ok(self)
    }
}

/// The message `serde_yaml::Mapping` gives for a repeated non-string key.
struct DuplicateKey<'a>(&'a serde_yaml::Value);

impl fmt::Display for DuplicateKey<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("duplicate entry ")?;
        match self.0 {
            serde_yaml::Value::Null => formatter.write_str("with null key"),
            serde_yaml::Value::Bool(boolean) => write!(formatter, "with key `{boolean}`"),
            serde_yaml::Value::Number(number) => write!(formatter, "with key {number}"),
            serde_yaml::Value::String(string) => write!(formatter, "with key {string:?}"),
            serde_yaml::Value::Sequence(_)
            | serde_yaml::Value::Mapping(_)
            | serde_yaml::Value::Tagged(_) => formatter.write_str("in YAML map"),
        }
    }
}

fn yaml_number_node(value: serde_yaml::Number, depth: usize) -> YamNode {
    let tag = if value.as_i64().is_some() || value.as_u64().is_some() {
        "!!int"
    } else {
        "!!float"
    };
    scalar_node(None, value.to_string(), tag, depth, 0)
}

/// The parsed root of a JSON document, built like `YamlRoot` but following
/// `serde_json::Value`'s rules.
struct JsonRoot(YamNode);

impl<'de> Deserialize<'de> for JsonRoot {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        JsonNodeSeed { depth: 0 }
            .deserialize(deserializer)
            .map(Self)
    }
}

#[derive(Clone, Copy)]
struct JsonNodeSeed {
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for JsonNodeSeed {
    type Value = YamNode;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<YamNode, D::Error> {
        deserializer.deserialize_any(self)
    }
}

/// Mirrors the callbacks of `serde_json::Value`'s visitor one for one.
impl<'de> Visitor<'de> for JsonNodeSeed {
    type Value = YamNode;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("any valid JSON value")
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<YamNode, E> {
        Ok(scalar_node(
            None,
            value.to_string(),
            "!!bool",
            self.depth,
            0,
        ))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<YamNode, E> {
        Ok(json_number_node(value.into(), self.depth))
    }

    fn visit_i128<E: de::Error>(self, value: i128) -> Result<YamNode, E> {
        let number = Number::deserialize(de::value::I128Deserializer::new(value))?;
        Ok(json_number_node(number, self.depth))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<YamNode, E> {
        Ok(json_number_node(value.into(), self.depth))
    }

    fn visit_u128<E: de::Error>(self, value: u128) -> Result<YamNode, E> {
        let number = Number::deserialize(de::value::U128Deserializer::new(value))?;
        Ok(json_number_node(number, self.depth))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<YamNode, E> {
        Ok(match Number::from_f64(value) {
            Some(number) => json_number_node(number, self.depth),
            None => null_node(self.depth),
        })
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<YamNode, E> {
        self.visit_string(value.to_owned())
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<YamNode, E> {
        Ok(scalar_node(None, value, "!!str", self.depth, 0))
    }

    fn visit_none<E: de::Error>(self) -> Result<YamNode, E> {
        Ok(null_node(self.depth))
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<YamNode, D::Error> {
        self.deserialize(deserializer)
    }

    fn visit_unit<E: de::Error>(self) -> Result<YamNode, E> {
        Ok(null_node(self.depth))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<YamNode, A::Error> {
        let seed = JsonNodeSeed {
            depth: self.depth + 1,
        };
        let mut children = Vec::new();
        while let Some(mut child) = seq.next_element_seed(seed)? {
            child.index = children.len();
            children.push(child);
        }
        Ok(container_node(NodeKind::Sequence, children, self.depth))
    }

    /// A repeated key keeps its first position and takes the last value, as
    /// inserting into an order-preserving `serde_json::Map` does.
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<YamNode, A::Error> {
        let seed = JsonNodeSeed {
            depth: self.depth + 1,
        };
        let mut children: Vec<YamNode> = Vec::new();
        let mut keys = KeyIndex::default();
        while let Some(key) = map.next_key::<String>()? {
            let mut child = map.next_value_seed(seed)?;
            match keys.find(&children, &key, |_| true) {
                Some(id) => {
                    child.index = id;
                    child.key = Some(key);
                    children[id] = child;
                }
                None => {
                    let id = children.len();
                    keys.insert(&children, &key, id, |_| true);
                    child.index = id;
                    child.key = Some(key);
                    children.push(child);
                }
            }
        }
        Ok(container_node(NodeKind::Mapping, children, self.depth))
    }
}

fn json_number_node(value: Number, depth: usize) -> YamNode {
    let tag = if value.is_i64() || value.is_u64() {
        "!!int"
    } else {
        "!!float"
    };
    scalar_node(None, value.to_string(), tag, depth, 0)
}

/// Finds a mapping's children by key while the mapping is being built.
/// Small mappings are scanned; larger ones get a hash index, so building a
/// mapping stays linear in its size.
#[derive(Default)]
struct KeyIndex {
    /// Hash of a key to the first child registered with that hash.
    by_hash: Option<(RandomState, HashMap<u64, usize>)>,
    len: usize,
}

impl KeyIndex {
    const SCAN_LIMIT: usize = 8;

    /// The registered child whose key is `key`. `registered` tells which
    /// children were registered.
    fn find(
        &self,
        children: &[YamNode],
        key: &str,
        registered: impl Fn(usize) -> bool,
    ) -> Option<usize> {
        let matches = |id: usize| registered(id) && children[id].key.as_deref() == Some(key);
        let scan = || (0..children.len()).find(|&id| matches(id));
        match &self.by_hash {
            None => scan(),
            Some((state, by_hash)) => match by_hash.get(&state.hash_one(key)) {
                None => None,
                Some(&id) if matches(id) => Some(id),
                // Another key with the same hash: fall back to comparing keys.
                Some(_) => scan(),
            },
        }
    }

    /// Registers the child about to be pushed at `id` under `key`.
    fn insert(
        &mut self,
        children: &[YamNode],
        key: &str,
        id: usize,
        registered: impl Fn(usize) -> bool,
    ) {
        self.len += 1;
        if self.by_hash.is_none() && self.len > Self::SCAN_LIMIT {
            let state = RandomState::new();
            let mut by_hash = HashMap::with_capacity(self.len * 2);
            for (existing, child) in children.iter().enumerate() {
                if let (true, Some(existing_key)) = (registered(existing), &child.key) {
                    by_hash
                        .entry(state.hash_one(existing_key.as_str()))
                        .or_insert(existing);
                }
            }
            self.by_hash = Some((state, by_hash));
        }
        if let Some((state, by_hash)) = &mut self.by_hash {
            by_hash.entry(state.hash_one(key)).or_insert(id);
        }
    }
}

fn container_node(kind: NodeKind, mut children: Vec<YamNode>, depth: usize) -> YamNode {
    // The parsers give no length up front, so `children` grew by doubling;
    // trimming it keeps the tree as small as one built from a known length.
    children.shrink_to_fit();
    YamNode {
        kind,
        key: None,
        value: None,
        tag: None,
        children,
        depth,
        index: 0,
    }
}

fn null_node(depth: usize) -> YamNode {
    scalar_node(None, "null".to_string(), "!!null", depth, 0)
}

fn scalar_node(
    key: Option<String>,
    value: String,
    tag: &'static str,
    depth: usize,
    index: usize,
) -> YamNode {
    YamNode {
        kind: NodeKind::Scalar,
        key,
        value: Some(value),
        tag: Some(tag.to_string()),
        children: Vec::new(),
        depth,
        index,
    }
}

fn yaml_key_to_string(value: &serde_yaml::Value) -> String {
    match value {
        serde_yaml::Value::String(value) => value.clone(),
        serde_yaml::Value::Bool(value) => value.to_string(),
        serde_yaml::Value::Number(value) => value.to_string(),
        serde_yaml::Value::Null => "null".to_string(),
        _ => serde_yaml::to_string(value)
            .unwrap_or_default()
            .trim()
            .to_string(),
    }
}

fn scalar_to_json(node: &YamNode) -> serde_json::Value {
    let value = node.value.as_deref().unwrap_or_default();
    match node.infer_type() {
        ScalarType::Null => serde_json::Value::Null,
        ScalarType::Boolean => serde_json::Value::Bool(matches!(value, "true" | "yes" | "on")),
        ScalarType::Number => {
            if let Ok(number) = value.parse::<i64>() {
                serde_json::Value::Number(Number::from(number))
            } else if let Ok(number) = value.parse::<u64>() {
                serde_json::Value::Number(Number::from(number))
            } else if let Ok(number) = value.parse::<f64>() {
                Number::from_f64(number)
                    .map(serde_json::Value::Number)
                    .unwrap_or_else(|| serde_json::Value::String(value.to_string()))
            } else {
                serde_json::Value::String(value.to_string())
            }
        }
        ScalarType::String => serde_json::Value::String(value.to_string()),
    }
}

fn is_number(input: &str) -> bool {
    if input.is_empty() {
        return false;
    }
    let mut chars = input.chars().peekable();
    if matches!(chars.peek(), Some('-' | '+')) {
        chars.next();
        if chars.peek().is_none() {
            return false;
        }
    }

    let mut has_digit = false;
    let mut has_dot = false;
    let mut has_exp = false;
    let mut prev_exp = false;

    for ch in chars {
        match ch {
            '0'..='9' => {
                has_digit = true;
                prev_exp = false;
            }
            '.' if !has_dot && !has_exp => {
                has_dot = true;
                prev_exp = false;
            }
            'e' | 'E' if !has_exp && has_digit => {
                has_exp = true;
                has_digit = false;
                prev_exp = true;
            }
            '+' | '-' if prev_exp => {
                prev_exp = false;
            }
            '_' => {}
            _ => return false,
        }
    }
    has_digit
}

/// `node`'s children ordered by key, keyless ones first; children sharing a
/// key keep their document order.
pub(crate) fn children_sorted_by_key(node: &YamNode) -> Vec<&YamNode> {
    let mut children: Vec<&YamNode> = node.children.iter().collect();
    children.sort_by(|a, b| a.key.cmp(&b.key));
    children
}

/// Serializes a node exactly as its `node_to_ordered_yaml` would serialize.
pub(crate) struct YamlView<'a> {
    pub node: &'a YamNode,
    pub sort_keys: bool,
}

impl Serialize for YamlView<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let node = self.node;
        match node.kind {
            NodeKind::Document => match node.children.first() {
                Some(child) => self.view(child).serialize(serializer),
                None => serializer.serialize_unit(),
            },
            NodeKind::Mapping if self.sort_keys => {
                self.serialize_mapping(serializer, &children_sorted_by_key(node))
            }
            NodeKind::Mapping => self.serialize_mapping(serializer, &node.children),
            NodeKind::Sequence => {
                serializer.collect_seq(node.children.iter().map(|child| self.view(child)))
            }
            NodeKind::Scalar => match node.infer_type() {
                ScalarType::Null => serializer.serialize_unit(),
                ScalarType::Boolean => serializer
                    .serialize_bool(matches!(node.value.as_deref(), Some("true" | "yes" | "on"))),
                ScalarType::Number => {
                    yaml_number(node.value.as_deref().unwrap_or_default()).serialize(serializer)
                }
                ScalarType::String => {
                    serializer.serialize_str(node.value.as_deref().unwrap_or_default())
                }
            },
        }
    }
}

impl<'a> YamlView<'a> {
    fn view(&self, node: &'a YamNode) -> Self {
        Self {
            node,
            sort_keys: self.sort_keys,
        }
    }

    fn serialize_mapping<S: Serializer, T: Borrow<YamNode>>(
        &self,
        serializer: S,
        children: &[T],
    ) -> Result<S::Ok, S::Error> {
        match merge_repeated_keys(children) {
            None => serializer.collect_map(children.iter().map(|child| {
                let child = child.borrow();
                (child.key.as_deref().unwrap_or_default(), self.view(child))
            })),
            Some(entries) => serializer.collect_map(
                entries
                    .into_iter()
                    .map(|(key, child)| (key, self.view(child))),
            ),
        }
    }
}

pub fn node_to_ordered_yaml(node: &YamNode, sort_keys: bool) -> serde_yaml::Value {
    match node.kind {
        NodeKind::Document => node
            .children
            .first()
            .map(|child| node_to_ordered_yaml(child, sort_keys))
            .unwrap_or(serde_yaml::Value::Null),
        NodeKind::Mapping => {
            let children: Vec<&YamNode> = if sort_keys {
                children_sorted_by_key(node)
            } else {
                node.children.iter().collect()
            };
            let mut map = serde_yaml::Mapping::new();
            for child in children {
                if let Some(key) = &child.key {
                    map.insert(
                        serde_yaml::Value::String(key.clone()),
                        node_to_ordered_yaml(child, sort_keys),
                    );
                }
            }
            serde_yaml::Value::Mapping(map)
        }
        NodeKind::Sequence => serde_yaml::Value::Sequence(
            node.children
                .iter()
                .map(|child| node_to_ordered_yaml(child, sort_keys))
                .collect(),
        ),
        NodeKind::Scalar => match node.infer_type() {
            ScalarType::Null => serde_yaml::Value::Null,
            ScalarType::Boolean => serde_yaml::Value::Bool(matches!(
                node.value.as_deref(),
                Some("true" | "yes" | "on")
            )),
            ScalarType::Number => yaml_number(node.value.as_deref().unwrap_or_default()),
            ScalarType::String => serde_yaml::Value::String(node.value.clone().unwrap_or_default()),
        },
    }
}

/// Converts a number scalar without starting a YAML parser for the plain
/// decimal forms that parsing produces; anything else keeps serde_yaml's own
/// resolution rules (leading zeros, `.inf`, exponents, edited values).
fn yaml_number(raw: &str) -> serde_yaml::Value {
    if is_plain_decimal(raw) {
        let number = if raw.contains('.') {
            raw.parse::<f64>().ok().map(serde_yaml::Number::from)
        } else {
            raw.parse::<i64>()
                .map(serde_yaml::Number::from)
                .or_else(|_| raw.parse::<u64>().map(serde_yaml::Number::from))
                .ok()
        };
        if let Some(number) = number {
            return serde_yaml::Value::Number(number);
        }
    }
    serde_yaml::from_str(raw).unwrap_or_else(|_| serde_yaml::Value::String(raw.to_string()))
}

/// `-?(0|[1-9][0-9]*)(\.[0-9]+)?`
fn is_plain_decimal(input: &str) -> bool {
    let digits = input.strip_prefix('-').unwrap_or(input);
    let (int, frac) = match digits.split_once('.') {
        Some((int, frac)) => (int, Some(frac)),
        None => (digits, None),
    };
    let int_ok = int == "0"
        || (!int.is_empty() && !int.starts_with('0') && int.bytes().all(|b| b.is_ascii_digit()));
    int_ok
        && frac
            .iter()
            .all(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The conversion through `serde_yaml::Value` / `serde_json::Value` that
    // parsing used before it built nodes directly; the tests compare the two.
    fn yaml_value_to_node(
        value: serde_yaml::Value,
        key: Option<String>,
        depth: usize,
        index: usize,
    ) -> YamNode {
        match value {
            serde_yaml::Value::Mapping(mapping) => {
                let mut children = Vec::with_capacity(mapping.len());
                for (idx, (key_value, value)) in mapping.into_iter().enumerate() {
                    children.push(yaml_value_to_node(
                        value,
                        Some(yaml_key_to_string(&key_value)),
                        depth + 1,
                        idx,
                    ));
                }
                YamNode {
                    kind: NodeKind::Mapping,
                    key,
                    value: None,
                    tag: None,
                    children,
                    depth,
                    index,
                }
            }
            serde_yaml::Value::Sequence(sequence) => {
                let children = sequence
                    .into_iter()
                    .enumerate()
                    .map(|(idx, value)| yaml_value_to_node(value, None, depth + 1, idx))
                    .collect();
                YamNode {
                    kind: NodeKind::Sequence,
                    key,
                    value: None,
                    tag: None,
                    children,
                    depth,
                    index,
                }
            }
            serde_yaml::Value::Bool(value) => {
                scalar_node(key, value.to_string(), "!!bool", depth, index)
            }
            serde_yaml::Value::Number(value) => {
                let tag = if value.as_i64().is_some() || value.as_u64().is_some() {
                    "!!int"
                } else {
                    "!!float"
                };
                scalar_node(key, value.to_string(), tag, depth, index)
            }
            serde_yaml::Value::String(value) => scalar_node(key, value, "!!str", depth, index),
            serde_yaml::Value::Null => scalar_node(key, "null".to_string(), "!!null", depth, index),
            serde_yaml::Value::Tagged(tagged) => {
                yaml_value_to_node(tagged.value, key, depth, index)
            }
        }
    }

    fn json_value_to_node(
        value: serde_json::Value,
        key: Option<String>,
        depth: usize,
        index: usize,
    ) -> YamNode {
        match value {
            serde_json::Value::Object(map) => {
                let mut children = Vec::with_capacity(map.len());
                for (idx, (key_string, value)) in map.into_iter().enumerate() {
                    children.push(json_value_to_node(value, Some(key_string), depth + 1, idx));
                }
                YamNode {
                    kind: NodeKind::Mapping,
                    key,
                    value: None,
                    tag: None,
                    children,
                    depth,
                    index,
                }
            }
            serde_json::Value::Array(values) => {
                let children = values
                    .into_iter()
                    .enumerate()
                    .map(|(idx, value)| json_value_to_node(value, None, depth + 1, idx))
                    .collect();
                YamNode {
                    kind: NodeKind::Sequence,
                    key,
                    value: None,
                    tag: None,
                    children,
                    depth,
                    index,
                }
            }
            serde_json::Value::String(value) => scalar_node(key, value, "!!str", depth, index),
            serde_json::Value::Bool(value) => {
                scalar_node(key, value.to_string(), "!!bool", depth, index)
            }
            serde_json::Value::Number(value) => {
                let tag = if value.is_i64() || value.is_u64() {
                    "!!int"
                } else {
                    "!!float"
                };
                scalar_node(key, value.to_string(), tag, depth, index)
            }
            serde_json::Value::Null => scalar_node(key, "null".to_string(), "!!null", depth, index),
        }
    }

    fn parse_yaml_via_value(input: &[u8]) -> Result<YamNode> {
        let value: serde_yaml::Value =
            serde_yaml::from_slice(input).context("failed to parse YAML")?;
        Ok(YamNode::document(yaml_value_to_node(value, None, 0, 0)))
    }

    fn parse_json_via_value(input: &[u8]) -> Result<YamNode> {
        let value: serde_json::Value =
            serde_json::from_slice(input).context("failed to parse JSON")?;
        Ok(YamNode::document(json_value_to_node(value, None, 0, 0)))
    }

    fn assert_same_parse(
        input: &str,
        parse: fn(&[u8]) -> Result<YamNode>,
        oracle: fn(&[u8]) -> Result<YamNode>,
    ) {
        match (parse(input.as_bytes()), oracle(input.as_bytes())) {
            (Ok(node), Ok(expected)) => assert_eq!(node, expected, "{input:?}"),
            (Err(err), Err(expected)) => {
                assert_eq!(format!("{err:#}"), format!("{expected:#}"), "{input:?}")
            }
            (node, expected) => panic!("{input:?}: {node:?} != {expected:?}"),
        }
    }

    #[test]
    fn yaml_parse_matches_value_conversion() {
        let many_keys: String = (0..50).map(|i| format!("k{i}: {i}\n")).collect();
        let many_int_keys: String = (0..50).map(|i| format!("{i}: v\n")).collect();
        let late_duplicate = format!("{many_keys}k7: again\n");
        let late_int_duplicate = format!("{many_int_keys}7: again\n");
        let cases = [
            "",
            "~",
            "null",
            "42",
            "-7",
            "18446744073709551615",
            "1.5",
            "1e3",
            ".inf",
            "-.inf",
            ".nan",
            "0x1F",
            "0o17",
            "true",
            "yes",
            "'quoted'",
            "a: 1\nb: [1, 2.5, x, null, true]\nc: {d: e}\n",
            "- a\n- [b, [c]]\n- {}\n- []\n",
            "a: &x {b: 1}\nc: *x\n",
            "base: &b {x: 1}\nderived:\n  <<: *b\n  y: 2\n",
            "!custom tagged\n",
            "a: !custom {b: 1}\nc: !!str 12\nd: !!int '7'\ne: !!float 1\n",
            "? [a, b]\n: complex\n? {k: v}\n: map\n",
            "1: int\n'1': string\ntrue: bool\n'true': s\n~: null\n'null': s\n1.5: f\n",
            "a: 1\na: 2\n",
            "1: a\n1: b\n",
            "true: a\ntrue: b\n",
            "~: a\nnull: b\n",
            "1.5: a\n1.5: b\n",
            "[a]: 1\n[a]: 2\n",
            "!t a: 1\n!t a: 2\n",
            &many_keys,
            &many_int_keys,
            &late_duplicate,
            &late_int_duplicate,
            "a: [1, 2\n",
            "a: b: c\n",
            "--- a\n--- b\n",
            "! a\n",
            "a: *missing\n",
            "key: \"multi\\nline\"\nblock: |\n  x\n  y\nfolded: >\n  p\n  q\n",
            "\u{feff}a: 1\n",
            "a:\n  - b:\n      - c: [[[[1]]]]\n",
        ];
        for input in cases {
            assert_same_parse(input, parse_yaml, parse_yaml_via_value);
        }
        let deep = format!("{}1{}", "[".repeat(200), "]".repeat(200));
        assert_same_parse(&deep, parse_yaml, parse_yaml_via_value);
    }

    #[test]
    fn json_parse_matches_value_conversion() {
        let many_keys: Vec<String> = (0..50).map(|i| format!("\"k{i}\": {i}")).collect();
        let many_keys = many_keys.join(", ");
        let late_duplicate = format!("{{{many_keys}, \"k7\": \"again\", \"k0\": [1]}}");
        let many_keys = format!("{{{many_keys}}}");
        let cases = [
            "null",
            "42",
            "-7",
            "18446744073709551615",
            "18446744073709551616",
            "-9223372036854775809",
            "1.0",
            "1e3",
            "1e400",
            "-0.0",
            "\"s\\u00e9\\n\"",
            "true",
            "[]",
            "{}",
            "[1, [2, [3]], {\"a\": null}]",
            "{\"a\": 1, \"b\": {\"c\": [true, false]}}",
            "{\"a\": 1, \"b\": 2, \"a\": 3}",
            "{\"a\": {\"x\": 1}, \"a\": [2]}",
            &many_keys,
            &late_duplicate,
            "",
            "{\"a\": 1,}",
            "[1, 2] trailing",
            "{1: 2}",
            "NaN",
        ];
        for input in cases {
            assert_same_parse(input, parse_json, parse_json_via_value);
        }
        let deep = format!("{}1{}", "[".repeat(200), "]".repeat(200));
        assert_same_parse(&deep, parse_json, parse_json_via_value);
    }

    /// Trees covering every scalar type, repeated shown keys, keyless and
    /// untagged children, and values edited to disagree with their tags.
    fn output_corpus() -> Vec<YamNode> {
        let mut nodes: Vec<YamNode> = [
            "",
            "~",
            "42",
            "-0.0",
            "1e3",
            ".inf",
            ".nan",
            "18446744073709551615",
            "plain",
            "'true'",
            "a: 1\nb: [1, 2.5, x, null, true, '', ' pad']\nc: {d: e}\ne: {}\nf: []\n",
            "- a\n- [b, [c]]\n- {}\n- []\n",
            "1: int\n'1': string\ntrue: bool\n'true': s\n~: null\n'null': s\n",
            "z: 1\n'1': a\ny: 2\n1: b\nx: 3\n",
            "'!tag': value\n",
            "'!tag': value\nother: 1\n",
            "zebra: 1\napple: {z: 1, a: 2}\nmango: [3, {b: 1, a: 0}]\n",
            "base: &b {x: 1}\nderived:\n  <<: *b\n  y: 2\n",
            "? [a, b]\n: complex\n? {k: v}\n: map\n",
            "s: \"multi\\nline\"\nq: 'a: b'\nh: '# c'\nu: \"\\u00e9\"\n",
        ]
        .iter()
        .map(|input| parse_yaml_str(input).unwrap())
        .collect();
        let many: String = (0..30).map(|i| format!("k{}: {i}\n", i % 20)).collect();
        nodes.push(
            parse_json(
                format!(
                    "{{{}}}",
                    many.lines()
                        .map(|l| {
                            let (k, v) = l.split_once(": ").unwrap();
                            format!("\"{k}\": {v}")
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                )
                .as_bytes(),
            )
            .unwrap(),
        );
        let mut many_shown_duplicates = String::new();
        for i in 0..12 {
            many_shown_duplicates.push_str(&format!("{i}: int{i}\n'{i}': str{i}\n"));
        }
        nodes.push(parse_yaml_str(&many_shown_duplicates).unwrap());

        // Values edited in the TUI keep their tags; also drop tags and keys.
        let mut edited = parse_yaml_str("n: 1\nb: true\nz: ~\ns: x\nf: 1.5\nu: 2\n").unwrap();
        let map = &mut edited.children[0];
        for (child, value) in map
            .children
            .iter_mut()
            .zip(["abc", "yes", "x", "007", "1e400", "-3"])
        {
            child.value = Some(value.to_string());
        }
        map.children[5].tag = None;
        nodes.push(edited.clone());
        let mut untagged =
            parse_yaml_str("a: ON\nb: '12'\nc: ''\nd: '~'\ne: Null\nf: 1_000\n").unwrap();
        for child in &mut untagged.children[0].children {
            child.tag = None;
        }
        nodes.push(untagged);
        let mut keyless = parse_yaml_str("a: 1\nb: 2\nc: 3\n").unwrap();
        keyless.children[0].children[1].key = None;
        nodes.push(keyless);
        nodes.push(YamNode {
            children: Vec::new(),
            ..YamNode::document(null_node(0))
        });
        nodes
    }

    #[test]
    fn streamed_json_matches_json_value() {
        for node in output_corpus() {
            let expected = to_json_value(&node);
            let mut streamed = Vec::new();
            write_json_pretty(&mut streamed, &node).unwrap();
            assert_eq!(
                String::from_utf8(streamed).unwrap(),
                serde_json::to_string_pretty(&expected).unwrap(),
                "{node:?}"
            );
            let raw_expected = match node.kind {
                NodeKind::Document | NodeKind::Scalar => None,
                _ => Some(serde_json::to_string(&expected).unwrap()),
            };
            if let Some(raw_expected) = raw_expected {
                assert_eq!(to_raw_value(&node), raw_expected);
            }
            for child in &node.children {
                if child.is_container() {
                    assert_eq!(
                        to_raw_value(child),
                        serde_json::to_string(&to_json_value(child)).unwrap()
                    );
                }
            }
        }
    }

    #[test]
    fn streamed_yaml_matches_ordered_yaml_value() {
        for node in output_corpus() {
            for sort_keys in [false, true] {
                assert_eq!(
                    serde_yaml::to_string(&YamlView {
                        node: &node,
                        sort_keys
                    })
                    .unwrap(),
                    serde_yaml::to_string(&node_to_ordered_yaml(&node, sort_keys)).unwrap(),
                    "{node:?} sort_keys={sort_keys}"
                );
            }
        }
    }

    #[test]
    fn take_by_path_matches_get_by_path() {
        let root = parse_yaml_str(
            "metadata:\n  name: yam\n  labels: {a: 1, b: 2}\nitems:\n  - x\n  - {y: [1, 2]}\n",
        )
        .unwrap();
        for path in [
            "",
            ".",
            ".metadata",
            ".metadata.labels.b",
            ".items",
            ".items[1].y[0]",
            ".items.1.y",
            ".metadata.missing",
            ".metadata.name.x",
            ".items[9]",
            ".items.x",
            ".items[x]",
            "metadata",
            ".items[1",
        ] {
            let expected = get_by_path(&root, path).cloned();
            match (take_by_path(root.clone(), path), expected) {
                (Ok(node), Ok(expected)) => assert_eq!(node, expected, "{path}"),
                (Err(err), Err(expected)) => {
                    assert_eq!(format!("{err:#}"), format!("{expected:#}"), "{path}")
                }
                (node, expected) => panic!("{path}: {node:?} != {expected:?}"),
            }
        }
        let scalar = parse_yaml_str("5").unwrap();
        assert_eq!(
            take_by_path(scalar.clone(), ".a").unwrap_err().to_string(),
            get_by_path(&scalar, ".a").unwrap_err().to_string()
        );
    }

    #[test]
    fn key_index_handles_scans_hashes_and_unregistered_children() {
        let mut children = Vec::new();
        let mut keys = KeyIndex::default();
        for i in 0..40 {
            let key = format!("k{i}");
            // Every fifth child is left unregistered, like a YAML key that was
            // not a plain string.
            if i % 5 != 0 {
                assert_eq!(keys.find(&children, &key, |id| id % 5 != 0), None);
                keys.insert(&children, &key, i, |id| id % 5 != 0);
            }
            let mut child = null_node(1);
            child.key = Some(key);
            children.push(child);
        }
        for i in 0..40 {
            let expected = (i % 5 != 0).then_some(i);
            assert_eq!(
                keys.find(&children, &format!("k{i}"), |id| id % 5 != 0),
                expected
            );
        }
        assert_eq!(keys.find(&children, "missing", |_| true), None);
    }

    #[test]
    fn yaml_number_matches_serde_yaml_resolution() {
        for raw in [
            "0",
            "1",
            "-1",
            "-0",
            "42",
            "007",
            "+7",
            "1_000",
            "1.5",
            "-0.0",
            "0.5",
            ".5",
            "1.",
            "1.0",
            "1000.0",
            "1e3",
            "1E-2",
            "+.5",
            "9223372036854775807",
            "-9223372036854775808",
            "18446744073709551615",
            "18446744073709551616",
            ".inf",
            "-.inf",
            ".nan",
            "abc",
            "1.2.3",
        ] {
            let expected = serde_yaml::from_str(raw)
                .unwrap_or_else(|_| serde_yaml::Value::String(raw.to_string()));
            assert_eq!(
                serde_yaml::to_string(&yaml_number(raw)).unwrap(),
                serde_yaml::to_string(&expected).unwrap(),
                "{raw}"
            );
        }
    }

    #[test]
    fn parses_path_segments() {
        assert_eq!(parse_path(".foo.bar").unwrap(), vec!["foo", "bar"]);
        assert_eq!(
            parse_path(".items[0].name").unwrap(),
            vec!["items", "0", "name"]
        );
        assert!(parse_path("foo").is_err());
        assert!(parse_path(".items[x]").is_err());
    }

    #[test]
    fn gets_node_by_path() {
        let root = parse_yaml_str("items:\n  - name: first\n").unwrap();
        let node = get_by_path(&root, ".items[0].name").unwrap();
        assert_eq!(node.value.as_deref(), Some("first"));
    }

    #[test]
    fn raw_value_returns_scalar_or_compact_json() {
        let root = parse_yaml_str("name: yam\nitems:\n  - one\n").unwrap();
        assert_eq!(to_raw_value(get_by_path(&root, ".name").unwrap()), "yam");
        assert_eq!(
            to_raw_value(get_by_path(&root, ".items").unwrap()),
            "[\"one\"]"
        );
    }
}
