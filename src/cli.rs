use crate::diff::{self, DiffOptions};
use crate::format::{format_json, format_yaml, FormatOptions};
use crate::parser::{
    content, documents, is_json_file, parse_json, parse_yaml, to_raw_value, write_json_pretty,
    NodeKind, YamNode,
};
use crate::path::{
    delete_path, format_path, get_by_path, is_multi, leaf_paths, parse_path, parse_value, query,
    set_path, target_documents, Hit, Segment,
};
use crate::renderer::{self, TreeStyle};
use crate::tui;
use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use std::fs;
use std::io::{self, BufWriter, IsTerminal, Read, StdoutLock, Write};
use std::path::{Path, PathBuf};

const ROOT_HELP: &str = "\
Path syntax:
  .key.sub          nested keys
  .items[0]         array element (negative indexes count from the end)
  .items[*].name    every element
  .data.*           every value of a mapping
  .[\"a.b\"]          keys containing dots or special characters
  ..image           \"image\" at any depth

Multi-document files (\"---\") are fully supported: paths are evaluated
against every document; use --doc to pick one.

Examples:
  yam config.yaml               Render a file
  yam -i config.yaml            Interactive TUI mode
  yam '.data.host' config.yaml  Extract the value at a path
  yam -r '..image' k8s.yaml     Every \"image\" in every document
  yam --yaml data.json          Convert JSON to YAML
  yam --paths config.yaml       List every leaf as 'path = value'";

#[derive(Debug, Parser)]
#[command(
    name = "yam",
    version,
    about = "A beautiful YAML renderer for the terminal",
    after_help = ROOT_HELP
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

    #[arg(short = 'j', long, help = "Output as JSON", group = "output")]
    json: bool,

    #[arg(
        short = 'y',
        long,
        help = "Output as YAML (e.g. convert JSON to YAML)",
        group = "output"
    )]
    yaml: bool,

    #[arg(
        short = 'p',
        long,
        help = "List every leaf value as 'path = value'",
        group = "output"
    )]
    paths: bool,

    #[arg(
        short = 'r',
        long,
        help = "Output raw value without decoration",
        group = "output"
    )]
    raw: bool,

    #[arg(
        short = 'd',
        long,
        value_name = "N",
        help = "Select document N (1-based) of a multi-document file"
    )]
    doc: Option<usize>,

    #[command(subcommand)]
    command: Option<Command>,

    #[arg(value_name = "path_or_file")]
    args: Vec<String>,
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(about = "Format YAML (and JSON) files")]
    Fmt(FmtArgs),

    #[command(about = "Compare two YAML/JSON files")]
    Diff(DiffArgs),

    #[command(about = "Set the value at a path")]
    Set(SetArgs),

    #[command(about = "Delete the value at a path", visible_alias = "rm")]
    Del(DelArgs),
}

#[derive(Debug, Args)]
#[command(after_help = "\
All documents of multi-document files are kept. JSON files (.json) are
formatted as indented JSON with key order preserved. Files that are already
formatted are not rewritten.

Exit codes: 0 success (with --check: everything formatted),
            1 error (with --check: some files need formatting)")]
struct FmtArgs {
    #[arg(
        short = 'w',
        long,
        help = "Write result to source files instead of stdout",
        conflicts_with = "check"
    )]
    write: bool,

    #[arg(
        short = 'c',
        long,
        help = "List files that need formatting and exit 1 (for CI)"
    )]
    check: bool,

    #[arg(
        short = 'i',
        long,
        default_value_t = 2,
        help = "Indentation width in spaces"
    )]
    indent: usize,

    #[arg(short = 's', long, help = "Sort keys alphabetically")]
    sort_keys: bool,

    files: Vec<PathBuf>,
}

#[derive(Debug, Args)]
#[command(after_help = "\
Multi-document files are compared document by document; Kubernetes-style
resources are matched by Kind/namespace/name. Lists of mappings with a unique
\"name\", \"id\" or \"key\" field are matched by that field; other lists are
aligned so that one inserted item is reported as one addition.
Use - for one side to read stdin.

