use anyhow::{anyhow, bail, Context, Result};
use indexmap::IndexMap;
use serde_json::Number;
use std::fs::File;
use std::io::Read;
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
    pub path: Vec<String>,
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
            path: Vec::new(),
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

    pub fn path_string(&self) -> String {
        if self.path.is_empty() {
            "$".to_string()
        } else {
            format!("$.{}", self.path.join("."))
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
    let mut file =
        File::open(path).with_context(|| format!("failed to open file: {}", path.display()))?;
    if is_json_file(path) {
        parse_json(&mut file)
    } else {
        parse_yaml(&mut file)
    }
}

pub fn parse_yaml(reader: &mut dyn Read) -> Result<YamNode> {
    let value: serde_yaml::Value =
        serde_yaml::from_reader(reader).context("failed to parse YAML")?;
    let root = yaml_value_to_node(value, None, 0, Vec::new(), 0);
    Ok(YamNode::document(root))
}

pub fn parse_json(reader: &mut dyn Read) -> Result<YamNode> {
    let value: serde_json::Value =
        serde_json::from_reader(reader).context("failed to parse JSON")?;
    let root = json_value_to_node(value, None, 0, Vec::new(), 0);
    Ok(YamNode::document(root))
}

pub fn parse_yaml_str(input: &str) -> Result<YamNode> {
    let mut bytes = input.as_bytes();
    parse_yaml(&mut bytes)
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
        match current.kind {
            NodeKind::Mapping => {
                current = current
                    .children
                    .iter()
                    .find(|child| child.key.as_deref() == Some(segment.as_str()))
                    .ok_or_else(|| anyhow!("path not found: {segment}"))?;
            }
            NodeKind::Sequence => {
                let index: usize = segment
                    .parse()
                    .with_context(|| format!("expected array index, got: {segment}"))?;
                current = current.children.get(index).ok_or_else(|| {
                    anyhow!(
                        "array index out of bounds: {index} (length: {})",
                        current.children.len()
                    )
                })?;
            }
            _ => bail!("cannot traverse into scalar value at: {segment}"),
        }
    }

    Ok(current)
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
    serde_json::to_string(&to_json_value(node)).unwrap_or_default()
}

pub fn is_json_file(path: impl AsRef<Path>) -> bool {
    path.as_ref()
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("json"))
        .unwrap_or(false)
}

fn yaml_value_to_node(
    value: serde_yaml::Value,
    key: Option<String>,
    depth: usize,
    path: Vec<String>,
    index: usize,
) -> YamNode {
    match value {
        serde_yaml::Value::Mapping(mapping) => {
            let mut children = Vec::with_capacity(mapping.len());
            for (idx, (key_value, value)) in mapping.into_iter().enumerate() {
                let key_string = yaml_key_to_string(&key_value);
                let mut child_path = path.clone();
                child_path.push(key_string.clone());
                children.push(yaml_value_to_node(
                    value,
                    Some(key_string),
                    depth + 1,
                    child_path,
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
                path,
                index,
            }
        }
        serde_yaml::Value::Sequence(sequence) => {
            let children = sequence
                .into_iter()
                .enumerate()
                .map(|(idx, value)| {
                    let mut child_path = path.clone();
                    child_path.push(idx.to_string());
                    yaml_value_to_node(value, None, depth + 1, child_path, idx)
                })
                .collect();
            YamNode {
                kind: NodeKind::Sequence,
                key,
                value: None,
                tag: None,
                children,
                depth,
                path,
                index,
            }
        }
        serde_yaml::Value::Bool(value) => {
            scalar_node(key, value.to_string(), "!!bool", depth, path, index)
        }
        serde_yaml::Value::Number(value) => {
            let tag = if value.as_i64().is_some() || value.as_u64().is_some() {
                "!!int"
            } else {
                "!!float"
            };
            scalar_node(key, value.to_string(), tag, depth, path, index)
        }
        serde_yaml::Value::String(value) => scalar_node(key, value, "!!str", depth, path, index),
        serde_yaml::Value::Null => {
            scalar_node(key, "null".to_string(), "!!null", depth, path, index)
        }
        serde_yaml::Value::Tagged(tagged) => {
            yaml_value_to_node(tagged.value, key, depth, path, index)
        }
    }
}

fn json_value_to_node(
    value: serde_json::Value,
    key: Option<String>,
    depth: usize,
    path: Vec<String>,
    index: usize,
) -> YamNode {
    match value {
        serde_json::Value::Object(map) => {
            let mut children = Vec::with_capacity(map.len());
            for (idx, (key_string, value)) in map.into_iter().enumerate() {
                let mut child_path = path.clone();
                child_path.push(key_string.clone());
                children.push(json_value_to_node(
                    value,
                    Some(key_string),
                    depth + 1,
                    child_path,
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
                path,
                index,
            }
        }
        serde_json::Value::Array(values) => {
            let children = values
                .into_iter()
                .enumerate()
                .map(|(idx, value)| {
                    let mut child_path = path.clone();
                    child_path.push(idx.to_string());
                    json_value_to_node(value, None, depth + 1, child_path, idx)
                })
                .collect();
            YamNode {
                kind: NodeKind::Sequence,
                key,
                value: None,
                tag: None,
                children,
                depth,
                path,
                index,
            }
        }
        serde_json::Value::String(value) => scalar_node(key, value, "!!str", depth, path, index),
        serde_json::Value::Bool(value) => {
            scalar_node(key, value.to_string(), "!!bool", depth, path, index)
        }
        serde_json::Value::Number(value) => {
            let tag = if value.is_i64() || value.is_u64() {
                "!!int"
            } else {
                "!!float"
            };
            scalar_node(key, value.to_string(), tag, depth, path, index)
        }
        serde_json::Value::Null => {
            scalar_node(key, "null".to_string(), "!!null", depth, path, index)
        }
    }
}

fn scalar_node(
    key: Option<String>,
    value: String,
    tag: &'static str,
    depth: usize,
    path: Vec<String>,
    index: usize,
) -> YamNode {
    YamNode {
        kind: NodeKind::Scalar,
        key,
        value: Some(value),
        tag: Some(tag.to_string()),
        children: Vec::new(),
        depth,
        path,
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
            ScalarType::Number => {
                let raw = node.value.as_deref().unwrap_or_default();
                serde_yaml::from_str(raw)
                    .unwrap_or_else(|_| serde_yaml::Value::String(raw.to_string()))
            }
            ScalarType::String => serde_yaml::Value::String(node.value.clone().unwrap_or_default()),
        },
    }
}

pub fn ordered_map_from_node(node: &YamNode) -> IndexMap<String, serde_json::Value> {
    let mut map = IndexMap::new();
    if node.kind == NodeKind::Mapping {
        for child in &node.children {
            if let Some(key) = &child.key {
                map.insert(key.clone(), to_json_value(child));
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

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
