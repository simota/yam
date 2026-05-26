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
| **Interactive TUI** | Navigate, fold/unfold, search with vim-like keybindings |
| **Inline Editing** | Edit scalar values directly in TUI mode |
| **Path Extraction** | Query values with JSONPath-like syntax |
| **JSON Support** | Bidirectional YAML/JSON conversion |
| **Formatting** | Format YAML files with consistent styling |
| **Diff** | Structural comparison between YAML/JSON files |

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

# Output as JSON
yam --json config.yaml

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
  -r, --raw            Output raw value without decoration
  -h, --help           Help for yam
  -v, --version        Version for yam
```

### Subcommands

#### `yam fmt` - Format YAML files

```
yam fmt [flags] [file]

Flags:
  -w, --write        Write result to source file instead of stdout
  -i, --indent int   Indentation width in spaces (default 2)
  -s, --sort-keys    Sort keys alphabetically
```

#### `yam diff` - Compare YAML/JSON files

```
yam diff [flags] <file1> <file2>

Flags:
  -i, --interactive   Interactive TUI mode with split view
  -s, --summary       Show only summary (no detailed diff)
```

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

### Editing

| Key | Action |
|-----|--------|
| `e` | Edit value |
| `Enter` | Confirm edit |
| `Esc` | Cancel edit |
| `Ctrl+s` | Save file |
| `u` | Undo |
| `Ctrl+r` | Redo |

### Diff TUI

| Key | Action |
|-----|--------|
| `n` / `]` | Next diff |
| `N` / `[` | Previous diff |
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