Exit codes: 0 no differences, 1 differences found, 2 error")]
struct DiffArgs {
    #[arg(short = 'i', long, help = "Interactive TUI mode with split view")]
    interactive: bool,

    #[arg(
        short = 's',
        long,
        help = "Show only summary (no detailed diff)",
        conflicts_with = "json"
    )]
    summary: bool,

    #[arg(short = 'j', long, help = "Output changes as JSON")]
    json: bool,

    #[arg(
        short = 'p',
        long,
        value_name = "PATH",
        help = "Only compare the subtree at this path (e.g. .spec)"
    )]
    path: Option<String>,

    file1: PathBuf,
    file2: PathBuf,
}

#[derive(Debug, Args)]
#[command(after_help = "\
The value is parsed as YAML: 42 is a number, true a boolean, '[a, b]' a list
and '{k: v}' a mapping. Use --string to store text as a string.
Missing parent keys are created; setting index N on a list of length N
appends. In multi-document files the value is set in every document that
already has the key (or, if none has it, every document that has its parent);
use --doc N to target one document.

Note: the file is re-serialized, so YAML comments are not preserved.

Examples:
  yam set .image.tag v1.2.3 values.yaml
  yam set -w .replicas 3 deploy.yaml
  yam set -w .version 1.10 --string chart.yaml
  cat cfg.yaml | yam set .debug true")]
struct SetArgs {
    #[command(flatten)]
    edit: EditArgs,

    #[arg(short = 's', long, help = "Store the value as a string")]
    string: bool,

    path: String,
    value: String,
    file: Option<PathBuf>,
}

#[derive(Debug, Args)]
#[command(after_help = "\
In multi-document files the path is removed from every document containing
it; use --doc N to target one document. Fails if nothing was deleted.

Note: the file is re-serialized, so YAML comments are not preserved.")]
struct DelArgs {
    #[command(flatten)]
    edit: EditArgs,

    path: String,
    file: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct EditArgs {
    #[arg(
        short = 'w',
        long,
        help = "Write result back to the file instead of stdout"
    )]
    write: bool,

    #[arg(
        short = 'd',
        long,
        value_name = "N",
        help = "Only change document N (1-based) of a multi-document file"
    )]
    doc: Option<usize>,
}

/// An error that ends the process with a specific exit code.
#[derive(Debug)]
pub struct ExitCode {
    pub code: i32,
    pub error: Option<anyhow::Error>,
}

impl std::fmt::Display for ExitCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.error {
            Some(err) => write!(f, "{err:#}"),
            None => write!(f, "exit status {}", self.code),
        }
    }
}

impl std::error::Error for ExitCode {}

fn exit_with(code: i32, error: anyhow::Error) -> anyhow::Error {
    anyhow::Error::new(ExitCode {
        code,
        error: Some(error),
    })
}

pub fn run() -> Result<i32> {
    run_with(Cli::parse())
}

fn run_with(cli: Cli) -> Result<i32> {
    match cli.command {
        Some(Command::Fmt(args)) => run_fmt(args),
        Some(Command::Diff(args)) => run_diff(args).map_err(|err| exit_with(2, err)),
        Some(Command::Set(args)) => run_set(args),
        Some(Command::Del(args)) => run_del(args),
        None => run_root(cli),
    }
}

fn parse_input(input: &[u8], filename: Option<&Path>) -> Result<YamNode> {
    if filename.map(is_json_file).unwrap_or(false) {
        parse_json(input)
    } else {
        parse_yaml(input)
    }
}

