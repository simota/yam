use crate::parser::{children_sorted_by_key, NodeKind, YamNode};
use std::cmp::Ordering;
use std::fmt;
use std::io::{self, Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffType {
    Unchanged,
    Added,
    Removed,
    Modified,
}

#[derive(Debug, Clone)]
pub struct DiffNode<'a> {
    pub left: Option<&'a YamNode>,
    pub right: Option<&'a YamNode>,
    pub diff_type: DiffType,
    pub children: Vec<DiffNode<'a>>,
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

pub fn compare<'a>(left: &'a YamNode, right: &'a YamNode) -> DiffResult<'a> {
    let root = compare_nodes(Some(left), Some(right));
    let mut summary = DiffSummary::default();
    if let Some(root) = &root {
        walk_summary(root, &mut summary);
    }
    summary.total = summary.added + summary.removed + summary.modified;

    DiffResult {
        root,
        summary,
        left_file: String::new(),
        right_file: String::new(),
    }
}

pub fn render(output: &mut impl Write, result: &DiffResult<'_>) -> io::Result<()> {
    if !result.left_file.is_empty() || !result.right_file.is_empty() {
        writeln!(output, "--- {}", result.left_file)?;
        writeln!(output, "+++ {}\n", result.right_file)?;
    }

    if let Some(root) = &result.root {
        render_diff_node(output, root, &mut String::new())?;
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

fn compare_nodes<'a>(
    left: Option<&'a YamNode>,
    right: Option<&'a YamNode>,
) -> Option<DiffNode<'a>> {
    match (left, right) {
        (None, None) => None,
        (None, Some(right)) => Some(DiffNode {
            left: None,
            right: Some(right),
            diff_type: DiffType::Added,
            children: Vec::new(),
        }),
        (Some(left), None) => Some(DiffNode {
            left: Some(left),
            right: None,
            diff_type: DiffType::Removed,
            children: Vec::new(),
        }),
        (Some(left), Some(right)) => compare_present_nodes(left, right),
    }
}

fn compare_present_nodes<'a>(left: &'a YamNode, right: &'a YamNode) -> Option<DiffNode<'a>> {
    if left.kind == NodeKind::Document && right.kind == NodeKind::Document {
        return compare_nodes(left.children.first(), right.children.first());
    }

    if left.kind == NodeKind::Mapping && right.kind == NodeKind::Mapping {
        let left_by_key = children_by_key(left);
        let right_by_key = children_by_key(right);
        let mut children = Vec::with_capacity(left_by_key.len().max(right_by_key.len()));
        let mut has_changes = false;

        // Walk the union of the keys in order, pairing equal keys.
        let (mut left_iter, mut right_iter) = (
            left_by_key.into_iter().peekable(),
            right_by_key.into_iter().peekable(),
        );
        loop {
            let (left_child, right_child) = match (left_iter.peek(), right_iter.peek()) {
                (None, None) => break,
                (Some(_), None) => (left_iter.next(), None),
                (None, Some(_)) => (None, right_iter.next()),
                (Some(left_child), Some(right_child)) => match left_child.key.cmp(&right_child.key)
                {
                    Ordering::Less => (left_iter.next(), None),
                    Ordering::Greater => (None, right_iter.next()),
                    Ordering::Equal => (left_iter.next(), right_iter.next()),
                },
            };
            if let Some(child_diff) = compare_nodes(left_child, right_child) {
                has_changes |= child_diff.diff_type != DiffType::Unchanged;
                children.push(child_diff);
            }
        }

        return Some(DiffNode {
            left: Some(left),
            right: Some(right),
            diff_type: if has_changes {
                DiffType::Modified
            } else {
                DiffType::Unchanged
            },
            children,
        });
    }

    if left.kind == NodeKind::Sequence && right.kind == NodeKind::Sequence {
        let max_len = left.children.len().max(right.children.len());
        let mut children = Vec::with_capacity(max_len);
        let mut has_changes = false;

        for idx in 0..max_len {
            if let Some(child_diff) = compare_nodes(left.children.get(idx), right.children.get(idx))
            {
                has_changes |= child_diff.diff_type != DiffType::Unchanged;
                children.push(child_diff);
            }
        }

        return Some(DiffNode {
            left: Some(left),
            right: Some(right),
            diff_type: if has_changes {
                DiffType::Modified
            } else {
                DiffType::Unchanged
            },
            children,
        });
    }

    if left.kind == NodeKind::Scalar && right.kind == NodeKind::Scalar {
        return Some(DiffNode {
            left: Some(left),
            right: Some(right),
            diff_type: if left.value == right.value {
                DiffType::Unchanged
            } else {
                DiffType::Modified
            },
            children: Vec::new(),
        });
    }

    Some(DiffNode {
        left: Some(left),
        right: Some(right),
        diff_type: if left.kind == right.kind {
            DiffType::Unchanged
        } else {
            DiffType::Modified
        },
        children: Vec::new(),
    })
}

/// A mapping's keyed children sorted by key; of children sharing a key, the
/// last one stands for the key.
fn children_by_key(node: &YamNode) -> Vec<&YamNode> {
    // Stable, so children sharing a key stay in document order.
    let mut children = children_sorted_by_key(node);
    children.retain(|child| child.key.is_some());
    children.dedup_by(|later, earlier| {
        let same_key = later.key == earlier.key;
        if same_key {
            *earlier = *later;
        }
        same_key
    });
    children
}

fn walk_summary(node: &DiffNode<'_>, summary: &mut DiffSummary) {
    match node.diff_type {
        DiffType::Added => summary.added += 1,
        DiffType::Removed => summary.removed += 1,
        DiffType::Modified => summary.modified += 1,
        DiffType::Unchanged => {}
    }
    for child in &node.children {
        walk_summary(child, summary);
    }
}

/// `indent` is shared by the whole walk and restored before returning.
fn render_diff_node(
    output: &mut impl Write,
    node: &DiffNode<'_>,
    indent: &mut String,
) -> io::Result<()> {
    if node.diff_type == DiffType::Unchanged && !has_changes(node) {
        return Ok(());
    }
    if is_document_node(node) {
        for child in &node.children {
            render_diff_node(output, child, indent)?;
        }
        return Ok(());
    }

    let key = node_key(node);
    if key.is_empty() && is_container_node(node) {
        for child in &node.children {
            render_diff_node(output, child, indent)?;
        }
        return Ok(());
    }

    let prefix = match node.diff_type {
        DiffType::Added => "+ ",
        DiffType::Removed => "- ",
        DiffType::Modified => "~ ",
        DiffType::Unchanged => "  ",
    };

    if node.diff_type == DiffType::Modified && is_scalar_node(node) {
        writeln!(
            output,
            "{prefix}{indent}{key}: {} -> {}",
            scalar_value(node.left),
            scalar_value(node.right)
        )?;
    } else if is_container_node(node) {
        writeln!(output, "{prefix}{indent}{key}:")?;
        let len = indent.len();
        indent.push_str("  ");
        for child in &node.children {
            render_diff_node(output, child, indent)?;
        }
        indent.truncate(len);
    } else {
        writeln!(output, "{prefix}{indent}{key}: {}", node_value(node))?;
    }
    Ok(())
}

fn has_changes(node: &DiffNode<'_>) -> bool {
    node.diff_type != DiffType::Unchanged || node.children.iter().any(has_changes)
}

fn is_document_node(node: &DiffNode<'_>) -> bool {
    node.left
        .is_some_and(|node| node.kind == NodeKind::Document)
        || node
            .right
            .is_some_and(|node| node.kind == NodeKind::Document)
}

fn is_container_node(node: &DiffNode<'_>) -> bool {
    let node = node.right.or(node.left);
    matches!(
        node.map(|node| node.kind),
        Some(NodeKind::Mapping | NodeKind::Sequence)
    )
}

fn is_scalar_node(node: &DiffNode<'_>) -> bool {
    node.left.is_some_and(|node| node.kind == NodeKind::Scalar)
        && node.right.is_some_and(|node| node.kind == NodeKind::Scalar)
}

/// How a diff row names its node: by key, else by sequence index.
enum NodeKey<'a> {
    Key(&'a str),
    Index(usize),
    Missing,
}

impl NodeKey<'_> {
    fn is_empty(&self) -> bool {
        matches!(self, NodeKey::Key("") | NodeKey::Missing)
    }
}

