use crate::parser::{
    content, documents, resource_id, to_json_value, NodeKind, ScalarType, YamNode,
};
use crate::path::{get_by_path, key_segment};
use anyhow::Result;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::{self, Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffType {
    Unchanged,
    Added,
    Removed,
    Modified,
}

impl DiffType {
    pub fn name(self) -> &'static str {
        match self {
            DiffType::Unchanged => "unchanged",
            DiffType::Added => "added",
            DiffType::Removed => "removed",
            DiffType::Modified => "modified",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiffNode<'a> {
    pub left: Option<&'a YamNode>,
    pub right: Option<&'a YamNode>,
    pub diff_type: DiffType,
    pub children: Vec<DiffNode<'a>>,
    /// Location in path syntax (".a.b[0]"); empty for a document root.
    pub path: String,
    /// Set on the document nodes of a multi-document diff.
    pub label: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffSummary {
    pub added: usize,
    pub removed: usize,
    pub modified: usize,
    pub total: usize,
}

#[derive(Debug, Clone)]
pub struct DiffResult<'a> {
    pub root: Option<DiffNode<'a>>,
    pub summary: DiffSummary,
    pub left_file: String,
    pub right_file: String,
}

#[derive(Debug, Clone, Default)]
pub struct DiffOptions {
    /// Only compare the subtree at this path (in every document).
    pub path: Option<String>,
}

pub fn compare<'a>(left: &'a YamNode, right: &'a YamNode) -> DiffResult<'a> {
    compare_with(left, right, &DiffOptions::default()).expect("no path to validate")
}

/// Compares two trees. Multi-document inputs are compared document by
/// document: Kubernetes-style resources are paired by Kind/namespace/name,
/// anything else by position.
pub fn compare_with<'a>(
    left: &'a YamNode,
    right: &'a YamNode,
    opts: &DiffOptions,
) -> Result<DiffResult<'a>> {
    if let Some(path) = &opts.path {
        let segs = crate::path::parse_path(path)?;
        if crate::path::is_multi(&segs) {
            anyhow::bail!("wildcards and '..' are not supported in diff paths");
        }
    }
    let select = |doc: &'a YamNode| -> Option<&'a YamNode> {
        match &opts.path {
            Some(path) => get_by_path(doc, path).ok(),
            None => Some(content(doc)),
        }
    };
    let base = opts
        .path
        .as_deref()
        .filter(|p| *p != ".")
        .unwrap_or("")
        .to_string();

    let root = if left.kind == NodeKind::Stream || right.kind == NodeKind::Stream {
        Some(compare_documents(
            documents(left),
            documents(right),
            &select,
            &base,
        ))
    } else {
        compare_nodes(select(left), select(right), base)
    };
    let mut summary = DiffSummary::default();
    if let Some(root) = &root {
        walk_summary(root, &mut summary);
    }
    summary.total = summary.added + summary.removed + summary.modified;
    Ok(DiffResult {
        root,
        summary,
        left_file: String::new(),
        right_file: String::new(),
    })
}

type Pair<'a> = (Option<&'a YamNode>, Option<&'a YamNode>);