fn run_root(cli: Cli) -> Result<i32> {
    if cli.args.len() > 2 {
        bail!("accepts at most 2 positional arguments: [path] [file]");
    }
    let style = parse_tree_style(&cli.style)?;

    let (path_query, filename) = parse_root_args(&cli.args);
    let input = read_input(filename.as_deref())?;
    let root = parse_input(&input, filename.as_deref())?;

    // Document selection
    let selected: &YamNode = match cli.doc {
        None => &root,
        Some(n) => {
            let docs = documents(&root);
            if n < 1 || n > docs.len() {
                bail!(
                    "document {n} does not exist (file has {} document(s))",
                    docs.len()
                );
            }
            docs[n - 1]
        }
    };

    let hits: Vec<Hit> = match &path_query {
        Some(path) => run_query(selected, path)?,
        None => vec![Hit {
            node: selected,
            doc: 0,
            path: Vec::new(),
        }],
    };

    if cli.interactive {
        if hits.len() > 1 {
            bail!(
                "path matched {} nodes; interactive mode needs exactly one",
                hits.len()
            );
        }
        let display_name = filename
            .as_deref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "stdin".to_string());
        // Always open the whole file so saving never drops content; a path or
        // --doc only decides where the cursor starts.
        let focus = (path_query.is_some() || cli.doc.is_some()).then(|| {
            let doc = match cli.doc {
                Some(n) => n - 1,
                None => hits[0].doc,
            };
            (doc, hits[0].path.clone())
        });
        let json = filename.as_deref().map(is_json_file).unwrap_or(false);
        // `hits` and `selected` borrow `root`; they are not used past `focus`,
        // so the tree moves into the TUI instead of being copied.
        tui::run_tree(tui::TreeOptions {
            root,
            filename,
            display_name,
            json,
            style,
            show_types: cli.types,
            focus,
        })?;
        return Ok(0);
    }

    let multi_doc = root.kind == NodeKind::Stream && cli.doc.is_none();
    write_stdout(|out| {
        if cli.raw {
            for hit in &hits {
                for doc in documents(hit.node) {
                    writeln!(out, "{}", to_raw_value(doc))?;
                }
            }
        } else if cli.json {
            for hit in &hits {
                for doc in documents(hit.node) {
                    write_json_pretty(&mut *out, doc)?;
                    writeln!(out)?;
                }
            }
        } else if cli.yaml {
            for (i, hit) in hits.iter().enumerate() {
                if i > 0 {
                    writeln!(out, "---")?;
                }
                let text = format_yaml(hit.node, FormatOptions::default())
                    .map_err(|err| io::Error::other(format!("{err:#}")))?;
                out.write_all(text.as_bytes())?;
            }
        } else if cli.paths {
            for hit in &hits {
                for line in leaf_paths(hit.node) {
                    writeln!(out, "{line}")?;
                }
            }
        } else {
            let multi = hits.len() > 1;
            for hit in &hits {
                if multi {
                    let mut header = format!("# {}", format_path(&hit.path));
                    if multi_doc {
                        header.push_str(&format!("  (document {})", hit.doc + 1));
                    }
                    writeln!(out, "{header}")?;
                }
                renderer::render(out, hit.node, style, cli.types)?;
            }
        }
        Ok(())
    })?;
    Ok(0)
}

/// Evaluates a path; when nothing matches, reports the most specific error.
fn run_query<'a>(root: &'a YamNode, path: &str) -> Result<Vec<Hit<'a>>> {
    let segs = parse_path(path)?;
    let hits = query(root, path)?;
    if hits.is_empty() {
        if !is_multi(&segs) && documents(root).len() == 1 {
            if let Err(err) = get_by_path(root, path) {
                return Err(err.context("path query failed"));
            }
        }
        bail!("path query failed: no match for {path}");
    }
    Ok(hits)
}

