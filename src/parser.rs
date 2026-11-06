use anyhow::{bail, Context, Result};
use serde_json::Number;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// Several documents ("---" separated); children are Document nodes.
    Stream,
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

    /// A stream of several documents.
    pub fn stream(docs: Vec<YamNode>) -> Self {
        let children = docs
            .into_iter()
            .enumerate()
            .map(|(index, mut doc)| {
                doc.index = index;
                doc
            })
            .collect();
        Self {
            kind: NodeKind::Stream,
            key: None,
            value: None,
            tag: None,
            children,
            depth: 0,
            index: 0,
        }
    }

    /// Builds a tree from documents: one document is returned as is, several
    /// become a stream.
    pub fn from_documents(mut docs: Vec<YamNode>) -> Self {
        if docs.len() == 1 {
            docs.pop().expect("one document")
        } else {
            Self::stream(docs)
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
            NodeKind::Stream | NodeKind::Document | NodeKind::Mapping | NodeKind::Sequence
        )
    }

    /// Short type label of a scalar: str, int, float, bool or null.
    pub fn type_label(&self) -> &'static str {
        match self.infer_type() {
            ScalarType::String => "str",
            ScalarType::Boolean => "bool",
            ScalarType::Null => "null",
            ScalarType::Number => match self.tag.as_deref() {
                Some("!!float") => "float",
                Some("!!int") => "int",
                _ => {
                    let v = self.value.as_deref().unwrap_or_default();
                    if v.contains(['.', 'e', 'E']) || v.ends_with("inf") || v.ends_with("nan") {
                        "float"
                    } else {
                        "int"
                    }
                }
            },
        }
    }
}

/// The documents of a tree: every document of a stream, or the node itself.
pub fn documents(root: &YamNode) -> Vec<&YamNode> {
    if root.kind == NodeKind::Stream {
        root.children.iter().collect()
    } else {
        vec![root]
    }
}

/// Mutable access to the documents of a tree.
pub fn documents_mut(root: &mut YamNode) -> Vec<&mut YamNode> {
    if root.kind == NodeKind::Stream {
        root.children.iter_mut().collect()
    } else {
        vec![root]
    }
}

/// The top-level content of a document (the document node itself otherwise).
pub fn content(node: &YamNode) -> &YamNode {
    if node.kind == NodeKind::Document {
        node.children.first().unwrap_or(node)
    } else {
        node
    }
}

/// "Kind/namespace/name" (or "Kind/name") for Kubernetes-style documents.
pub fn resource_id(doc: &YamNode) -> Option<String> {
    let root = content(doc);
    if root.kind != NodeKind::Mapping {
        return None;
    }
    let child = |node: &YamNode, key: &str| {
        node.children
            .iter()
            .find(|c| c.key.as_deref() == Some(key))
            .cloned()
    };
    let scalar = |node: Option<YamNode>| {
        node.filter(|n| n.kind == NodeKind::Scalar)
            .and_then(|n| n.value)
            .filter(|v| !v.is_empty())
    };
    let kind = scalar(child(root, "kind"))?;
    let metadata = child(root, "metadata")?;
    let name = scalar(child(&metadata, "name"))?;
    Some(match scalar(child(&metadata, "namespace")) {
        Some(ns) => format!("{kind}/{ns}/{name}"),
        None => format!("{kind}/{name}"),
    })
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

/// Parses YAML. Several documents ("---") produce a stream node; merge keys
/// ("<<") are applied and aliases resolved.
pub fn parse_yaml(input: &[u8]) -> Result<YamNode> {
    use serde::Deserialize;
    let mut docs = Vec::new();
    for (idx, de) in serde_yaml::Deserializer::from_slice(input).enumerate() {
        let mut value = serde_yaml::Value::deserialize(de).with_context(|| {
            if idx == 0 {
                "failed to parse YAML".to_string()
            } else {
                format!("failed to parse YAML (document {})", idx + 1)
            }
        })?;
        value
            .apply_merge()
            .context("failed to apply YAML merge keys (<<)")?;
        docs.push(YamNode::document(yaml_value_to_node(value, None, 0, 0)));
    }
    if docs.is_empty() {
        // An empty input is a single null document.
        docs.push(YamNode::document(yaml_value_to_node(
            serde_yaml::Value::Null,
            None,
            0,
            0,
        )));
    }
    Ok(YamNode::from_documents(docs))
}

/// Parses JSON with key order preserved. Several concatenated values
/// (JSON Lines) produce a stream node.
pub fn parse_json(input: &[u8]) -> Result<YamNode> {
    let mut docs = Vec::new();
    for value in serde_json::Deserializer::from_slice(input).into_iter::<serde_json::Value>() {
        let value = value.context("failed to parse JSON")?;
        docs.push(YamNode::document(json_value_to_node(value, None, 0, 0)));
    }
    if docs.is_empty() {
        bail!("failed to parse JSON: empty input");
    }
    Ok(YamNode::from_documents(docs))
}

pub fn parse_yaml_str(input: &str) -> Result<YamNode> {
    parse_yaml(input.as_bytes())
}

pub fn to_json_value(node: &YamNode) -> serde_json::Value {
    match node.kind {
        NodeKind::Stream => {
            serde_json::Value::Array(node.children.iter().map(to_json_value).collect())
        }
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
    serde_json::to_string(&to_json_value(node)).unwrap_or_default()
}

pub fn is_json_file(path: impl AsRef<Path>) -> bool {
    path.as_ref()
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            ["json", "jsonl", "ndjson"]
                .iter()
                .any(|known| ext.eq_ignore_ascii_case(known))
        })
        .unwrap_or(false)
}

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
        serde_yaml::Value::Tagged(tagged) => yaml_value_to_node(tagged.value, key, depth, index),
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

pub fn node_to_ordered_yaml(node: &YamNode, sort_keys: bool) -> serde_yaml::Value {
    match node.kind {
        NodeKind::Stream => serde_yaml::Value::Sequence(
            node.children
                .iter()
                .map(|child| node_to_ordered_yaml(child, sort_keys))
                .collect(),
        ),
        NodeKind::Document => node
            .children
            .first()
            .map(|child| node_to_ordered_yaml(child, sort_keys))
            .unwrap_or(serde_yaml::Value::Null),
        NodeKind::Mapping => {
            let mut children: Vec<&YamNode> = node.children.iter().collect();
            if sort_keys {
                children.sort_by(|a, b| a.key.cmp(&b.key));
            }
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
    use crate::path::get_by_path;

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
    fn raw_value_returns_scalar_or_compact_json() {
        let root = parse_yaml_str("name: yam\nitems:\n  - one\n").unwrap();
        assert_eq!(to_raw_value(get_by_path(&root, ".name").unwrap()), "yam");
        assert_eq!(
            to_raw_value(get_by_path(&root, ".items").unwrap()),
            "[\"one\"]"
        );
    }
}
