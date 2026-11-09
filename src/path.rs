//! Path queries (`.a.b[0]`, `[*]`, `..key`, `["a.b"]`) and path-based edits.

use crate::parser::{content, documents, documents_mut, parse_yaml, NodeKind, YamNode};
use anyhow::{anyhow, bail, Result};

/// One step of a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SegKind {
    /// `.name` or `["name"]`
    Key(String),
    /// `[N]`; negative indexes count from the end
    Index(isize),
    /// `.*` or `[*]`
    Wildcard,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub kind: SegKind,
    /// Preceded by `..`: matches at any depth.
    pub recursive: bool,
}

impl Segment {
    pub fn key(key: impl Into<String>) -> Self {
        Self {
            kind: SegKind::Key(key.into()),
            recursive: false,
        }
    }

    pub fn index(index: isize) -> Self {
        Self {
            kind: SegKind::Index(index),
            recursive: false,
        }
    }
}

impl std::fmt::Display for Segment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.recursive {
            f.write_str(".")?;
        }
        match &self.kind {
            SegKind::Key(key) => f.write_str(&key_segment(key)),
            SegKind::Index(idx) => write!(f, "[{idx}]"),
            SegKind::Wildcard => f.write_str("[*]"),
        }
    }
}

/// Parses a path expression.
///
/// ```text
/// .                 the document
/// .key.sub          mapping keys
/// .items[0]         sequence index (negative counts from the end)
/// .items[*] / .*    every element / every value
/// .["a.b"] .['k']   quoted keys
/// ..name            "name" at any depth
/// ```
pub fn parse_path(path: &str) -> Result<Vec<Segment>> {
    if path.is_empty() || path == "." {
        return Ok(Vec::new());
    }
    if !path.starts_with('.') && !path.starts_with('[') {
        bail!("path must start with '.': {path}");
    }
    let chars: Vec<char> = path.chars().collect();
    let n = chars.len();
    let mut segs = Vec::new();
    let mut i = 0;
    let mut recursive = false;

    while i < n {
        match chars[i] {
            '.' => {
                i += 1;
                if i < n && chars[i] == '.' {
                    if recursive {
                        bail!("invalid path {path:?}: too many dots");
                    }
                    recursive = true;
                    continue;
                }
                if i >= n {
                    if recursive {
                        bail!("invalid path {path:?}: '..' must be followed by a key");
                    }
                    bail!("invalid path {path:?}: trailing '.'");
                }
                if chars[i] == '[' {
                    continue;
                }
                let start = i;
                while i < n && chars[i] != '.' && chars[i] != '[' {
                    i += 1;
                }
                let name: String = chars[start..i].iter().collect();
                let kind = if name == "*" {
                    SegKind::Wildcard
                } else {
                    SegKind::Key(name)
                };
                segs.push(Segment { kind, recursive });
                recursive = false;
            }
            '[' => {
                let (kind, next) = parse_bracket(path, &chars, i)?;
                segs.push(Segment { kind, recursive });
                recursive = false;
                i = next;
            }
            _ => bail!("invalid path {path:?} at position {i}"),
        }
    }
    if recursive {
        bail!("invalid path {path:?}: '..' must be followed by a key");
    }
    Ok(segs)
}

fn parse_bracket(path: &str, chars: &[char], start: usize) -> Result<(SegKind, usize)> {
    let n = chars.len();
    let mut j = start + 1;
    if j < n && (chars[j] == '"' || chars[j] == '\'') {
        let quote = chars[j];
        let mut key = String::new();
        j += 1;
        while j < n && chars[j] != quote {
            if chars[j] == '\\' && j + 1 < n {
                j += 1;
            }
            key.push(chars[j]);
            j += 1;
        }
        if j >= n {
            bail!("unclosed quote in path: {path}");
        }
        j += 1;
        if j >= n || chars[j] != ']' {
            bail!("expected ']' after quoted key in path: {path}");
        }
        return Ok((SegKind::Key(key), j + 1));
    }
    let Some(end) = chars[start..].iter().position(|&c| c == ']') else {
        bail!("unclosed bracket in path: {path}");
    };
    let inner: String = chars[start + 1..start + end].iter().collect();
    let inner = inner.trim();
    let next = start + end + 1;
    if inner == "*" {
        return Ok((SegKind::Wildcard, next));
    }
    let idx = inner
        .parse::<isize>()
        .map_err(|_| anyhow!("invalid array index: {inner}"))?;
    Ok((SegKind::Index(idx), next))
}