fn run_fmt(args: FmtArgs) -> Result<i32> {
    if args.indent == 0 || args.indent > 16 {
        bail!(
            "invalid --indent {} (must be between 1 and 16)",
            args.indent
        );
    }
    let opts = FormatOptions {
        indent: args.indent,
        sort_keys: args.sort_keys,
    };

    let stdin = args.files.is_empty() || (args.files.len() == 1 && args.files[0] == Path::new("-"));
    if stdin {
        if args.write {
            bail!("cannot use -w with stdin input");
        }
        let input = read_input(None)?;
        let output = format_yaml(&parse_yaml(&input).context("invalid YAML")?, opts)?;
        if args.check {
            if input != output.as_bytes() {
                println!("stdin");
                return Ok(1);
            }
            return Ok(0);
        }
        write_stdout(|out| out.write_all(output.as_bytes()))?;
        return Ok(0);
    }
    if args.files.len() > 1 && !args.write && !args.check {
        bail!("formatting several files requires -w (write in place) or --check");
    }

    let (mut failed, mut unformatted) = (0, 0);
    for file in &args.files {
        let result = (|| -> Result<(Vec<u8>, String)> {
            let input = read_input(Some(file))?;
            let output = if is_json_file(file) {
                format_json(&parse_json(&input).context("invalid JSON")?, opts)?
            } else {
                format_yaml(&parse_yaml(&input).context("invalid YAML")?, opts)?
            };
            Ok((input, output))
        })();
        let (input, output) = match result {
            Ok(pair) => pair,
            Err(err) => {
                eprintln!("Error: {}: {err:#}", file.display());
                failed += 1;
                continue;
            }
        };
        let unchanged = input == output.as_bytes();
        if args.check {
            if !unchanged {
                println!("{}", file.display());
                unformatted += 1;
            }
        } else if args.write {
            if !unchanged {
                if let Err(err) = write_atomic(file, output.as_bytes()) {
                    eprintln!("Error: {err:#}");
                    failed += 1;
                }
            }
        } else {
            write_stdout(|out| out.write_all(output.as_bytes()))?;
        }
    }
    if failed > 0 {
        bail!("{failed} file(s) could not be formatted");
    }
    Ok(if unformatted > 0 { 1 } else { 0 })
}

fn run_diff(args: DiffArgs) -> Result<i32> {
    if args.file1 == Path::new("-") && args.file2 == Path::new("-") {
        bail!("only one side can be read from stdin");
    }
    let load = |file: &Path| -> Result<YamNode> {
        let name = (file != Path::new("-")).then_some(file);
        let input = read_input(name)?;
        parse_input(&input, name).with_context(|| format!("failed to parse {}", file.display()))
    };
    let left = load(&args.file1)?;
    let right = load(&args.file2)?;
    let opts = DiffOptions {
        path: args.path.clone(),
    };
    let mut result = diff::compare_with(&left, &right, &opts).context("invalid --path")?;
    result.left_file = args.file1.display().to_string();
    result.right_file = args.file2.display().to_string();

    if args.interactive {
        tui::run_diff(&result)?;
        return Ok(0);
    }

    if args.json {
        write_stdout(|out| {
            serde_json::to_writer_pretty(&mut *out, &diff::render_json(&result))?;
            writeln!(out)
        })?;
    } else if args.summary {
        println!("{}", diff::render_summary(&result.summary));
    } else if result.summary.total == 0 {
        println!("No differences found.");
    } else {
        write_stdout(|out| diff::render(out, &result))?;
    }

    Ok(if result.summary.total > 0 { 1 } else { 0 })
}

/// Reads and parses the input of `set`/`del`.
fn load_for_edit(edit: &EditArgs, file: Option<&Path>) -> Result<(YamNode, Vec<u8>)> {
    let file = file.filter(|f| *f != Path::new("-"));
    if edit.write && file.is_none() {
        bail!("cannot use -w with stdin input");
    }
    let input = read_input(file)?;
    let root = parse_input(&input, file)?;
    Ok((root, input))
}

/// Serializes an edited tree in the file's own format.
pub fn serialize(root: &YamNode, json: bool, indent: usize) -> Result<String> {
    if json {
        format_json(root, FormatOptions::default())
    } else {
        format_yaml(
            root,
            FormatOptions {
                indent,
                sort_keys: false,
            },
        )
    }
}