fn compare_documents<'a>(
    left: Vec<&'a YamNode>,
    right: Vec<&'a YamNode>,
    select: &dyn Fn(&'a YamNode) -> Option<&'a YamNode>,
    base: &str,
) -> DiffNode<'a> {
    let mut pairs: Vec<Pair<'a>> = Vec::new();
    if all_have_ids(&left) && all_have_ids(&right) {
        let right_by_id: HashMap<String, &YamNode> = right
            .iter()
            .map(|d| (resource_id(d).unwrap_or_default(), *d))
            .collect();
        let mut used = HashSet::new();
        for d in &left {
            let id = resource_id(d).unwrap_or_default();
            let r = right_by_id.get(&id).copied();
            if r.is_some() {
                used.insert(id);
            }
            pairs.push((Some(*d), r));
        }
        for d in &right {
            if !used.contains(&resource_id(d).unwrap_or_default()) {
                pairs.push((None, Some(*d)));
            }
        }
    } else {
        for i in 0..left.len().max(right.len()) {
            pairs.push((left.get(i).copied(), right.get(i).copied()));
        }
    }

    let mut root = DiffNode {
        left: None,
        right: None,
        diff_type: DiffType::Unchanged,
        children: Vec::new(),
        path: String::new(),
        label: None,
    };
    for (l, r) in pairs {
        let reference = r.or(l).expect("pair has a side");
        let label =
            resource_id(reference).unwrap_or_else(|| format!("document {}", reference.index + 1));
        let Some(inner) = compare_nodes(l.and_then(select), r.and_then(select), base.to_string())
        else {
            continue;
        };
        let diff_type = match (l, r) {
            (None, _) => DiffType::Added,
            (_, None) => DiffType::Removed,
            _ => inner.diff_type,
        };
        if diff_type != DiffType::Unchanged {
            root.diff_type = DiffType::Modified;
        }
        root.children.push(DiffNode {
            left: l,
            right: r,
            diff_type,
            children: vec![inner],
            path: String::new(),
            label: Some(label),
        });
    }
    root
}

fn all_have_ids(docs: &[&YamNode]) -> bool {
    let mut seen = HashSet::new();
    docs.iter()
        .all(|d| resource_id(d).is_some_and(|id| seen.insert(id)))
}

fn compare_nodes<'a>(
    left: Option<&'a YamNode>,
    right: Option<&'a YamNode>,
    path: String,
) -> Option<DiffNode<'a>> {
    let leaf = |diff_type| DiffNode {
        left,
        right,
        diff_type,
        children: Vec::new(),
        path: path.clone(),
        label: None,
    };
    match (left, right) {
        (None, None) => None,
        (None, Some(_)) => Some(leaf(DiffType::Added)),
        (Some(_), None) => Some(leaf(DiffType::Removed)),
        (Some(l), Some(r)) => Some(compare_present_nodes(l, r, path)),
    }
}

fn container<'a>(
    left: &'a YamNode,
    right: &'a YamNode,
    path: String,
    children: Vec<DiffNode<'a>>,
) -> DiffNode<'a> {
    let changed = children.iter().any(|c| c.diff_type != DiffType::Unchanged);
    DiffNode {
        left: Some(left),
        right: Some(right),
        diff_type: if changed {
            DiffType::Modified
        } else {
            DiffType::Unchanged
        },
        children,
        path,
        label: None,
    }
}

fn compare_present_nodes<'a>(left: &'a YamNode, right: &'a YamNode, path: String) -> DiffNode<'a> {
    if left.kind == NodeKind::Document || right.kind == NodeKind::Document {
        return compare_nodes(Some(content(left)), Some(content(right)), path)
            .expect("both sides exist");
    }

    if left.kind == NodeKind::Mapping && right.kind == NodeKind::Mapping {
        let by_key = |node: &'a YamNode| -> HashMap<&'a str, &'a YamNode> {
            node.children
                .iter()
                .filter_map(|c| c.key.as_deref().map(|k| (k, c)))
                .collect()
        };
        let (lk, rk) = (by_key(left), by_key(right));
        let keys: BTreeSet<&str> = lk.keys().chain(rk.keys()).copied().collect();
        let children = keys
            .into_iter()
            .filter_map(|key| {
                compare_nodes(
                    lk.get(key).copied(),
                    rk.get(key).copied(),
                    format!("{path}{}", key_segment(key)),
                )
            })
            .collect();
        return container(left, right, path, children);
    }

    if left.kind == NodeKind::Sequence && right.kind == NodeKind::Sequence {
        let children = match_sequence(&left.children, &right.children)
            .into_iter()
            .filter_map(|(l, r)| {
                let idx = r.or(l).map(|n| n.index).unwrap_or_default();
                compare_nodes(l, r, format!("{path}[{idx}]"))
            })
            .collect();
        return container(left, right, path, children);
    }

    let modified = if left.kind == NodeKind::Scalar && right.kind == NodeKind::Scalar {
        left.value != right.value || left.infer_type() != right.infer_type()
    } else {
        left.kind != right.kind
    };
    DiffNode {
        left: Some(left),
        right: Some(right),
        diff_type: if modified {
            DiffType::Modified
        } else {
            DiffType::Unchanged
        },
        children: Vec::new(),
        path,
        label: None,
    }
}

