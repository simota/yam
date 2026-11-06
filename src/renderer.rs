use crate::parser::{resource_id, NodeKind, ScalarType, YamNode};
use std::io::{self, Write};

#[derive(Debug, Clone, Copy)]
pub enum TreeStyle {
    Unicode,
    Ascii,
    Indent,
}

#[derive(Debug, Clone, Copy)]
struct TreeChars {
    vertical: &'static str,
    horizontal: &'static str,
    corner: &'static str,
    tee: &'static str,
}

pub fn render(
    output: &mut impl Write,
    root: &YamNode,
    style: TreeStyle,
    show_types: bool,
) -> io::Result<()> {
    render_node(output, root, "", true, chars_for(style), show_types)
}

fn render_node(
    output: &mut impl Write,
    node: &YamNode,
    prefix: &str,
    is_last: bool,
    chars: TreeChars,
    show_types: bool,
) -> io::Result<()> {
    if node.kind == NodeKind::Stream {
        for doc in &node.children {
            writeln!(output, "--- {}", document_label(doc))?;
            render_node(output, doc, prefix, true, chars, show_types)?;
        }
        return Ok(());
    }
    if node.kind == NodeKind::Document {
        for (idx, child) in node.children.iter().enumerate() {
            render_node(
                output,
                child,
                prefix,
                idx == node.children.len() - 1,
                chars,
                show_types,
            )?;
        }
        return Ok(());
    }

    // The top-level container has no key and would be an empty line.
    let is_root_container = node.depth == 0 && node.key.is_none() && node.has_children();
    if !is_root_container {
        render_single_node(output, node, prefix, is_last, chars, show_types)?;
    }

    if node.has_children() {
        let new_prefix = child_prefix(prefix, is_last, node.depth, chars);
        for (idx, child) in node.children.iter().enumerate() {
            render_node(
                output,
                child,
                &new_prefix,
                idx == node.children.len() - 1,
                chars,
                show_types,
            )?;
        }
    }
    Ok(())
}

fn render_single_node(
    output: &mut impl Write,
    node: &YamNode,
    prefix: &str,
    is_last: bool,
    chars: TreeChars,
    show_types: bool,
) -> io::Result<()> {
    if node.depth > 0 {
        let branch = if is_last { chars.corner } else { chars.tee };
        write!(output, "{prefix}{branch}{} ", chars.horizontal)?;
    }

    if let Some(key) = &node.key {
        write!(output, "{key}: ")?;
    } else if node.depth > 0 {
        write!(output, "[{}] ", node.index)?;
    }

    match node.kind {
        NodeKind::Mapping | NodeKind::Sequence => {
            if node.children.is_empty() {
                let empty = if node.kind == NodeKind::Mapping {
                    "{}"
                } else {
                    "[]"
                };
                output.write_all(empty.as_bytes())?;
            }
        }
        NodeKind::Document | NodeKind::Stream => {}
        NodeKind::Scalar => output.write_all(render_value(node, show_types).as_bytes())?,
    }
    output.write_all(b"\n")
}

fn render_value(node: &YamNode, show_types: bool) -> String {
    let value = node.value.as_deref().unwrap_or_default();
    let scalar_type = node.infer_type();
    let rendered = match scalar_type {
        ScalarType::Null if value.is_empty() || value == "~" => "null".to_string(),
        ScalarType::String if needs_quoting(value) => format!("{value:?}"),
        _ => value.to_string(),
    };

    if show_types {
        format!("{rendered} <{}>", node.type_label())
    } else {
        rendered
    }
}

/// "document N" plus "(Kind/name)" for Kubernetes-style documents.
pub fn document_label(doc: &YamNode) -> String {
    match resource_id(doc) {
        Some(id) => format!("document {} ({id})", doc.index + 1),
        None => format!("document {}", doc.index + 1),
    }
}

fn child_prefix(prefix: &str, is_last: bool, depth: usize, chars: TreeChars) -> String {
    if depth == 0 {
        String::new()
    } else if is_last {
        format!("{prefix}    ")
    } else {
        format!("{prefix}{}   ", chars.vertical)
    }
}

fn needs_quoting(input: &str) -> bool {
    if input.is_empty() {
        return true;
    }
    match input.to_ascii_lowercase().as_str() {
        "true" | "false" | "yes" | "no" | "on" | "off" | "null" | "~" => return true,
        _ => {}
    }
    if input != input.trim() {
        return true;
    }
    input
        .chars()
        .any(|ch| matches!(ch, ':' | '#' | '\n' | '\t'))
}

fn chars_for(style: TreeStyle) -> TreeChars {
    match style {
        TreeStyle::Ascii => TreeChars {
            vertical: "|",
            horizontal: "-",
            corner: "`",
            tee: "+",
        },
        TreeStyle::Indent => TreeChars {
            vertical: " ",
            horizontal: " ",
            corner: " ",
            tee: " ",
        },
        TreeStyle::Unicode => TreeChars {
            vertical: "│",
            horizontal: "─",
            corner: "└",
            tee: "├",
        },
    }
}