/// Whether a path can match more than one node.
pub fn is_multi(segs: &[Segment]) -> bool {
    segs.iter()
        .any(|s| s.recursive || s.kind == SegKind::Wildcard)
}

/// A step of a concrete node location.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PathPart {
    Key(String),
    Index(usize),
}

/// Formats a location in path syntax (".a.b[0]"; the document is ".").
pub fn format_path(parts: &[PathPart]) -> String {
    if parts.is_empty() {
        return ".".into();
    }
    parts
        .iter()
        .map(|p| match p {
            PathPart::Key(k) => key_segment(k),
            PathPart::Index(i) => format!("[{i}]"),
        })
        .collect()
}

/// Formats a mapping key as a path segment, quoting it when needed.
pub fn key_segment(key: &str) -> String {
    let plain = !key.is_empty()
        && key != "*"
        && key
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '/' | '$' | '@' | ':' | '+'));
    if plain {
        format!(".{key}")
    } else {
        format!("[\"{}\"]", key.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

fn resolve_index(idx: isize, len: usize) -> Option<usize> {
    let resolved = if idx < 0 { len as isize + idx } else { idx };
    (resolved >= 0 && (resolved as usize) < len).then_some(resolved as usize)
}

/// Strict lookup of a single node in the first document, with descriptive
/// errors. Wildcard paths return their first match.
pub fn get_by_path<'a>(root: &'a YamNode, path: &str) -> Result<&'a YamNode> {
    let segs = parse_path(path)?;
    if segs.is_empty() {
        return Ok(root);
    }
    if is_multi(&segs) {
        return query(root, path)?
            .into_iter()
            .next()
            .map(|hit| hit.node)
            .ok_or_else(|| anyhow!("path not found: {path}"));
    }
    let mut current = content(documents(root)[0]);
    for seg in &segs {
        current = step(current, seg)?;
    }
    Ok(current)
}

fn step<'a>(node: &'a YamNode, seg: &Segment) -> Result<&'a YamNode> {
    match (node.kind, &seg.kind) {
        (NodeKind::Mapping, SegKind::Key(key)) => node
            .children
            .iter()
            .find(|c| c.key.as_deref() == Some(key))
            .ok_or_else(|| anyhow!("path not found: {key}")),
        (NodeKind::Mapping, _) => bail!("expected key, got {seg} on a mapping"),
        (NodeKind::Sequence, SegKind::Index(idx)) => resolve_index(*idx, node.children.len())
            .map(|i| &node.children[i])
            .ok_or_else(|| {
                anyhow!(
                    "array index out of bounds: {idx} (length: {})",
                    node.children.len()
                )
            }),
        (NodeKind::Sequence, _) => bail!("expected array index, got: {seg}"),
        _ => {
            let name = match &seg.kind {
                SegKind::Key(key) => key.clone(),
                _ => seg.to_string(),
            };
            bail!("cannot traverse into scalar value at: {name}")
        }
    }
}

/// A query result: the node, the document it is in and its location.
#[derive(Debug, Clone)]
pub struct Hit<'a> {
    pub node: &'a YamNode,
    pub doc: usize,
    pub path: Vec<PathPart>,
}

/// Evaluates a path against every document. Documents without a match
/// contribute nothing; only invalid syntax is an error.
pub fn query<'a>(root: &'a YamNode, path: &str) -> Result<Vec<Hit<'a>>> {
    let segs = parse_path(path)?;
    let mut out = Vec::new();
    for (doc_idx, doc) in documents(root).into_iter().enumerate() {
        if segs.is_empty() {
            out.push(Hit {
                node: doc,
                doc: doc_idx,
                path: Vec::new(),
            });
            continue;
        }
        let mut current = vec![(content(doc), Vec::new())];
        for seg in &segs {
            let mut next = Vec::new();
            for (node, parts) in current {
                if seg.recursive {
                    walk_with_path(node, parts, &mut |n, p| match_segment(n, p, seg, &mut next));
                } else {
                    match_segment(node, parts, seg, &mut next);
                }
            }
            current = next;
            if current.is_empty() {
                break;
            }
        }
        out.extend(current.into_iter().map(|(node, path)| Hit {
            node,
            doc: doc_idx,
            path,
        }));
    }
    Ok(out)
}

