use crate::parser::{NodeKind, YamNode};
use std::collections::{BTreeSet, HashMap};

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
    pub path: String,
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
    let root = compare_nodes(Some(left), Some(right), "$");
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

pub fn render(result: &DiffResult<'_>) -> String {
    let mut output = String::new();
    if !result.left_file.is_empty() || !result.right_file.is_empty() {
        output.push_str(&format!("--- {}\n", result.left_file));
        output.push_str(&format!("+++ {}\n\n", result.right_file));
    }

    if let Some(root) = &result.root {
        render_diff_node(&mut output, root, "");
    }

    if result.summary.total > 0 {
        output.push('\n');
        output.push_str(&render_summary(&result.summary));
        output.push('\n');
    }

    output
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
    path: &str,
) -> Option<DiffNode<'a>> {
    match (left, right) {
        (None, None) => None,
        (None, Some(right)) => Some(DiffNode {
            left: None,
            right: Some(right),
            diff_type: DiffType::Added,
            children: Vec::new(),
            path: path.to_string(),
        }),
        (Some(left), None) => Some(DiffNode {
            left: Some(left),
            right: None,
            diff_type: DiffType::Removed,
            children: Vec::new(),
            path: path.to_string(),
        }),
        (Some(left), Some(right)) => compare_present_nodes(left, right, path),
    }
}

fn compare_present_nodes<'a>(
    left: &'a YamNode,
    right: &'a YamNode,
    path: &str,
) -> Option<DiffNode<'a>> {
    if left.kind == NodeKind::Document && right.kind == NodeKind::Document {
        return compare_nodes(left.children.first(), right.children.first(), path);
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
            let child_path = format!("{path}.{key}");
            if let Some(child_diff) = compare_nodes(
                left_by_key.get(key).copied(),
                right_by_key.get(key).copied(),
                &child_path,
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
            path: path.to_string(),
        });
    }

    if left.kind == NodeKind::Sequence && right.kind == NodeKind::Sequence {
        let max_len = left.children.len().max(right.children.len());
        let mut children = Vec::new();
        let mut has_changes = false;

        for idx in 0..max_len {
            let child_path = format!("{path}[{idx}]");
            if let Some(child_diff) =
                compare_nodes(left.children.get(idx), right.children.get(idx), &child_path)
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
            path: path.to_string(),
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
            path: path.to_string(),
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
        path: path.to_string(),
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

fn render_diff_node(output: &mut String, node: &DiffNode<'_>, indent: &str) {
    if node.diff_type == DiffType::Unchanged && !has_changes(node) {
        return;
    }
    if is_document_node(node) {
        for child in &node.children {
            render_diff_node(output, child, indent);
        }
        return;
    }

    let key = node_key(node);
    if key.is_empty() && is_container_node(node) {
        for child in &node.children {
            render_diff_node(output, child, indent);
        }
        return;
    }

    let prefix = match node.diff_type {
        DiffType::Added => "+ ",
        DiffType::Removed => "- ",
        DiffType::Modified => "~ ",
        DiffType::Unchanged => "  ",
    };

    if node.diff_type == DiffType::Modified && is_scalar_node(node) {
        output.push_str(&format!(
            "{prefix}{indent}{key}: {} -> {}\n",
            scalar_value(node.left),
            scalar_value(node.right)
        ));
    } else if is_container_node(node) {
        output.push_str(&format!("{prefix}{indent}{key}:\n"));
        let child_indent = format!("{indent}  ");
        for child in &node.children {
            render_diff_node(output, child, &child_indent);
        }
    } else {
        output.push_str(&format!("{prefix}{indent}{key}: {}\n", node_value(node)));
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_yaml_str;

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