fn write_edited(
    edit: &EditArgs,
    file: Option<&Path>,
    root: &YamNode,
    original: &[u8],
) -> Result<i32> {
    let file = file.filter(|f| *f != Path::new("-"));
    let json = file.map(is_json_file).unwrap_or(false);
    let output = serialize(root, json, detect_indent(original))?;
    if edit.write {
        let file = file.expect("checked in load_for_edit");
        if !json && has_comments(original) {
            eprintln!(
                "warning: {}: YAML comments are not preserved",
                file.display()
            );
        }
        write_atomic(file, output.as_bytes())?;
    } else {
        write_stdout(|out| out.write_all(output.as_bytes()))?;
    }
    Ok(0)
}

fn run_set(args: SetArgs) -> Result<i32> {
    let segs = parse_path(&args.path).context("invalid path")?;
    if segs.is_empty() {
        bail!("cannot set the document root; give a path such as .key");
    }
    let value = parse_value(&args.value, args.string)?;
    let (mut root, original) = load_for_edit(&args.edit, args.file.as_deref())?;
    let is_stream = root.kind == NodeKind::Stream;
    // In multi-document files without --doc, change the documents that
    // already have the key; if none has it, the ones that have its parent.
    let selective = is_stream && args.edit.doc.is_none();
    let any_has_target = selective
        && documents(&root)
            .iter()
            .any(|doc| get_by_path(doc, &args.path).is_ok());
    let docs = target_documents(&mut root, args.edit.doc)?;
    let mut changed = 0;
    let mut last_err = None;
    for doc in docs {
        if selective {
            let eligible = if any_has_target {
                get_by_path(doc, &args.path).is_ok()
            } else {
                parent_exists(doc, &segs)
            };
            if !eligible {
                continue;
            }
        }
        match set_path(doc, &segs, value.clone()) {
            Ok(()) => changed += 1,
            Err(err) => last_err = Some(err),
        }
    }
    if changed == 0 {
        return Err(match last_err {
            Some(err) => err.context(format!("cannot set {}", args.path)),
            None => anyhow!(
                "cannot set {}: parent path not found in any document (use --doc N to create it)",
                args.path
            ),
        });
    }
    write_edited(&args.edit, args.file.as_deref(), &root, &original)
}

fn parent_exists(doc: &YamNode, segs: &[Segment]) -> bool {
    let parent: String = segs[..segs.len() - 1]
        .iter()
        .map(|s| s.to_string())
        .collect();
    parent.is_empty()
        || get_by_path(content(doc), &parent).is_ok()
        || get_by_path(doc, &parent).is_ok()
}

fn run_del(args: DelArgs) -> Result<i32> {
    let segs = parse_path(&args.path).context("invalid path")?;
    let (mut root, original) = load_for_edit(&args.edit, args.file.as_deref())?;
    let mut removed = 0;
    for doc in target_documents(&mut root, args.edit.doc)? {
        if delete_path(doc, &segs)? {
            removed += 1;
        }
    }
    if removed == 0 {
        bail!("path not found: {}", args.path);
    }
    write_edited(&args.edit, args.file.as_deref(), &root, &original)
}

fn has_comments(input: &[u8]) -> bool {
    String::from_utf8_lossy(input).lines().any(|line| {
        let t = line.trim_start();
        t.starts_with('#') || t.contains(" #")
    })
}

/// Guesses the indentation width of YAML content (default 2).
pub fn detect_indent(input: &[u8]) -> usize {
    let text = String::from_utf8_lossy(input);
    let best = text
        .lines()
        .filter_map(|line| {
            let n = line.len() - line.trim_start_matches(' ').len();
            let rest = &line[n..];
            (n > 0 && !rest.is_empty() && !rest.starts_with('#') && !rest.starts_with("- "))
                .then_some(n)
        })
        .min()
        .unwrap_or(2);
    if (2..=8).contains(&best) {
        best
    } else {
        2
    }
}

