use crate::parser::{YamNode, YamlView};
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

    /// The line-by-line rewrite before it wrote into one buffer.
    fn normalize_indentation_before(input: &str, indent: usize) -> String {
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

    #[test]
    fn indentation_matches_the_line_by_line_rewrite() {
        let inputs = [
            "",
            "\n",
            "a\n",
            "a\n\nb",
            "a:\n  b:\n    - c\n     odd: 5\n\t tab\n  \u{3000}wide\r\n   x\r\ny",
            "no newline",
            "  lead\n",
        ];
        for input in inputs {
            for indent in [0, 1, 2, 3, 4, 8] {
                assert_eq!(
                    normalize_indentation(input.to_string(), indent),
                    normalize_indentation_before(input, indent),
                    "{input:?} {indent}"
                );
            }
        }
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