/// Fields used to pair list items that are mappings (Kubernetes containers,
/// env vars and ports all have "name").
const IDENTITY_FIELDS: [&str; 3] = ["name", "id", "key"];

/// Aligns two sequences: by a unique identity field when every item has
/// one, otherwise with a longest common subsequence so that one insertion
/// does not mark every following item as changed.
fn match_sequence<'a>(left: &'a [YamNode], right: &'a [YamNode]) -> Vec<Pair<'a>> {
    // Fast path: lists that line up item by item need no matching.
    if left.len() == right.len() && left.iter().zip(right).all(|(l, r)| same_content(l, r)) {
        return match_by_position(left, right);
    }
    if let Some(field) = common_identity_field(left, right) {
        return match_by_field(left, right, field);
    }
    match_by_lcs(left, right)
}

/// Structural equality ignoring position fields (depth, index).
fn same_content(left: &YamNode, right: &YamNode) -> bool {
    left.kind == right.kind
        && left.key == right.key
        && left.value == right.value
        && left.infer_type() == right.infer_type()
        && left.children.len() == right.children.len()
        && left
            .children
            .iter()
            .zip(&right.children)
            .all(|(l, r)| same_content(l, r))
}

fn identity(node: &YamNode, field: &str) -> Option<String> {
    if node.kind != NodeKind::Mapping {
        return None;
    }
    node.children
        .iter()
        .find(|c| c.key.as_deref() == Some(field))
        .filter(|c| c.kind == NodeKind::Scalar)
        .and_then(|c| c.value.clone())
        .filter(|v| !v.is_empty())
}

fn common_identity_field(left: &[YamNode], right: &[YamNode]) -> Option<&'static str> {
    if left.is_empty() || right.is_empty() {
        return None;
    }
    IDENTITY_FIELDS.into_iter().find(|field| {
        [left, right].iter().all(|list| {
            let mut seen = HashSet::new();
            list.iter()
                .all(|item| identity(item, field).is_some_and(|v| seen.insert(v)))
        })
    })
}

fn match_by_field<'a>(left: &'a [YamNode], right: &'a [YamNode], field: &str) -> Vec<Pair<'a>> {
    let left_by: HashMap<String, &YamNode> = left
        .iter()
        .filter_map(|l| identity(l, field).map(|id| (id, l)))
        .collect();
    let right_ids: HashSet<String> = right.iter().filter_map(|r| identity(r, field)).collect();
    let mut out: Vec<Pair> = right
        .iter()
        .map(|r| {
            (
                identity(r, field).and_then(|id| left_by.get(&id).copied()),
                Some(r),
            )
        })
        .collect();
    out.extend(
        left.iter()
            .filter(|l| identity(l, field).is_none_or(|id| !right_ids.contains(&id)))
            .map(|l| (Some(l), None)),
    );
    out
}

const MAX_LCS_CELLS: usize = 4_000_000;

