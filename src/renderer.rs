use crate::parser::{NodeKind, ScalarType, YamNode};

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

pub fn render(root: &YamNode, style: TreeStyle, show_types: bool) -> String {
    let mut output = String::new();
    let chars = chars_for(style);
    render_node(&mut output, root, "", true, chars, show_types);
    output
}

fn render_node(
    output: &mut String,
    node: &YamNode,
    prefix: &str,
    is_last: bool,
    chars: TreeChars,
    show_types: bool,
) {
    if node.kind == NodeKind::Document {
        for (idx, child) in node.children.iter().enumerate() {
            render_node(
                output,
                child,
                prefix,
                idx == node.children.len() - 1,
                chars,
                show_types,
            );
        }
        return;
    }

    render_single_node(output, node, prefix, is_last, chars, show_types);

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
            );
        }
    }
}

fn render_single_node(
    output: &mut String,
    node: &YamNode,
    prefix: &str,
    is_last: bool,
    chars: TreeChars,
    show_types: bool,
) {
    if node.depth > 0 {
        output.push_str(prefix);
        output.push_str(if is_last { chars.corner } else { chars.tee });
        output.push_str(chars.horizontal);
        output.push(' ');
    }

    if let Some(key) = &node.key {
        output.push_str(key);
        output.push_str(": ");
    } else if node.depth > 0 {
        output.push_str(&format!("[{}] ", node.index));
    }

    match node.kind {
        NodeKind::Mapping | NodeKind::Sequence | NodeKind::Document => {}
        NodeKind::Scalar => output.push_str(&render_value(node, show_types)),
    }
    output.push('\n');
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
        format!("{rendered} {}", type_label(scalar_type))
    } else {
        rendered
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

fn type_label(scalar_type: ScalarType) -> &'static str {
    match scalar_type {
        ScalarType::String => "<str>",
        ScalarType::Number => "<int>",
        ScalarType::Boolean => "<bool>",
        ScalarType::Null => "<null>",
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