impl fmt::Display for NodeKey<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        match self {
            NodeKey::Key(key) => formatter.write_str(key),
            NodeKey::Index(index) => write!(formatter, "[{index}]"),
            NodeKey::Missing => Ok(()),
        }
    }
}

fn node_key<'a>(node: &DiffNode<'a>) -> NodeKey<'a> {
    if let Some(key) = node.left.and_then(|node| node.key.as_deref()) {
        return NodeKey::Key(key);
    }
    if let Some(key) = node.right.and_then(|node| node.key.as_deref()) {
        return NodeKey::Key(key);
    }
    if let Some(node) = node.left.or(node.right) {
        return NodeKey::Index(node.index);
    }
    NodeKey::Missing
}

/// The value shown for an added, removed or unchanged node.
struct NodeValue<'a>(Option<&'a YamNode>);

impl fmt::Display for NodeValue<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        let Some(node) = self.0 else {
            return Ok(());
        };
        match node.kind {
            NodeKind::Mapping => formatter.write_str("{...}"),
            NodeKind::Sequence => write!(formatter, "[{} items]", node.children.len()),
            _ => formatter.write_str(node.value.as_deref().unwrap_or_default()),
        }
    }
}

fn node_value<'a>(node: &DiffNode<'a>) -> NodeValue<'a> {
    NodeValue(node.right.or(node.left))
}

