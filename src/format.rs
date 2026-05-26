use crate::parser::{node_to_ordered_yaml, YamNode};
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

pub fn format_yaml(node: &YamNode, opts: FormatOptions) -> Result<String> {
    let value = node_to_ordered_yaml(node, opts.sort_keys);
    let mut output = serde_yaml::to_string(&value).context("failed to format YAML")?;
    output = normalize_indentation(&output, opts.indent);
    if !output.ends_with('\n') {
        output.push('\n');
    }
    Ok(output)
}

fn normalize_indentation(input: &str, indent: usize) -> String {
    if indent == 2 {
        return input.to_string();
    }
    input
        .lines()
        .map(|line| {
            let leading = line.chars().take_while(|ch| *ch == ' ').count();
            let level = leading / 2;
            format!("{}{}", " ".repeat(level * indent), line.trim_start())
        })
        .collect::<Vec<_>>()
        .join("\n")
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
}