fn walk_with_path<'a>(
    node: &'a YamNode,
    parts: Vec<PathPart>,
    f: &mut impl FnMut(&'a YamNode, Vec<PathPart>),
) {
    f(node, parts.clone());
    for (i, child) in node.children.iter().enumerate() {
        let mut p = parts.clone();
        p.push(child_part(node, child, i));
        walk_with_path(child, p, f);
    }
}

fn child_part(parent: &YamNode, child: &YamNode, i: usize) -> PathPart {
    match (parent.kind, &child.key) {
        (NodeKind::Mapping, Some(key)) => PathPart::Key(key.clone()),
        _ => PathPart::Index(i),
    }
}

fn match_segment<'a>(
    node: &'a YamNode,
    parts: Vec<PathPart>,
    seg: &Segment,
    out: &mut Vec<(&'a YamNode, Vec<PathPart>)>,
) {
    let with = |part: PathPart| {
        let mut p = parts.clone();
        p.push(part);
        p
    };
    match (node.kind, &seg.kind) {
        (NodeKind::Mapping, SegKind::Key(key)) => {
            if let Some(c) = node.children.iter().find(|c| c.key.as_deref() == Some(key)) {
                out.push((c, with(PathPart::Key(key.clone()))));
            }
        }
        (NodeKind::Mapping | NodeKind::Sequence, SegKind::Wildcard) => {
            for (i, c) in node.children.iter().enumerate() {
                out.push((c, with(child_part(node, c, i))));
            }
        }
        (NodeKind::Sequence, SegKind::Index(idx)) => {
            if let Some(i) = resolve_index(*idx, node.children.len()) {
                out.push((&node.children[i], with(PathPart::Index(i))));
            }
        }
        _ => {}
    }
}

/// Every scalar leaf (and empty container) as "path = json-value" lines.
/// Documents of a stream are prefixed with "#N ".
pub fn leaf_paths(root: &YamNode) -> Vec<String> {
    let docs = documents(root);
    let multi = docs.len() > 1;
    let mut lines = Vec::new();
    for (di, doc) in docs.into_iter().enumerate() {
        let prefix = if multi {
            format!("#{} ", di + 1)
        } else {
            String::new()
        };
        walk_with_path(content(doc), Vec::new(), &mut |node, parts| {
            let is_leaf = node.kind == NodeKind::Scalar || node.children.is_empty();
            if is_leaf {
                let value =
                    serde_json::to_string(&crate::parser::JsonView::new(node)).unwrap_or_default();
                lines.push(format!("{prefix}{} = {value}", format_path(&parts)));
            }
        });
    }
    lines
}

// ---- edits ------------------------------------------------------------------

/// Parses a user-supplied value as YAML ("42" → int, "[a, b]" → list ...).
/// With `force_string` the text is always stored as a string.
pub fn parse_value(text: &str, force_string: bool) -> Result<YamNode> {
    if force_string || text.trim().is_empty() {
        return Ok(string_node(text));
    }
    let root =
        parse_yaml(text.as_bytes()).map_err(|err| anyhow!("invalid value {text:?}: {err:#}"))?;
    if root.kind == NodeKind::Stream {
        bail!("invalid value {text:?}: contains several documents");
    }
    Ok(content(&root).clone())
}

pub fn string_node(text: &str) -> YamNode {
    YamNode {
        kind: NodeKind::Scalar,
        key: None,
        value: Some(text.to_string()),
        tag: Some("!!str".into()),
        children: Vec::new(),
        depth: 0,
        index: 0,
    }
}

/// Re-types an edited scalar: string scalars stay strings; others are
/// resolved from the text the way YAML would ("true" → bool, "7" → int).
pub fn retag_scalar(node: &mut YamNode, text: &str) {
    node.value = Some(text.to_string());
    if node.tag.as_deref() == Some("!!str") {
        return;
    }
    node.tag = match parse_value(text, false) {
        Ok(parsed) if parsed.kind == NodeKind::Scalar && parsed.value.as_deref() == Some(text) => {
            parsed.tag
        }
        _ => Some("!!str".into()),
    };
}

/// Recomputes depth and index fields below `node` after a structural change.
pub fn renumber(node: &mut YamNode, depth: usize) {
    node.depth = depth;
    let child_depth = if node.kind == NodeKind::Document || node.kind == NodeKind::Stream {
        depth
    } else {
        depth + 1
    };
    for (i, child) in node.children.iter_mut().enumerate() {
        child.index = i;
        if node.kind == NodeKind::Sequence {
            child.key = None;
        }
        renumber(child, child_depth);
    }
}

