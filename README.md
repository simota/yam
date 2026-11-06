# yam

[![Rust](https://img.shields.io/badge/Rust-1.75+-000000?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**A beautiful YAML viewer for the terminal.**

Tree visualization, syntax highlighting, interactive TUI mode with inline editing.

![yam demo](demo.gif)

## Features

| Feature | Description |
|---------|-------------|
| **Tree View** | Beautiful tree visualization (Unicode/ASCII/Indent styles) |
| **Type Annotations** | Optional scalar type labels |
| **Interactive TUI** | Navigate, fold/unfold, search, jump to a path, copy paths/values |
| **Structural Editing** | Edit values, rename keys, add and delete entries in the TUI, with undo/redo |
| **Path Queries** | Wildcards (`[*]`), negative indexes, quoted keys and `..` recursive search |
| **Multi-Document** | `---` separated files (e.g. Kubernetes manifests) everywhere |
| **Scripted Edits** | `yam set` / `yam del` change files from the command line |
| **JSON Support** | Bidirectional YAML/JSON conversion with key order kept and merge keys applied |
| **Formatting** | Format YAML/JSON files in place, or check formatting in CI |
| **Diff** | Structural comparison with smart list/document matching and JSON output |

## Installation

```bash
# Cargo install from a local checkout
cargo install --path .

# From source
git clone https://github.com/simota/yam.git
cd yam && cargo build --release
```

## Quick Start

```bash
# View a YAML file
yam config.yaml

# Interactive TUI mode
yam -i config.yaml

# Pipe from stdin
cat config.yaml | yam

# Extract a value
yam '.metadata.name' config.yaml

# Every container image in every document of a manifest
yam -r '..image' k8s.yaml

# Output as JSON / convert JSON to YAML
yam --json config.yaml
yam --yaml data.json

# Change a value in place
yam set -w '.spec.replicas' 3 deploy.yaml

# Format a YAML file
yam fmt config.yaml

# Format in-place
yam fmt -w config.yaml

# Compare two files
yam diff config-dev.yaml config-prod.yaml
```

## Usage

```
yam [flags] [path] [file]

Flags:
  -i, --interactive    Interactive TUI mode
  -s, --style string   Tree style: unicode, ascii, indent (default "unicode")
  -t, --types          Show type annotations
  -j, --json           Output as JSON
  -y, --yaml           Output as YAML (e.g. convert JSON to YAML)
  -p, --paths          List every leaf value as 'path = value'
  -r, --raw            Output raw value without decoration
  -d, --doc N          Select document N (1-based) of a multi-document file
  -h, --help           Help for yam
  -V, --version        Version for yam
```

Use `-` as the file name to read stdin. `.json`, `.jsonl` and `.ndjson` files
are read as JSON (several concatenated values become several documents).
Errors print their cause and exit with code 1.

### Path syntax

| Syntax | Meaning |
|--------|---------|
| `.` | The whole document |
| `.key.sub` | Nested mapping keys |
| `.items[0]` / `.items[-1]` | Array element (negative indexes count from the end) |
| `.items[*].name` | `name` of every element |
| `.data.*` | Every value of a mapping |
| `.["a.b"]` / `.['x y']` | Keys containing dots, spaces or brackets |
| `..image` | `image` at any depth |

Paths are evaluated against every document of a multi-document file, so
`yam -r .kind k8s.yaml` prints the kind of each resource. `<<` merge keys are
applied. When a path matches several nodes, each result is printed under a
header with its own path. `yam --paths` prints every leaf as `path = value`,
handy for `grep` and for finding the path to pass to `yam set`.

### Subcommands

#### `yam fmt` - Format YAML files

```
yam fmt [flags] [file...]

Flags:
  -w, --write        Write result to source files instead of stdout
  -c, --check        List files that need formatting and exit 1 (for CI)
  -i, --indent int   Indentation width in spaces (default 2)
  -s, --sort-keys    Sort keys alphabetically
```

All documents of a multi-document file are kept. JSON files are formatted as
indented JSON with key order kept. Files that are already formatted are not
rewritten; writes are atomic and keep file permissions and symlinks.

#### `yam set` / `yam del` - Edit from the command line

```
yam set [flags] <path> <value> [file]
yam del [flags] <path> [file]

Flags:
  -w, --write      Write result back to the file instead of stdout
  -d, --doc N      Only change document N of a multi-document file
  -s, --string     (set) Store the value as a string
```

The value is parsed as YAML (`3` → number, `true` → bool, `'[a, b]'` → list,
`'{k: v}'` → mapping). Missing parent keys are created; setting index N on a
list of length N appends. In multi-document files `set` changes the documents
that already have the key (or, if none does, those that have its parent) and
`del` removes the path wherever it exists. JSON files stay JSON.

> **Note:** like `fmt` and saving from the TUI, `set -w`/`del -w` re-serialize
> the file, so YAML comments are not preserved (a warning is printed).

#### `yam diff` - Compare YAML/JSON files

```
yam diff [flags] <file1> <file2>

Flags:
  -i, --interactive   Interactive TUI mode with split view
  -s, --summary       Show only summary (no detailed diff)
  -j, --json          Output changes as JSON
  -p, --path PATH     Only compare the subtree at this path
```

- Counts reflect actual changes: a changed value deep in the tree is one modification.
- Multi-document files are compared per document; Kubernetes-style resources are
  matched by `Kind/namespace/name`, so reordered manifests do not produce noise.
- Lists of mappings with a unique `name`, `id` or `key` field are matched by that
  field; other lists are aligned so that one inserted item is one addition.
- Use `-` for one side to read stdin. Exit codes: `0` no differences,
  `1` differences, `2` error.

## Examples

### View Kubernetes ConfigMap

```bash
kubectl get configmap my-config -o yaml | yam -i
```

### Extract nested value

```bash
yam '.spec.containers[0].image' deployment.yaml
```

### Convert YAML to JSON

```bash
yam --json values.yaml > values.json
```

### Show type annotations

```bash
yam -t config.yaml
```

### Format YAML with sorted keys

```bash
yam fmt --sort-keys config.yaml > config-sorted.yaml
```

### Compare dev and prod configs

```bash
yam diff config-dev.yaml config-prod.yaml
```

### Interactive diff with split view

```bash
yam diff -i config-dev.yaml config-prod.yaml
```

### Check rendered manifests against what is deployed

```bash
helm template . | yam diff deployed.yaml -
yam diff --json old.yaml new.yaml | jq '.changes[] | select(.type=="modified")'
```

### Fail CI when files are not formatted

```bash
yam fmt --check $(git ls-files '*.yaml')
```

## TUI Keybindings

### Navigation

| Key | Action |
|-----|--------|
| `j` / `↓` | Move down |
| `k` / `↑` | Move up |
| `g` / `Home` | Go to top |
| `G` / `End` | Go to bottom |
| `Ctrl+d` | Half page down |
| `Ctrl+u` | Half page up |

### Folding

| Key | Action |
|-----|--------|
| `Enter` / `o` | Toggle fold |
| `O` | Expand all |
| `C` | Collapse all |

### Search

| Key | Action |
|-----|--------|
| `/` | Start search |
| `n` | Next match |
| `N` | Previous match |
| `Esc` | Cancel search |
| `:` | Go to path (e.g. `.spec.containers[0]`) |

### Editing

| Key | Action |
|-----|--------|
| `e` | Edit value (strings stay strings; other values are re-typed, e.g. `true` → bool) |
| `r` | Rename key |
| `a` | Add entry (`key: value` in a mapping, a value in a list) |
| `D` | Delete node |
| `Enter` / `Esc` | Confirm / cancel the prompt |
| `Ctrl+s` | Save file (JSON stays JSON; all documents kept) |
| `u` / `Ctrl+r` | Undo / redo any change (up to 100 steps) |
| `y` / `Y` | Copy path / value to the clipboard (OSC 52, works over SSH) |

`yam -i '.spec' file.yaml` (or `--doc N`) opens the whole file with the cursor
on that node, so saving never drops the rest of the file. Multi-document files
show one row per document; `C` collapses them into an overview.

### Diff TUI

| Key | Action |
|-----|--------|
| `n` / `]` | Next diff |
| `N` / `[` | Previous diff |
| `c` | Show only changed parts |
| `q` / `Esc` | Quit |

### Other

| Key | Action |
|-----|--------|
| `?` | Toggle help |
| `q` | Quit |

## Built With

- [clap](https://docs.rs/clap/) - CLI parser
- [crossterm](https://docs.rs/crossterm/) - Terminal UI
- [serde_yaml](https://docs.rs/serde_yaml/) - YAML parser/emitter
- [serde_json](https://docs.rs/serde_json/) - JSON parser/emitter

## License

MIT License - see [LICENSE](LICENSE) for details.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.
