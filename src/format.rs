use crate::parser::{documents, JsonView, NodeKind, YamNode, YamlView};
use anyhow::{Context, Result};

#[derive(Debug, Clone, Copy)]
pub struct FormatOptions {
    pub indent: usize,
    pub sort_keys: bool,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            indent: 2,
            sort_keys: false,
        }
    }
}

/// Formats YAML. Every document of a stream is written, separated by "---".
pub fn format_yaml(node: &YamNode, opts: FormatOptions) -> Result<String> {
    if node.kind == NodeKind::Stream {
        let mut out = String::new();
        for (i, doc) in node.children.iter().enumerate() {
            if i > 0 {
                out.push_str("---\n");
            }
            out.push_str(&format_document(doc, opts)?);
        }
        return Ok(out);
    }
    format_document(node, opts)
}

/// Formats every document as indented JSON (one value per document) with
/// key order preserved.
pub fn format_json(node: &YamNode, opts: FormatOptions) -> Result<String> {
    let mut out = String::new();
    for doc in documents(node) {
        let value = JsonView {
            node: doc,
            sort_keys: opts.sort_keys,
        };
        let indent = " ".repeat(opts.indent.max(1));
        let formatter = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
        let mut buf = Vec::new();
        let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);
        serde::Serialize::serialize(&value, &mut ser).context("failed to format JSON")?;
        out.push_str(&String::from_utf8(buf).context("invalid UTF-8")?);
        out.push('\n');
    }
    Ok(out)
}

fn format_document(node: &YamNode, opts: FormatOptions) -> Result<String> {
    let view = YamlView {
        node,
        sort_keys: opts.sort_keys,
    };
    let output = serde_yaml::to_string(&view).context("failed to format YAML")?;
    let mut output = normalize_indentation(output, opts.indent);
    if !output.ends_with('\n') {
        output.push('\n');
    }
    Ok(output)
}

fn normalize_indentation(input: String, indent: usize) -> String {
    if indent == 2 {
        return input;
    }
    let mut output = String::with_capacity(input.len());
    for (idx, line) in input.lines().enumerate() {
        if idx > 0 {
            output.push('\n');
        }
        let leading = line.bytes().take_while(|byte| *byte == b' ').count();
        let level = leading / 2;
        for _ in 0..level * indent {
            output.push(' ');
        }
        output.push_str(line.trim_start());
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_yaml_str;

    #[test]
    fn sorts_mapping_keys() {
        let node = parse_yaml_str("zebra: 1\napple: 2\nmango: 3\n").unwrap();
        let output = format_yaml(
            &node,
            FormatOptions {
                indent: 2,
                sort_keys: true,
            },
        )
        .unwrap();

        assert!(output.starts_with("apple: 2\nmango: 3\nzebra: 1\n"));
    }

    #[test]
    fn supports_custom_indentation() {
        let node = parse_yaml_str("app:\n  name: yam\n").unwrap();
        let output = format_yaml(
            &node,
            FormatOptions {
                indent: 4,
                sort_keys: false,
            },
        )
        .unwrap();

        assert!(output.contains("    name: yam"));
    }

    #[test]
    fn keeps_every_document() {
        let node = parse_yaml_str("a:   1\n---\nb:    2\n").unwrap();
        assert_eq!(
            format_yaml(&node, FormatOptions::default()).unwrap(),
            "a: 1\n---\nb: 2\n"
        );
    }

    #[test]
    fn formats_json_with_order() {
        let node = crate::parser::parse_json(br#"{"b":1,"a":[1,2]}"#).unwrap();
        assert_eq!(
            format_json(&node, FormatOptions::default()).unwrap(),
            "{\n  \"b\": 1,\n  \"a\": [\n    1,\n    2\n  ]\n}\n"
        );
    }
}