fn empty_container_for(next: &Segment) -> YamNode {
    let kind = if matches!(next.kind, SegKind::Index(_)) {
        NodeKind::Sequence
    } else {
        NodeKind::Mapping
    };
    YamNode {
        kind,
        key: None,
        value: None,
        tag: None,
        children: Vec::new(),
        depth: 0,
        index: 0,
    }
}

fn is_null(node: &YamNode) -> bool {
    node.kind == NodeKind::Scalar && node.infer_type() == crate::parser::ScalarType::Null
}

/// Sets the value at `segs` in a document, creating missing mappings (and
/// appending when the index equals a list's length).
pub fn set_path(doc: &mut YamNode, segs: &[Segment], value: YamNode) -> Result<()> {
    if is_multi(segs) {
        bail!("wildcards and '..' are not supported when setting values");
    }
    if segs.is_empty() {
        bail!("cannot replace the document root; give a path such as .key");
    }
    {
        let root = if doc.kind == NodeKind::Document {
            if doc.children.is_empty() {
                doc.children.push(empty_container_for(&segs[0]));
            }
            &mut doc.children[0]
        } else {
            &mut *doc
        };
        if is_null(root) {
            *root = empty_container_for(&segs[0]);
        }
        let mut value = Some(value);
        set_in(root, segs, &mut value)?;
    }
    renumber(doc, 0);
    Ok(())
}

fn set_in(node: &mut YamNode, segs: &[Segment], value: &mut Option<YamNode>) -> Result<()> {
    let seg = &segs[0];
    let last = segs.len() == 1;
    let make_child = |value: &mut Option<YamNode>| {
        if last {
            value.take().expect("value set once")
        } else {
            empty_container_for(&segs[1])
        }
    };
    let child: &mut YamNode = match (node.kind, &seg.kind) {
        (NodeKind::Mapping, SegKind::Key(key)) => {
            match node
                .children
                .iter()
                .position(|c| c.key.as_deref() == Some(key))
            {
                Some(pos) => {
                    if last {
                        let mut v = value.take().expect("value set once");
                        v.key = Some(key.clone());
                        node.children[pos] = v;
                        return Ok(());
                    }
                    if is_null(&node.children[pos]) {
                        let mut c = empty_container_for(&segs[1]);
                        c.key = Some(key.clone());
                        node.children[pos] = c;
                    }
                    &mut node.children[pos]
                }
                None => {
                    let mut c = make_child(value);
                    c.key = Some(key.clone());
                    node.children.push(c);
                    if last {
                        return Ok(());
                    }
                    node.children.last_mut().expect("just pushed")
                }
            }
        }
        (NodeKind::Mapping, _) => bail!("cannot use index {seg} on a mapping"),
        (NodeKind::Sequence, SegKind::Index(idx)) => {
            let len = node.children.len();
            let pos = if *idx < 0 { len as isize + idx } else { *idx };
            if pos < 0 || pos as usize > len {
                bail!("array index out of bounds: {idx} (length: {len})");
            }
            let pos = pos as usize;
            if pos == len {
                node.children.push(make_child(value));
                if last {
                    return Ok(());
                }
                node.children.last_mut().expect("just pushed")
            } else {
                if last {
                    node.children[pos] = value.take().expect("value set once");
                    return Ok(());
                }
                if is_null(&node.children[pos]) {
                    node.children[pos] = empty_container_for(&segs[1]);
                }
                &mut node.children[pos]
            }
        }
        (NodeKind::Sequence, _) => bail!("cannot use key {seg} on a sequence"),
        _ => bail!("cannot traverse into scalar value at {seg}"),
    };
    set_in(child, &segs[1..], value)
}