fn match_by_lcs<'a>(left: &'a [YamNode], right: &'a [YamNode]) -> Vec<Pair<'a>> {
    let (n, m) = (left.len(), right.len());
    if n == 0 || m == 0 || n.saturating_mul(m) > MAX_LCS_CELLS {
        return match_by_position(left, right);
    }
    let fingerprint =
        |node: &YamNode| serde_json::to_string(&to_json_value(node)).unwrap_or_default();
    let lh: Vec<String> = left.iter().map(fingerprint).collect();
    let rh: Vec<String> = right.iter().map(fingerprint).collect();
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if lh[i] == rh[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    fn flush<'a>(out: &mut Vec<Pair<'a>>, gl: &mut Vec<&'a YamNode>, gr: &mut Vec<&'a YamNode>) {
        for i in 0..gl.len().max(gr.len()) {
            out.push((gl.get(i).copied(), gr.get(i).copied()));
        }
        gl.clear();
        gr.clear();
    }

    let mut out = Vec::new();
    let (mut gap_l, mut gap_r): (Vec<&YamNode>, Vec<&YamNode>) = (Vec::new(), Vec::new());
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if lh[i] == rh[j] {
            flush(&mut out, &mut gap_l, &mut gap_r);
            out.push((Some(&left[i]), Some(&right[j])));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            gap_l.push(&left[i]);
            i += 1;
        } else {
            gap_r.push(&right[j]);
            j += 1;
        }
    }
    gap_l.extend(&left[i..]);
    gap_r.extend(&right[j..]);
    flush(&mut out, &mut gap_l, &mut gap_r);
    out
}

fn match_by_position<'a>(left: &'a [YamNode], right: &'a [YamNode]) -> Vec<Pair<'a>> {
    (0..left.len().max(right.len()))
        .map(|i| (left.get(i), right.get(i)))
        .collect()
}

/// Counts real changes only: additions, removals and changed values.
/// Containers that merely contain changes are not counted.
fn walk_summary(node: &DiffNode<'_>, summary: &mut DiffSummary) {
    match node.diff_type {
        DiffType::Added => summary.added += 1,
        DiffType::Removed => summary.removed += 1,
        DiffType::Modified if node.children.is_empty() => summary.modified += 1,
        _ => {
            for child in &node.children {
                walk_summary(child, summary);
            }
        }
    }
}

/// One entry of a machine-readable diff.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub change_type: DiffType,
    pub path: String,
    pub document: Option<String>,
    pub old: Option<serde_json::Value>,
    pub new: Option<serde_json::Value>,
}

/// The individual changes of a result, in display order.
pub fn changes(result: &DiffResult<'_>) -> Vec<Change> {
    fn walk(node: &DiffNode<'_>, doc: &Option<String>, out: &mut Vec<Change>) {
        let doc = node.label.clone().or_else(|| doc.clone());
        let is_change = match node.diff_type {
            DiffType::Added | DiffType::Removed => true,
            DiffType::Modified => node.children.is_empty(),
            DiffType::Unchanged => false,
        };
        if is_change {
            let path = if node.path.is_empty() {
                ".".to_string()
            } else {
                node.path.clone()
            };
            out.push(Change {
                change_type: node.diff_type,
                path,
                document: doc,
                old: node.left.map(|n| to_json_value(content(n))),
                new: node.right.map(|n| to_json_value(content(n))),
            });
            return;
        }
        for child in &node.children {
            walk(child, &doc, out);
        }
    }
    let mut out = Vec::new();
    if let Some(root) = &result.root {
        walk(root, &None, &mut out);
    }
    out
}

/// The result as JSON: {"left","right","summary":{...},"changes":[...]}.
pub fn render_json(result: &DiffResult<'_>) -> serde_json::Value {
    let changes: Vec<serde_json::Value> = changes(result)
        .into_iter()
        .map(|c| {
            let mut obj = serde_json::Map::new();
            obj.insert("type".into(), c.change_type.name().into());
            obj.insert("path".into(), c.path.into());
            if let Some(doc) = c.document {
                obj.insert("document".into(), doc.into());
            }
            if let Some(old) = c.old {
                obj.insert("old".into(), old);
            }
            if let Some(new) = c.new {
                obj.insert("new".into(), new);
            }
            serde_json::Value::Object(obj)
        })
        .collect();
    serde_json::json!({
        "left": result.left_file,
        "right": result.right_file,
        "summary": {
            "added": result.summary.added,
            "removed": result.summary.removed,
            "modified": result.summary.modified,
            "total": result.summary.total,
        },
        "changes": changes,
    })
}