/// Streams output to stdout. A reader that stops early (`yam big.yaml | head`)
/// closes the pipe, which ends the output normally.
fn write_stdout(
    write: impl FnOnce(&mut BufWriter<StdoutLock<'static>>) -> io::Result<()>,
) -> Result<()> {
    let mut out = BufWriter::new(io::stdout().lock());
    ignore_broken_pipe(write(&mut out).and_then(|()| out.flush()))
}

fn ignore_broken_pipe(result: io::Result<()>) -> Result<()> {
    match result {
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => Ok(result?),
    }
}

/// `[path] [file]`, `[file]` or `[path]`. A single argument is a path when it
/// starts with '.' or '[' and is not an existing file (so `./config.yaml`
/// still opens the file). "-" means stdin.
fn parse_root_args(args: &[String]) -> (Option<String>, Option<PathBuf>) {
    let file = |name: &str| (name != "-").then(|| PathBuf::from(name));
    match args {
        [path, name] => (Some(path.clone()), file(name)),
        [single] if is_path_arg(single) => (Some(single.clone()), None),
        [single] => (None, file(single)),
        _ => (None, None),
    }
}

fn is_path_arg(arg: &str) -> bool {
    (arg.starts_with('.') && !Path::new(arg).is_file()) || arg.starts_with('[')
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

/// Writes through a temp file in the same directory and renames it into
/// place. Symlinks are followed and the original permissions are kept.
pub fn write_atomic(path: &Path, content: &[u8]) -> Result<()> {
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let dir = target
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = (0u32..)
        .map(|i| dir.join(format!(".{name}.yam-{}-{i}.tmp", std::process::id())))
        .find(|p| !p.exists())
        .expect("a free temp name");
    let permissions = fs::metadata(&target).map(|m| m.permissions()).ok();
    let result = (|| -> Result<()> {
        fs::write(&tmp, content)
            .with_context(|| format!("failed to create temp file: {}", tmp.display()))?;
        if let Some(permissions) = permissions {
            fs::set_permissions(&tmp, permissions).context("failed to set permissions")?;
        }
        fs::rename(&tmp, &target)
            .with_context(|| format!("failed to write file: {}", path.display()))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn parse_tree_style(style: &str) -> Result<TreeStyle> {
    match style {
        "unicode" | "" => Ok(TreeStyle::Unicode),
        "ascii" => Ok(TreeStyle::Ascii),
        "indent" => Ok(TreeStyle::Indent),
        _ => bail!("invalid style {style:?} (valid: unicode, ascii, indent)"),
    }
}

#[cfg(test)]
mod tests {
    use super::{detect_indent, ignore_broken_pipe, parse_root_args, write_atomic};
    use std::fs;
    use std::io;
    use std::path::PathBuf;

    #[test]
    fn only_a_closed_pipe_is_ignored() {
        assert!(ignore_broken_pipe(Ok(())).is_ok());
        assert!(ignore_broken_pipe(Err(io::ErrorKind::BrokenPipe.into())).is_ok());
        for kind in [
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::WriteZero,
            io::ErrorKind::Interrupted,
        ] {
            let err = ignore_broken_pipe(Err(io::Error::new(kind, "disk full"))).unwrap_err();
            assert_eq!(err.to_string(), "disk full");
        }
    }

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
        assert_eq!(parse_root_args(&["-".to_string()]), (None, None));
        assert_eq!(
            parse_root_args(&["./Cargo.toml".to_string()]),
            (None, Some(PathBuf::from("./Cargo.toml"))),
            "an existing ./file is a file, not a path"
        );
    }

    #[test]
    fn detects_indentation() {
        assert_eq!(detect_indent(b"a:\n    b: 1\n"), 4);
        assert_eq!(detect_indent(b"a:\n  b: 1\n"), 2);
        assert_eq!(detect_indent(b"a: 1\n"), 2);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_keeps_permissions_and_symlinks() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = std::env::temp_dir().join(format!("yam-atomic-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("real.yaml");
        let link = dir.join("link.yaml");
        fs::write(&target, "a: 1\n").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        let _ = fs::remove_file(&link);
        symlink(&target, &link).unwrap();

        write_atomic(&link, b"a: 2\n").unwrap();

        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read_to_string(&target).unwrap(), "a: 2\n");
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let leftovers = fs::read_dir(&dir).unwrap().count();
        assert_eq!(leftovers, 2, "no temp files left behind");
        let _ = fs::remove_dir_all(&dir);
    }
}