/// Removes the node at `segs` from a document; reports whether it existed.
pub fn delete_path(doc: &mut YamNode, segs: &[Segment]) -> Result<bool> {
    if is_multi(segs) {
        bail!("wildcards and '..' are not supported when deleting values");
    }
    let Some((last, parents)) = segs.split_last() else {
        bail!("cannot delete the document root");
    };
    let mut node = if doc.kind == NodeKind::Document {
        match doc.children.first_mut() {
            Some(c) => c,
            None => return Ok(false),
        }
    } else {
        &mut *doc
    };
    for seg in parents {
        let pos = match (node.kind, &seg.kind) {
            (NodeKind::Mapping, SegKind::Key(key)) => node
                .children
                .iter()
                .position(|c| c.key.as_deref() == Some(key)),
            (NodeKind::Sequence, SegKind::Index(idx)) => resolve_index(*idx, node.children.len()),
            _ => None,
        };
        match pos {
            Some(pos) => node = &mut node.children[pos],
            None => return Ok(false),
        }
    }
    let pos = match (node.kind, &last.kind) {
        (NodeKind::Mapping, SegKind::Key(key)) => node
            .children
            .iter()
            .position(|c| c.key.as_deref() == Some(key)),
        (NodeKind::Sequence, SegKind::Index(idx)) => resolve_index(*idx, node.children.len()),
        _ => None,
    };
    let Some(pos) = pos else {
        return Ok(false);
    };
    node.children.remove(pos);
    renumber(doc, 0);
    Ok(true)
}