pub fn render(output: &mut impl Write, result: &DiffResult<'_>) -> io::Result<()> {
    if !result.left_file.is_empty() || !result.right_file.is_empty() {
        writeln!(output, "--- {}", result.left_file)?;
        writeln!(output, "+++ {}\n", result.right_file)?;
    }

    if let Some(root) = &result.root {
        render_diff_node(output, root, "")?;
    }

    if result.summary.total > 0 {
        writeln!(output, "\n{}", render_summary(&result.summary))?;
    }

    Ok(())
}

pub fn render_summary(summary: &DiffSummary) -> String {
    if summary.total == 0 {
        "Summary: no changes".to_string()
    } else {
        format!(
            "Summary: {} added, {} removed, {} modified",
            summary.added, summary.removed, summary.modified
        )
    }
}

fn prefix_for(diff_type: DiffType) -> &'static str {
    match diff_type {
        DiffType::Added => "+ ",
        DiffType::Removed => "- ",
        DiffType::Modified => "~ ",
        DiffType::Unchanged => "  ",
    }
}

fn render_diff_node(output: &mut impl Write, node: &DiffNode<'_>, indent: &str) -> io::Result<()> {
    if node.diff_type == DiffType::Unchanged && !has_changes(node) {
        return Ok(());
    }
    let prefix = prefix_for(node.diff_type);

    if let Some(label) = &node.label {
        let suffix = match node.diff_type {
            DiffType::Added => " (added)",
            DiffType::Removed => " (removed)",
            _ => "",
        };
        writeln!(output, "{prefix}{indent}=== {label}{suffix}")?;
        let child_indent = format!("{indent}  ");
        if matches!(node.diff_type, DiffType::Added | DiffType::Removed) {
            if let Some(doc) = node.right.or(node.left) {
                render_subtree(output, content(doc), prefix, &child_indent)?;
            }
            return Ok(());
        }
        for child in &node.children {
            render_diff_node(output, child, &child_indent)?;
        }
        return Ok(());
    }

    // The root of a document (or of the compared subtree) has no own line.
    let key = node_key(node);
    if (key.is_none() || node.path.is_empty()) && node.children.iter().len() > 0 {
        for child in &node.children {
            render_diff_node(output, child, indent)?;
        }
        return Ok(());
    }
    let key = key.unwrap_or_else(|| "(root)".into());

    if node.diff_type == DiffType::Modified && node.children.is_empty() {
        writeln!(
            output,
            "{prefix}{indent}{key}: {} -> {}",
            display_value(node.left),
            display_value(node.right)
        )?;
    } else if is_container_node(node)
        && matches!(node.diff_type, DiffType::Added | DiffType::Removed)
    {
        let side = node.right.or(node.left).expect("one side exists");
        if side.children.is_empty() {
            writeln!(
                output,
                "{prefix}{indent}{key}: {}",
                display_value(Some(side))
            )?;
        } else {
            writeln!(output, "{prefix}{indent}{key}:")?;
            render_subtree(output, side, prefix, &format!("{indent}  "))?;
        }
    } else if is_container_node(node) {
        writeln!(output, "{prefix}{indent}{key}:")?;
        let child_indent = format!("{indent}  ");
        for child in &node.children {
            render_diff_node(output, child, &child_indent)?;
        }
    } else {
        writeln!(
            output,
            "{prefix}{indent}{key}: {}",
            display_value(node.right.or(node.left))
        )?;
    }
    Ok(())
}

