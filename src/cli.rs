use crate::diff;
use crate::format::{format_yaml, FormatOptions};
use crate::parser::{get_by_path, parse_file, parse_json, parse_yaml, to_json_value, to_raw_value};
use crate::renderer::{self, TreeStyle};
use crate::tui;
use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use std::fs;
use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(
    name = "yam",
    version,
    about = "A beautiful YAML renderer for the terminal"
)]
struct Cli {
    #[arg(short = 'i', long, help = "Interactive TUI mode")]
    interactive: bool,

    #[arg(
        short = 's',
        long,
        default_value = "unicode",
        help = "Tree style: unicode, ascii, indent"
    )]
    style: String,

    #[arg(short = 't', long, help = "Show type annotations")]
    types: bool,

    #[arg(short = 'j', long, help = "Output as JSON")]
    json: bool,

    #[arg(short = 'r', long, help = "Output raw value without decoration")]
    raw: bool,

    #[command(subcommand)]
    command: Option<Command>,

    #[arg(value_name = "path_or_file")]
    args: Vec<String>,
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(about = "Format YAML files")]
    Fmt(FmtArgs),

    #[command(about = "Compare two YAML/JSON files")]
    Diff(DiffArgs),
}

#[derive(Debug, Args)]
struct FmtArgs {
    #[arg(
        short = 'w',
        long,
        help = "Write result to source file instead of stdout"
    )]
    write: bool,

    #[arg(
        short = 'i',
        long,
        default_value_t = 2,
        help = "Indentation width in spaces"
    )]
    indent: usize,

    #[arg(short = 's', long, help = "Sort keys alphabetically")]
    sort_keys: bool,

    file: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct DiffArgs {
    #[arg(short = 'i', long, help = "Interactive TUI mode with split view")]
    interactive: bool,

    #[arg(short = 's', long, help = "Show only summary (no detailed diff)")]
    summary: bool,

    file1: PathBuf,
    file2: PathBuf,
}

pub fn run() -> Result<i32> {
    run_with(Cli::parse())
}

fn run_with(cli: Cli) -> Result<i32> {
    match cli.command {
        Some(Command::Fmt(args)) => run_fmt(args),
        Some(Command::Diff(args)) => run_diff(args),
        None => run_root(cli),
    }
}

fn run_root(cli: Cli) -> Result<i32> {
    if cli.args.len() > 2 {
        bail!("accepts at most 2 positional arguments: [path] [file]");
    }

    let (path_query, filename) = parse_root_args(&cli.args);
    let input = read_input(filename.as_deref())?;
    let mut node = if filename
        .as_deref()
        .map(crate::parser::is_json_file)
        .unwrap_or(false)
    {
        parse_json(&mut input.as_slice())?
    } else {
        parse_yaml(&mut input.as_slice())?
    };

    if let Some(path_query) = path_query {
        node = get_by_path(&node, &path_query)?.clone();
    }

    let style = parse_tree_style(&cli.style);
    if cli.interactive {
        let display_name = filename
            .as_deref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "stdin".to_string());
        tui::run_tree(node, filename, display_name, style, cli.types)?;
        return Ok(0);
    }

    if cli.raw {
        println!("{}", to_raw_value(&node));
        return Ok(0);
    }

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&to_json_value(&node))?);
        return Ok(0);
    }

    print!("{}", renderer::render(&node, style, cli.types));
    Ok(0)
}

fn run_fmt(args: FmtArgs) -> Result<i32> {
    if args.write && args.file.is_none() {
        bail!("cannot use -w with stdin input");
    }
    let input = read_input(args.file.as_deref())?;
    let node = parse_yaml(&mut input.as_slice()).context("invalid YAML")?;
    let output = format_yaml(
        &node,
        FormatOptions {
            indent: args.indent,
            sort_keys: args.sort_keys,
        },
    )?;

    if let Some(file) = args.file {
        if args.write {
            write_atomic(&file, output.as_bytes())?;
        } else {
            print!("{output}");
        }
    } else {
        print!("{output}");
    }

    Ok(0)
}

fn run_diff(args: DiffArgs) -> Result<i32> {
    let left = parse_file(&args.file1)?;
    let right = parse_file(&args.file2)?;
    let mut result = diff::compare(&left, &right);
    result.left_file = args.file1.display().to_string();
    result.right_file = args.file2.display().to_string();

    if args.interactive {
        tui::run_diff(&result)?;
        return Ok(0);
    }

    if args.summary {
        println!("{}", diff::render_summary(&result.summary));
    } else if result.summary.total == 0 {
        println!("No differences found.");
    } else {
        print!("{}", diff::render(&result));
    }

    Ok(if result.summary.total > 0 { 1 } else { 0 })
}

fn parse_root_args(args: &[String]) -> (Option<String>, Option<PathBuf>) {
    match args {
        [path, file] => (Some(path.clone()), Some(PathBuf::from(file))),
        [single] if single.starts_with('.') => (Some(single.clone()), None),
        [single] => (None, Some(PathBuf::from(single))),
        _ => (None, None),
    }
}

fn read_input(path: Option<&Path>) -> Result<Vec<u8>> {
    if let Some(path) = path {
        return fs::read(path).with_context(|| format!("failed to open file: {}", path.display()));
    }

    if io::stdin().is_terminal() {
        bail!("no input: provide a file or pipe YAML content");
    }
    let mut input = Vec::new();
    io::stdin().read_to_end(&mut input)?;
    Ok(input)
}

fn write_atomic(path: &Path, content: &[u8]) -> Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = dir.join(format!(".yam-fmt-{}.yaml", std::process::id()));
    fs::write(&tmp, content)
        .with_context(|| format!("failed to create temp file: {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("failed to write file: {}", path.display()))?;
    Ok(())
}

fn parse_tree_style(style: &str) -> TreeStyle {
    match style {
        "ascii" => TreeStyle::Ascii,
        "indent" => TreeStyle::Indent,
        _ => TreeStyle::Unicode,
    }
}

#[cfg(test)]
mod tests {
    use super::parse_root_args;
    use std::path::PathBuf;

    #[test]
    fn parses_root_args_like_go_cli() {
        assert_eq!(parse_root_args(&[]), (None, None));
        assert_eq!(
            parse_root_args(&["file.yaml".to_string()]),
            (None, Some(PathBuf::from("file.yaml")))
        );
        assert_eq!(
            parse_root_args(&[".metadata.name".to_string()]),
            (Some(".metadata.name".to_string()), None)
        );
        assert_eq!(
            parse_root_args(&[".metadata.name".to_string(), "file.yaml".to_string()]),
            (
                Some(".metadata.name".to_string()),
                Some(PathBuf::from("file.yaml"))
            )
        );
    }
}