/// Applies `f` to the documents an edit targets: all of them, or the 1-based
/// document `doc` of a stream.
pub fn target_documents(root: &mut YamNode, doc: Option<usize>) -> Result<Vec<&mut YamNode>> {
    let docs = documents_mut(root);
    let total = docs.len();
    match doc {
        None => Ok(docs),
        Some(n) if n >= 1 && n <= total => Ok(docs.into_iter().skip(n - 1).take(1).collect()),
        Some(n) => bail!("document {n} does not exist (file has {total} document(s))"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{format_yaml, FormatOptions};
    use crate::parser::{parse_json, parse_yaml_str, to_raw_value};

    fn segs(path: &str) -> Vec<Segment> {
        parse_path(path).unwrap()
    }

    fn values(root: &YamNode, path: &str) -> Vec<String> {
        query(root, path)
            .unwrap()
            .into_iter()
            .map(|h| to_raw_value(h.node))
            .collect()
    }

    const K8S: &str = "kind: Deployment\nmetadata:\n  name: web\nspec:\n  containers:\n    - name: app\n      image: nginx\n    - name: side\n      image: envoy\n---\nkind: Service\nmetadata:\n  name: web\n";

    #[test]
    fn parses_path_syntax() {
        let key = Segment::key;
        let idx = Segment::index;
        let wild = Segment {
            kind: SegKind::Wildcard,
            recursive: false,
        };
        assert_eq!(segs("."), vec![]);
        assert_eq!(segs(".a.b"), vec![key("a"), key("b")]);
        assert_eq!(segs(".a[0]"), vec![key("a"), idx(0)]);
        assert_eq!(segs(".a[-1]"), vec![key("a"), idx(-1)]);
        assert_eq!(segs(".a[*].b"), vec![key("a"), wild.clone(), key("b")]);
        assert_eq!(segs(".a.*"), vec![key("a"), wild]);
        assert_eq!(segs(r#".["a.b"].c"#), vec![key("a.b"), key("c")]);
        assert_eq!(segs(".a['x y']"), vec![key("a"), key("x y")]);
        assert_eq!(
            segs("..image"),
            vec![Segment {
                kind: SegKind::Key("image".into()),
                recursive: true
            }]
        );
        assert_eq!(segs("[0]"), vec![idx(0)]);
        for bad in [
            "a.b", ".a[", ".a[x]", r#".["a]"#, "..", ".a.", ".a..", "...a",
        ] {
            assert!(parse_path(bad).is_err(), "{bad} should fail");
        }
    }

    #[test]
    fn queries_every_document() {
        let root = parse_yaml_str(K8S).unwrap();
        assert_eq!(root.kind, NodeKind::Stream);
        assert_eq!(values(&root, ".kind"), ["Deployment", "Service"]);
        assert_eq!(
            values(&root, ".spec.containers[*].image"),
            ["nginx", "envoy"]
        );
        assert_eq!(values(&root, "..image"), ["nginx", "envoy"]);
        assert_eq!(values(&root, ".spec.containers[-1].name"), ["side"]);
        assert_eq!(values(&root, ".metadata.*"), ["web", "web"]);
        assert!(values(&root, ".missing").is_empty());
        let hits = query(&root, "..image").unwrap();
        assert_eq!(format_path(&hits[1].path), ".spec.containers[1].image");
        assert_eq!(hits[1].doc, 0);
    }

    #[test]
    fn strict_lookup_errors() {
        let root = parse_yaml_str("a:\n  - 1\n").unwrap();
        for (path, want) in [
            (".b", "path not found"),
            (".a[5]", "out of bounds"),
            (".a.x", "expected array index"),
            (".a[0].", "trailing"),
        ] {
            let err = get_by_path(&root, path).unwrap_err().to_string();
            assert!(err.contains(want), "{path}: {err}");
        }
    }

    #[test]
    fn formatted_paths_resolve_back() {
        let root = parse_yaml_str("a:\n  \"b.c\":\n    - x\n  d e: 1\n").unwrap();
        for line in leaf_paths(&root) {
            let path = line.split(" = ").next().unwrap();
            assert!(get_by_path(&root, path).is_ok(), "{path}");
        }
        assert_eq!(
            leaf_paths(&root),
            [r#".a["b.c"][0] = "x""#, r#".a["d e"] = 1"#]
        );
    }

    #[test]
    fn merge_keys_are_applied() {
        let root = parse_yaml_str("base: &b\n  x: 1\n  y: 2\nd:\n  <<: *b\n  y: 3\n").unwrap();
        assert_eq!(
            to_raw_value(get_by_path(&root, ".d").unwrap()),
            r#"{"y":3,"x":1}"#
        );
    }

    #[test]
    fn json_keeps_order_and_reads_streams() {
        let root = parse_json(br#"{"z":1,"a":{"b":true}}"#).unwrap();
        assert_eq!(to_raw_value(&root), r#"{"z":1,"a":{"b":true}}"#);
        let stream = parse_json(b"{\"a\":1}\n{\"a\":2}\n").unwrap();
        assert_eq!(values(&stream, ".a"), ["1", "2"]);
        assert!(parse_json(b"{\"a\":").is_err());
    }

    fn set(src: &str, path: &str, value: &str, string: bool) -> String {
        let mut root = parse_yaml_str(src).unwrap();
        set_path(&mut root, &segs(path), parse_value(value, string).unwrap()).unwrap();
        format_yaml(&root, FormatOptions::default()).unwrap()
    }

    #[test]
    fn sets_values() {
        assert_eq!(set("a: 1\n", ".a", "2", false), "a: 2\n");
        assert_eq!(set("a: 1\n", ".b.c", "x", false), "a: 1\nb:\n  c: x\n");
        assert_eq!(set("l:\n- a\n", ".l[1]", "b", false), "l:\n- a\n- b\n");
        assert_eq!(
            set("l:\n- a\n- b\n", ".l[-1]", "z", false),
            "l:\n- a\n- z\n"
        );
        assert_eq!(set("a: 1\n", ".a", "1.10", true), "a: '1.10'\n");
        assert_eq!(
            set("a: 1\n", ".a", "{k: [1, 2]}", false),
            "a:\n  k:\n  - 1\n  - 2\n"
        );
        assert_eq!(set("a: ~\n", ".a.b", "1", false), "a:\n  b: 1\n");
        assert_eq!(set("a: 1\n", ".x[0].y", "v", false), "a: 1\nx:\n- y: v\n");

        let mut root = parse_yaml_str("a: 1\nl: [1]\n").unwrap();
        for path in [".a.b", ".l[5]", ".l.x", ".l[*]"] {
            assert!(
                set_path(&mut root, &segs(path), string_node("v")).is_err(),
                "{path}"
            );
        }
    }

    #[test]
    fn deletes_values() {
        let mut root = parse_yaml_str("a: 1\nb:\n  c: 2\n  d: 3\nl:\n- x\n- y\n").unwrap();
        assert!(delete_path(&mut root, &segs(".b.c")).unwrap());
        assert!(delete_path(&mut root, &segs(".l[-1]")).unwrap());
        assert!(!delete_path(&mut root, &segs(".nope")).unwrap());
        assert_eq!(
            format_yaml(&root, FormatOptions::default()).unwrap(),
            "a: 1\nb:\n  d: 3\nl:\n- x\n"
        );
    }

    #[test]
    fn retags_edited_scalars() {
        let root = parse_yaml_str("s: x\nn: 1\n").unwrap();
        let mut s = get_by_path(&root, ".s").unwrap().clone();
        retag_scalar(&mut s, "42");
        assert_eq!(s.tag.as_deref(), Some("!!str"), "strings stay strings");
        let mut n = get_by_path(&root, ".n").unwrap().clone();
        retag_scalar(&mut n, "true");
        assert_eq!(n.tag.as_deref(), Some("!!bool"));
        retag_scalar(&mut n, "hello world");
        assert_eq!(n.tag.as_deref(), Some("!!str"));
    }
}