/// Prints every entry of an added or removed subtree.
fn render_subtree(
    output: &mut impl Write,
    node: &YamNode,
    prefix: &str,
    indent: &str,
) -> io::Result<()> {
    if node.kind == NodeKind::Scalar {
        return writeln!(output, "{prefix}{indent}{}", display_value(Some(node)));
    }
    for child in &node.children {
        let label = match (&child.key, node.kind) {
            (Some(k), NodeKind::Mapping) => k.clone(),
            _ => format!("[{}]", child.index),
        };
        if child.kind != NodeKind::Scalar && !child.children.is_empty() {
            writeln!(output, "{prefix}{indent}{label}:")?;
            render_subtree(output, child, prefix, &format!("{indent}  "))?;
        } else {
            writeln!(
                output,
                "{prefix}{indent}{label}: {}",
                display_value(Some(child))
            )?;
        }
    }
    Ok(())
}

fn has_changes(node: &DiffNode<'_>) -> bool {
    node.diff_type != DiffType::Unchanged || node.children.iter().any(has_changes)
}

fn is_container_node(node: &DiffNode<'_>) -> bool {
    matches!(
        node.right.or(node.left).map(|node| node.kind),
        Some(NodeKind::Mapping | NodeKind::Sequence)
    )
}

/// The key or "[index]" of a node, from the new side when it exists.
fn node_key(node: &DiffNode<'_>) -> Option<String> {
    let side = node.right.or(node.left)?;
    if let Some(key) = &side.key {
        return Some(key.clone());
    }
    (side.depth > 0).then(|| format!("[{}]", side.index))
}