fn scalar_value(node: Option<&YamNode>) -> &str {
    node.and_then(|node| node.value.as_deref())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_yaml_str;

    /// The diff before it stopped building hash maps and sets per mapping.
    #[allow(dead_code)]
    mod before {
        use crate::parser::{NodeKind, YamNode};
        use std::collections::{BTreeSet, HashMap};
        use std::io::{self, Write};

        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum DiffType {
            Unchanged,
            Added,
            Removed,
            Modified,
        }

        #[derive(Debug, Clone)]
        pub struct DiffNode<'a> {
            pub left: Option<&'a YamNode>,
            pub right: Option<&'a YamNode>,
            pub diff_type: DiffType,
            pub children: Vec<DiffNode<'a>>,
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

        pub fn compare<'a>(left: &'a YamNode, right: &'a YamNode) -> DiffResult<'a> {
            let root = compare_nodes(Some(left), Some(right));
            let mut summary = DiffSummary::default();
            if let Some(root) = &root {
                walk_summary(root, &mut summary);
            }
            summary.total = summary.added + summary.removed + summary.modified;

            DiffResult {
                root,
                summary,
                left_file: String::new(),
                right_file: String::new(),
            }
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

        fn compare_nodes<'a>(
            left: Option<&'a YamNode>,
            right: Option<&'a YamNode>,
        ) -> Option<DiffNode<'a>> {
            match (left, right) {
                (None, None) => None,
                (None, Some(right)) => Some(DiffNode {
                    left: None,
                    right: Some(right),
                    diff_type: DiffType::Added,
                    children: Vec::new(),
                }),
                (Some(left), None) => Some(DiffNode {
                    left: Some(left),
                    right: None,
                    diff_type: DiffType::Removed,
                    children: Vec::new(),
                }),
                (Some(left), Some(right)) => compare_present_nodes(left, right),
            }
        }

        fn compare_present_nodes<'a>(
            left: &'a YamNode,
            right: &'a YamNode,
        ) -> Option<DiffNode<'a>> {
            if left.kind == NodeKind::Document && right.kind == NodeKind::Document {
                return compare_nodes(left.children.first(), right.children.first());
            }

            if left.kind == NodeKind::Mapping && right.kind == NodeKind::Mapping {
                let left_by_key: HashMap<&str, &YamNode> = left
                    .children
                    .iter()
                    .filter_map(|child| child.key.as_deref().map(|key| (key, child)))
                    .collect();
                let right_by_key: HashMap<&str, &YamNode> = right
                    .children
                    .iter()
                    .filter_map(|child| child.key.as_deref().map(|key| (key, child)))
                    .collect();

                let keys: BTreeSet<&str> = left_by_key
                    .keys()
                    .chain(right_by_key.keys())
                    .copied()
                    .collect();
                let mut children = Vec::new();
                let mut has_changes = false;

                for key in keys {
                    if let Some(child_diff) = compare_nodes(
                        left_by_key.get(key).copied(),
                        right_by_key.get(key).copied(),
                    ) {
                        has_changes |= child_diff.diff_type != DiffType::Unchanged;
                        children.push(child_diff);
                    }
                }

                return Some(DiffNode {
                    left: Some(left),
                    right: Some(right),
                    diff_type: if has_changes {
                        DiffType::Modified
                    } else {
                        DiffType::Unchanged
                    },
                    children,
                });
            }

            if left.kind == NodeKind::Sequence && right.kind == NodeKind::Sequence {
                let max_len = left.children.len().max(right.children.len());
                let mut children = Vec::new();
                let mut has_changes = false;

                for idx in 0..max_len {
                    if let Some(child_diff) =
                        compare_nodes(left.children.get(idx), right.children.get(idx))
                    {
                        has_changes |= child_diff.diff_type != DiffType::Unchanged;
                        children.push(child_diff);
                    }
                }

                return Some(DiffNode {
                    left: Some(left),
                    right: Some(right),
                    diff_type: if has_changes {
                        DiffType::Modified
                    } else {
                        DiffType::Unchanged
                    },
                    children,
                });
            }

            if left.kind == NodeKind::Scalar && right.kind == NodeKind::Scalar {
                return Some(DiffNode {
                    left: Some(left),
                    right: Some(right),
                    diff_type: if left.value == right.value {
                        DiffType::Unchanged
                    } else {
                        DiffType::Modified
                    },
                    children: Vec::new(),
                });
            }

            Some(DiffNode {
                left: Some(left),
                right: Some(right),
                diff_type: if left.kind == right.kind {
                    DiffType::Unchanged
                } else {
                    DiffType::Modified
                },
                children: Vec::new(),
            })
        }

        fn walk_summary(node: &DiffNode<'_>, summary: &mut DiffSummary) {
            match node.diff_type {
                DiffType::Added => summary.added += 1,
                DiffType::Removed => summary.removed += 1,
                DiffType::Modified => summary.modified += 1,
                DiffType::Unchanged => {}
            }
            for child in &node.children {
                walk_summary(child, summary);
            }
        }

        fn render_diff_node(
            output: &mut impl Write,
            node: &DiffNode<'_>,
            indent: &str,
        ) -> io::Result<()> {
            if node.diff_type == DiffType::Unchanged && !has_changes(node) {
                return Ok(());
            }
            if is_document_node(node) {
                for child in &node.children {
                    render_diff_node(output, child, indent)?;
                }
                return Ok(());
            }

            let key = node_key(node);
            if key.is_empty() && is_container_node(node) {
                for child in &node.children {
                    render_diff_node(output, child, indent)?;
                }
                return Ok(());
            }

            let prefix = match node.diff_type {
                DiffType::Added => "+ ",
                DiffType::Removed => "- ",
                DiffType::Modified => "~ ",
                DiffType::Unchanged => "  ",
            };

            if node.diff_type == DiffType::Modified && is_scalar_node(node) {
                writeln!(
                    output,
                    "{prefix}{indent}{key}: {} -> {}",
                    scalar_value(node.left),
                    scalar_value(node.right)
                )?;
            } else if is_container_node(node) {
                writeln!(output, "{prefix}{indent}{key}:")?;
                let child_indent = format!("{indent}  ");
                for child in &node.children {
                    render_diff_node(output, child, &child_indent)?;
                }
            } else {
                writeln!(output, "{prefix}{indent}{key}: {}", node_value(node))?;
            }
            Ok(())
        }

        fn has_changes(node: &DiffNode<'_>) -> bool {
            node.diff_type != DiffType::Unchanged || node.children.iter().any(has_changes)
        }

        fn is_document_node(node: &DiffNode<'_>) -> bool {
            node.left
                .is_some_and(|node| node.kind == NodeKind::Document)
                || node
                    .right
                    .is_some_and(|node| node.kind == NodeKind::Document)
        }

        fn is_container_node(node: &DiffNode<'_>) -> bool {
            let node = node.right.or(node.left);
            matches!(
                node.map(|node| node.kind),
                Some(NodeKind::Mapping | NodeKind::Sequence)
            )
        }

        fn is_scalar_node(node: &DiffNode<'_>) -> bool {
            node.left.is_some_and(|node| node.kind == NodeKind::Scalar)
                && node.right.is_some_and(|node| node.kind == NodeKind::Scalar)
        }

        fn node_key(node: &DiffNode<'_>) -> String {
            if let Some(key) = node.left.and_then(|node| node.key.clone()) {
                return key;
            }
            if let Some(key) = node.right.and_then(|node| node.key.clone()) {
                return key;
            }
            if let Some(node) = node.left.or(node.right) {
                return format!("[{}]", node.index);
            }
            String::new()
        }

        fn node_value(node: &DiffNode<'_>) -> String {
            let Some(node) = node.right.or(node.left) else {
                return String::new();
            };
            match node.kind {
                NodeKind::Mapping => "{...}".to_string(),
                NodeKind::Sequence => format!("[{} items]", node.children.len()),
                _ => node.value.clone().unwrap_or_default(),
            }
        }

        fn scalar_value(node: Option<&YamNode>) -> String {
            node.and_then(|node| node.value.clone()).unwrap_or_default()
        }
    }

    /// Each node's sides, by address, and its type, in preorder.
    fn shape(node: &DiffNode<'_>, out: &mut Vec<(usize, usize, DiffType)>) {
        let addr = |side: Option<&YamNode>| side.map_or(0, |node| node as *const _ as usize);
        out.push((addr(node.left), addr(node.right), node.diff_type));
        for child in &node.children {
            shape(child, out);
        }
    }

    fn shape_before(node: &before::DiffNode<'_>, out: &mut Vec<(usize, usize, DiffType)>) {
        let addr = |side: Option<&YamNode>| side.map_or(0, |node| node as *const _ as usize);
        let diff_type = match node.diff_type {
            before::DiffType::Unchanged => DiffType::Unchanged,
            before::DiffType::Added => DiffType::Added,
            before::DiffType::Removed => DiffType::Removed,
            before::DiffType::Modified => DiffType::Modified,
        };
        out.push((addr(node.left), addr(node.right), diff_type));
        for child in &node.children {
            shape_before(child, out);
        }
    }

    #[test]
    fn diff_matches_the_hashing_implementation() {
        let documents = [
            "",
            "5",
            "x",
            "[1, 2]",
            "a: 1\nb: [1, 2]\nc: {d: e}\n",
            "c: {d: f, g: 1}\nb: [1, 3, 4]\na: 1\nz: new\n",
            "a: {x: 1}\nb: [1]\n'': empty key\n",
            "a: [1]\nb: {x: 1}\n'': {k: v}\n",
            "1: int\n'1': string\nm: [a, {b: c}]\n",
            "'1': string\n1: int\nm: [a, {b: d}, e]\n",
            "- {a: 1, b: 2}\n- [x, y]\n- s\n",
            "- {b: 2, a: 1, c: 3}\n- [x]\n- {s: 1}\n- extra\n",
            "k1: 1\nk10: 10\nk2: 2\nK1: 0\nk: ~\n",
            "k10: 11\nk2: 2\nka: 1\nk: null\n",
        ];
        let roots: Vec<YamNode> = documents
            .iter()
            .map(|input| parse_yaml_str(input).unwrap())
            .collect();
        for left in &roots {
            for right in &roots {
                let mut result = compare(left, right);
                let mut expected = before::compare(left, right);
                assert_eq!(result.summary, {
                    let s = &expected.summary;
                    DiffSummary {
                        added: s.added,
                        removed: s.removed,
                        modified: s.modified,
                        total: s.total,
                    }
                });
                let (mut got, mut want) = (Vec::new(), Vec::new());
                if let Some(root) = &result.root {
                    shape(root, &mut got);
                }
                if let Some(root) = &expected.root {
                    shape_before(root, &mut want);
                }
                assert_eq!(got, want, "{left:?} vs {right:?}");

                for files in [("", ""), ("left.yaml", "right.yaml")] {
                    result.left_file = files.0.into();
                    result.right_file = files.1.into();
                    expected.left_file = files.0.into();
                    expected.right_file = files.1.into();
                    let (mut output, mut want_output) = (Vec::new(), Vec::new());
                    render(&mut output, &result).unwrap();
                    before::render(&mut want_output, &expected).unwrap();
                    assert_eq!(
                        String::from_utf8(output).unwrap(),
                        String::from_utf8(want_output).unwrap()
                    );
                }
            }
        }
    }

    #[test]
    fn compares_mapping_and_counts_parent_modified() {
        let left = parse_yaml_str("app:\n  name: yam\n").unwrap();
        let right = parse_yaml_str("app:\n  name: rust-yam\n  version: 1\n").unwrap();
        let result = compare(&left, &right);

        assert_eq!(result.summary.added, 1);
        assert_eq!(result.summary.modified, 3);
        assert_eq!(result.summary.total, 4);
    }

    #[test]
    fn sequence_diff_is_index_based() {
        let left = parse_yaml_str("items:\n  - one\n  - two\n").unwrap();
        let right = parse_yaml_str("items:\n  - zero\n  - two\n").unwrap();
        let result = compare(&left, &right);

        assert_eq!(result.summary.modified, 3);
    }
}
