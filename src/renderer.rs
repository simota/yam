use crate::parser::{NodeKind, ScalarType, YamNode};
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
    let mut prefix = String::new();
    render_node(
        output,
        root,
        &mut prefix,
        true,
        chars_for(style),
        show_types,
    )
}

/// `prefix` is the branch art for `node`'s row; it is shared by the whole walk
/// and restored before returning.
fn render_node(
    output: &mut impl Write,
    node: &YamNode,
    prefix: &mut String,
    is_last: bool,
    chars: TreeChars,
    show_types: bool,
) -> io::Result<()> {
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

    render_single_node(output, node, prefix, is_last, chars, show_types)?;

    if node.has_children() {
        // Children of a root start from an empty prefix.
        let saved = (node.depth == 0).then(|| std::mem::take(prefix));
        let len = prefix.len();
        if saved.is_none() {
            push_child_prefix(prefix, is_last, chars);
        }
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
        match saved {
            Some(saved) => *prefix = saved,
            None => prefix.truncate(len),
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
        output.write_all(prefix.as_bytes())?;
        output.write_all(branch.as_bytes())?;
        output.write_all(chars.horizontal.as_bytes())?;
        output.write_all(b" ")?;
    }

    if let Some(key) = &node.key {
        output.write_all(key.as_bytes())?;
        output.write_all(b": ")?;
    } else if node.depth > 0 {
        write!(output, "[{}] ", node.index)?;
    }

    match node.kind {
        NodeKind::Mapping | NodeKind::Sequence | NodeKind::Document => {}
        NodeKind::Scalar => write_value(output, node, show_types)?,
    }
    output.write_all(b"\n")
}

fn write_value(output: &mut impl Write, node: &YamNode, show_types: bool) -> io::Result<()> {
    let value = node.value.as_deref().unwrap_or_default();
    let scalar_type = node.infer_type();
    match scalar_type {
        ScalarType::Null if value.is_empty() || value == "~" => output.write_all(b"null")?,
        ScalarType::String if needs_quoting(value) => write!(output, "{value:?}")?,
        _ => output.write_all(value.as_bytes())?,
    }

    if show_types {
        output.write_all(b" ")?;
        output.write_all(type_label(scalar_type).as_bytes())?;
    }
    Ok(())
}

/// Extends a non-root node's prefix to the prefix of its children.
fn push_child_prefix(prefix: &mut String, is_last: bool, chars: TreeChars) {
    if is_last {
        prefix.push_str("    ");
    } else {
        prefix.push_str(chars.vertical);
        prefix.push_str("   ");
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
    const RESERVED: [&str; 8] = ["true", "false", "yes", "no", "on", "off", "null", "~"];
    if RESERVED
        .iter()
        .any(|reserved| input.eq_ignore_ascii_case(reserved))
    {
        return true;
    }
    if input.len() != input.trim().len() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{parse_json, parse_yaml_str, take_by_path};

    /// The renderer before it stopped allocating per node.
    mod before {
        use super::super::{chars_for, type_label, TreeChars, TreeStyle};
        use crate::parser::{NodeKind, ScalarType, YamNode};
        use std::io::{self, Write};

        pub fn render_before(
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

            render_single_node(output, node, prefix, is_last, chars, show_types)?;

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
                NodeKind::Mapping | NodeKind::Sequence | NodeKind::Document => {}
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
    }

    #[test]
    fn output_matches_the_allocating_renderer() {
        let mut trees: Vec<YamNode> = [
            "",
            "plain",
            "a: 1\nb: [1, 2.5, x, null, true, '', ' pad', '~', 'No', 'a: b', 'c # d', \"t\\tab\"]\nc: {d: e}\ne: {}\nf: []\n",
            "- a\n- [b, [c, {d: [e]}]]\n- {}\n- []\n",
            "k: \"multi\\nline\"\nu: \"\\u00e9\\u0001\"\nq: '\"'\n",
            "top:\n  mid:\n    - leaf: 1\n    - leaf: 2\n  last: {x: [1, [2, [3]]]}\nend: ~\n",
        ]
        .iter()
        .map(|input| parse_yaml_str(input).unwrap())
        .collect();
        trees.push(parse_json(br#"{"a": [1, {"b": null}], "c": "true"}"#).unwrap());
        let root = parse_yaml_str("a:\n  b:\n    - {c: 1, d: [2]}\n    - e\n").unwrap();
        trees.push(take_by_path(root.clone(), ".a.b").unwrap());
        trees.push(take_by_path(root.clone(), ".a.b[0]").unwrap());
        // A depth-0 node below other nodes starts its children's prefix over.
        let mut nested = parse_yaml_str("x:\n  - y\n  - z: [1, 2]\nw: 3\n").unwrap();
        let inner = parse_yaml_str("p: {q: 1}\nr: 2\n")
            .unwrap()
            .children
            .remove(0);
        nested.children[0].children[0].children.insert(0, inner);
        trees.push(nested);
        let mut untagged = parse_yaml_str("a: ''\nb: '~'\nc: NULL\nd: 'yes'\n").unwrap();
        for child in &mut untagged.children[0].children {
            child.tag = None;
        }
        trees.push(untagged);

        for tree in &trees {
            for style in [TreeStyle::Unicode, TreeStyle::Ascii, TreeStyle::Indent] {
                for show_types in [false, true] {
                    let mut output = Vec::new();
                    render(&mut output, tree, style, show_types).unwrap();
                    let mut expected = Vec::new();
                    before::render_before(&mut expected, tree, style, show_types).unwrap();
                    assert_eq!(
                        String::from_utf8(output).unwrap(),
                        String::from_utf8(expected).unwrap(),
                        "{tree:?} {style:?} {show_types}"
                    );
                }
            }
        }
    }
}