/// Shows a value so that type differences are visible: strings that would
/// read as another type are quoted ("1" vs 1).
pub fn display_value(node: Option<&YamNode>) -> String {
    let Some(node) = node else {
        return String::new();
    };
    match node.kind {
        NodeKind::Mapping if node.children.is_empty() => "{}".into(),
        NodeKind::Mapping => "{...}".into(),
        NodeKind::Sequence if node.children.is_empty() => "[]".into(),
        NodeKind::Sequence => format!("[{} items]", node.children.len()),
        NodeKind::Document | NodeKind::Stream => String::new(),
        NodeKind::Scalar => {
            let value = node.value.clone().unwrap_or_default();
            match node.infer_type() {
                ScalarType::Null if value.is_empty() => "null".into(),
                ScalarType::String => {
                    let probe = YamNode {
                        tag: None,
                        ..node.clone()
                    };
                    if value.is_empty()
                        || probe.infer_type() != ScalarType::String
                        || value.trim() != value
                    {
                        format!("{value:?}")
                    } else {
                        value
                    }
                }
                _ => value,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_yaml_str;

    fn diff_changes(left: &str, right: &str) -> Vec<String> {
        let (l, r) = (
            parse_yaml_str(left).unwrap(),
            parse_yaml_str(right).unwrap(),
        );
        let result = compare(&l, &r);
        changes(&result)
            .into_iter()
            .map(|c| match c.document {
                Some(doc) => format!("{doc} {} {}", c.change_type.name(), c.path),
                None => format!("{} {}", c.change_type.name(), c.path),
            })
            .collect()
    }

    fn summary(left: &str, right: &str) -> DiffSummary {
        let (l, r) = (
            parse_yaml_str(left).unwrap(),
            parse_yaml_str(right).unwrap(),
        );
        compare(&l, &r).summary
    }

    fn rendered(left: &str, right: &str) -> String {
        let (l, r) = (
            parse_yaml_str(left).unwrap(),
            parse_yaml_str(right).unwrap(),
        );
        let mut out = Vec::new();
        render(&mut out, &compare(&l, &r)).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn counts_only_real_changes() {
        let s = summary(
            "app:\n  name: yam\n",
            "app:\n  name: rust-yam\n  version: 1\n",
        );
        assert_eq!((s.added, s.removed, s.modified, s.total), (1, 0, 1, 2));
        let s = summary("a:\n  b:\n    c: 1\n", "a:\n  b:\n    c: 2\n");
        assert_eq!((s.modified, s.total), (1, 1));
    }

    #[test]
    fn sequence_insertion_is_aligned() {
        assert_eq!(
            diff_changes("l: [a, b, c]\n", "l: [x, a, b, c]\n"),
            ["added .l[0]"]
        );
        assert_eq!(
            diff_changes("l: [a, b, c, d]\n", "l: [a, c, D]\n"),
            ["removed .l[1]", "modified .l[2]"]
        );
    }

    #[test]
    fn list_items_are_matched_by_name() {
        let left = "c:\n  - name: app\n    image: v1\n  - name: side\n    image: s1\n";
        let right =
            "c:\n  - name: side\n    image: s1\n  - name: app\n    image: v2\n  - name: new\n";
        assert_eq!(
            diff_changes(left, right),
            ["modified .c[1].image", "added .c[2]"]
        );
    }

    #[test]
    fn documents_are_matched_by_resource_id() {
        let left =
            "kind: A\nmetadata:\n  name: x\nv: 1\n---\nkind: B\nmetadata:\n  name: y\nv: 1\n";
        let right = "kind: B\nmetadata:\n  name: y\nv: 2\n---\nkind: C\nmetadata:\n  name: z\n";
        assert_eq!(
            diff_changes(left, right),
            ["A/x removed .", "B/y modified .v", "C/z added ."]
        );
        let out = rendered(left, right);
        for want in [
            "=== A/x (removed)",
            "=== B/y",
            "=== C/z (added)",
            "v: 1 -> 2",
        ] {
            assert!(out.contains(want), "missing {want:?} in\n{out}");
        }
        assert_eq!(
            diff_changes("a: 1\n---\nb: 1\n", "a: 1\n---\nb: 2\n"),
            ["document 2 modified .b"]
        );
    }

    #[test]
    fn root_is_not_labelled_as_index() {
        let out = rendered("l: [a]\n", "l: [b]\n");
        assert!(out.contains("~ l:\n~   [0]: a -> b"), "{out}");
    }

    #[test]
    fn type_changes_are_visible() {
        assert_eq!(diff_changes("n: 1\n", "n: \"1\"\n"), ["modified .n"]);
        assert!(rendered("n: 1\n", "n: \"1\"\n").contains(r#"n: 1 -> "1""#));
    }

    #[test]
    fn added_subtree_is_shown_in_full() {
        let out = rendered("a: 1\n", "a: 1\nb:\n  c: 1\n  d: [x]\n");
        for want in ["+ b:", "+   c: 1", "+   d:", "+     [0]: x"] {
            assert!(out.contains(want), "missing {want:?} in\n{out}");
        }
    }

    #[test]
    fn compares_a_subtree() {
        let (l, r) = (
            parse_yaml_str("spec:\n  a: 1\nother: 1\n").unwrap(),
            parse_yaml_str("spec:\n  a: 2\nother: 2\n").unwrap(),
        );
        let opts = DiffOptions {
            path: Some(".spec".into()),
        };
        let result = compare_with(&l, &r, &opts).unwrap();
        let paths: Vec<String> = changes(&result).into_iter().map(|c| c.path).collect();
        assert_eq!(paths, [".spec.a"]);
        let bad = DiffOptions {
            path: Some(".spec[*]".into()),
        };
        assert!(compare_with(&l, &r, &bad).is_err());
    }

    #[test]
    fn json_output_lists_changes() {
        let (l, r) = (
            parse_yaml_str("a: 1\nb: x\n").unwrap(),
            parse_yaml_str("a: 2\nc: [1]\n").unwrap(),
        );
        let json = render_json(&compare(&l, &r));
        assert_eq!(json["summary"]["total"], 3);
        assert_eq!(json["changes"][0]["path"], ".a");
        assert_eq!(json["changes"][0]["old"], 1);
        assert_eq!(json["changes"][0]["new"], 2);
        assert_eq!(json["changes"][2]["new"], serde_json::json!([1]));
    }
}
