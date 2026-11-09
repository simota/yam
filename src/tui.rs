use crate::cli::{serialize, write_atomic};
use crate::diff::{display_value, DiffNode, DiffResult, DiffType};
use crate::format::{format_yaml, FormatOptions};
use crate::parser::{content, documents, NodeKind, YamNode};
use crate::path::{format_path, parse_value, query, renumber, retag_scalar, string_node, PathPart};
use crate::renderer::{document_label, TreeStyle};
use anyhow::Result;
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, queue};
use std::collections::HashSet;
use std::io::{self, Write};
use std::path::PathBuf;

const MAX_UNDO: usize = 100;

/// A node's place in the preorder walk of the viewed tree. Its index in
/// `TreeApp::nodes` is the node's id. Ids are recomputed after structural
/// edits (add, delete); fold state is carried over by path.
#[derive(Debug, Clone, Copy)]
struct NodeMeta {
    parent: Option<usize>,
    child_index: usize,
    /// Id one past the last node of this subtree.
    end: usize,
    /// Number of ancestors.
    level: usize,
}

/// One undo/redo state: the whole tree plus the edited-node highlights.
#[derive(Debug, Clone)]
struct Snapshot {
    root: YamNode,
    modified: HashSet<String>,
    cursor: Option<String>,
    version: u64,
    desc: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputMode {
    Normal,
    Search,
    Edit,
    Rename,
    Add,
    GoTo,
}

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

/// What the interactive viewer opens.
pub struct TreeOptions {
    /// The whole file (a document or a stream of documents).
    pub root: YamNode,
    /// File to save to; `None` makes the session read-only (stdin).
    pub filename: Option<PathBuf>,
    pub display_name: String,
    /// Save as JSON instead of YAML.
    pub json: bool,
    pub style: TreeStyle,
    pub show_types: bool,
    /// Document index and path of the node to start on.
    pub focus: Option<(usize, Vec<PathPart>)>,
}

pub fn run_tree(opts: TreeOptions) -> Result<()> {
    let _guard = TerminalGuard::enter()?;
    TreeApp::new(opts).run()
}

pub fn run_diff(result: &DiffResult<'_>) -> Result<()> {
    let _guard = TerminalGuard::enter()?;
    DiffApp::new(result).run()
}

struct TreeApp {
    root: YamNode,
    filename: Option<PathBuf>,
    display_name: String,
    json: bool,
    indent: usize,
    style: TreeStyle,
    show_types: bool,
    nodes: Vec<NodeMeta>,
    /// Ids of the visible nodes in preorder; rebuilt only when folding changes.
    rows: Vec<usize>,
    collapsed: HashSet<usize>,
    cursor: usize,
    offset: usize,
    show_help: bool,
    mode: InputMode,
    input: String,
    /// Node the open prompt applies to.
    target: Option<usize>,
    /// Keys (see `node_key`) of nodes changed since the last save.
    modified: HashSet<String>,
    version: u64,
    next_version: u64,
    saved_version: u64,
    quit_armed: bool,
    status: String,
    /// Ids of the search hits in ascending order.
    matches: Vec<usize>,
    match_index: usize,
    undo_stack: Vec<Snapshot>,
    redo_stack: Vec<Snapshot>,
    /// Text waiting to be sent to the terminal clipboard (OSC 52).
    pending_copy: Option<String>,
    #[cfg(test)]
    row_builds: usize,
}

impl TreeApp {
    fn new(opts: TreeOptions) -> Self {
        let mut app = Self {
            root: opts.root,
            filename: opts.filename,
            display_name: opts.display_name,
            json: opts.json,
            indent: 2,
            style: opts.style,
            show_types: opts.show_types,
            nodes: Vec::new(),
            rows: Vec::new(),
            collapsed: HashSet::new(),
            cursor: 0,
            offset: 0,
            show_help: false,
            mode: InputMode::Normal,
            input: String::new(),
            target: None,
            modified: HashSet::new(),
            version: 0,
            next_version: 0,
            saved_version: 0,
            quit_armed: false,
            status: String::new(),
            matches: Vec::new(),
            match_index: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            pending_copy: None,
            #[cfg(test)]
            row_builds: 0,
        };
        if let Some(path) = &app.filename {
            if let Ok(input) = std::fs::read(path) {
                app.indent = crate::cli::detect_indent(&input);
            }
        }
        app.reindex();
        if let Some((doc, parts)) = opts.focus {
            if let Some(id) = app.id_for(doc, &parts) {
                app.jump_to(id);
            }
        }
        app
    }

    fn dirty(&self) -> bool {
        self.version != self.saved_version
    }

    fn is_stream(&self) -> bool {
        self.root.kind == NodeKind::Stream
    }

    fn run(&mut self) -> Result<()> {
        loop {
            self.draw()?;
            // Block until the next event; any event, a resize included, redraws.
            if let Event::Key(key) = event::read()? {
                if self.handle_key(key)? {
                    break;
                }
            }
        }
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        if self.mode != InputMode::Normal {
            self.quit_armed = false;
            self.handle_prompt_key(key);
            return Ok(false);
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let is_quit_key = matches!(key.code, KeyCode::Char('q'))
            || (ctrl && matches!(key.code, KeyCode::Char('c')));
        if !is_quit_key {
            self.quit_armed = false;
        }
        self.status.clear();

        match key.code {
            _ if is_quit_key => {
                if self.dirty() && !self.quit_armed {
                    self.status =
                        "Unsaved changes! Press q again to discard, or Ctrl+S to save".into();
                    self.quit_armed = true;
                    return Ok(false);
                }
                return Ok(true);
            }
            KeyCode::Char('u') if ctrl => {
                self.move_cursor(-((self.viewport_height() / 2) as isize))
            }
            KeyCode::Char('d') if ctrl => self.move_cursor((self.viewport_height() / 2) as isize),
            KeyCode::Char('s') if ctrl => self.save_file()?,
            KeyCode::Char('r') if ctrl => self.redo(),
            _ if ctrl => {}
            KeyCode::Char('?') => self.show_help = !self.show_help,
            KeyCode::Up | KeyCode::Char('k') => self.move_cursor(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_cursor(1),
            KeyCode::PageUp | KeyCode::Char('b') => {
                self.move_cursor(-(self.viewport_height() as isize))
            }
            KeyCode::PageDown | KeyCode::Char('f') | KeyCode::Char(' ') => {
                self.move_cursor(self.viewport_height() as isize)
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.cursor = 0;
                self.offset = 0;
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.cursor = self.rows.len().saturating_sub(1);
                self.adjust_offset();
            }
            KeyCode::Enter | KeyCode::Tab | KeyCode::Char('o') => self.toggle_current(),
            KeyCode::Char('O') => {
                self.collapsed.clear();
                self.rebuild_rows();
            }
            KeyCode::Char('C') => self.collapse_all(),
            KeyCode::Char('/') => {
                self.open_prompt(InputMode::Search, String::new(), None);
                self.matches.clear();
            }
            KeyCode::Char(':') => {
                let start = self
                    .current_id()
                    .filter(|&id| self.node(id).kind != NodeKind::Document && id != 0)
                    .map(|id| format_path(&self.location(id).1))
                    .unwrap_or_default();
                self.open_prompt(InputMode::GoTo, start, None);
            }
            KeyCode::Char('n') => self.next_match(),
            KeyCode::Char('N') => self.prev_match(),
            KeyCode::Char('e') => self.start_edit(),
            KeyCode::Char('r') => self.start_rename(),
            KeyCode::Char('a') => self.start_add(),
            KeyCode::Char('D') | KeyCode::Delete => self.delete_current(),
            KeyCode::Char('y') => self.yank_path(),
            KeyCode::Char('Y') => self.yank_value(),
            KeyCode::Char('u') => self.undo(),
            _ => {}
        }
        Ok(false)
    }

    fn open_prompt(&mut self, mode: InputMode, value: String, target: Option<usize>) {
        self.mode = mode;
        self.input = value;
        self.target = target;
    }

    fn handle_prompt_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => {
                if self.mode == InputMode::Search {
                    self.matches.clear();
                }
                self.mode = InputMode::Normal;
                self.input.clear();
                self.target = None;
                return;
            }
            KeyCode::Enter => {
                let mode = self.mode;
                let input = std::mem::take(&mut self.input);
                let target = self.target.take();
                self.mode = InputMode::Normal;
                match mode {
                    InputMode::Search => self.finish_search(),
                    InputMode::Edit => self.confirm_edit(target, &input),
                    InputMode::Rename => self.confirm_rename(target, &input),
                    InputMode::Add => self.confirm_add(target, &input),
                    InputMode::GoTo => self.go_to(&input),
                    InputMode::Normal => {}
                }
                return;
            }
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char('u') if ctrl => self.input.clear(),
            KeyCode::Char(ch) if !ctrl => self.input.push(ch),
            _ => return,
        }
        if self.mode == InputMode::Search {
            self.update_search_matches();
        }
    }

    fn draw(&mut self) -> Result<()> {
        let (width, height) = terminal::size()?;
        self.clamp_cursor();
        self.adjust_offset();

        let mut stdout = io::stdout();
        if let Some(text) = self.pending_copy.take() {
            // OSC 52: ask the terminal to put text on the clipboard. Works
            // over SSH in terminals that support it.
            queue!(
                stdout,
                Print(format!("\x1b]52;c;{}\x07", base64(text.as_bytes())))
            )?;
        }
        queue!(stdout, MoveTo(0, 0), Clear(ClearType::All))?;

        let mut header = format!(" yam - {}", self.display_name);
        if self.is_stream() {
            header.push_str(&format!(" ({} documents)", self.root.children.len()));
        }
        if self.dirty() {
            header.push_str(" [modified]");
        }
        if self.filename.is_none() {
            header.push_str(" [read-only]");
        }
        draw_bar(&mut stdout, 0, width, &header, Color::Blue)?;

        let viewport = self
            .viewport_height()
            .min(height.saturating_sub(3) as usize);
        for line_idx in 0..viewport {
            let row_idx = self.offset + line_idx;
            queue!(stdout, MoveTo(0, (line_idx + 1) as u16))?;
            let Some(&id) = self.rows.get(row_idx) else {
                continue;
            };
            if row_idx == self.cursor {
                queue!(stdout, SetAttribute(Attribute::Reverse))?;
            } else if self.modified.contains(&self.node_key(id)) {
                queue!(stdout, SetForegroundColor(Color::Yellow))?;
            } else if self.is_match(id) {
                queue!(stdout, SetForegroundColor(Color::DarkYellow))?;
            }
            queue!(
                stdout,
                Print(truncate(&self.render_node_line(id), width as usize))
            )?;
            queue!(stdout, ResetColor, SetAttribute(Attribute::Reset))?;
        }

        let footer_y = height.saturating_sub(2);
        let footer = self.footer_text();
        draw_bar(&mut stdout, footer_y, width, &footer, Color::DarkGrey)?;

        queue!(stdout, MoveTo(0, height.saturating_sub(1)))?;
        let help = if self.show_help {
            "j/k move  PgUp/PgDn  g/G top/end  Enter/o fold  O/C expand/collapse all  / n N search  : go to  e edit  r rename  a add  D delete  y/Y copy path/value  u/Ctrl+R undo/redo  Ctrl+S save  q quit"
        } else {
            "j/k move  Enter fold  e edit  a add  D delete  / search  : go to  ? help  q quit"
        };
        queue!(stdout, Print(truncate(help, width as usize)))?;
        stdout.flush()?;
        Ok(())
    }

    fn footer_text(&self) -> String {
        match self.mode {
            InputMode::Search => {
                let suffix = if self.input.is_empty() {
                    String::new()
                } else if self.matches.is_empty() {
                    "  [no matches]".to_string()
                } else {
                    format!("  [{} matches]", self.matches.len())
                };
                format!("/{}{suffix}", self.input)
            }
            InputMode::Edit => format!("Edit: {}  [Enter: confirm, Esc: cancel]", self.input),
            InputMode::Rename => {
                format!("Rename key: {}  [Enter: confirm, Esc: cancel]", self.input)
            }
            InputMode::Add => {
                let is_seq = self
                    .target
                    .is_some_and(|id| self.node(id).kind == NodeKind::Sequence);
                let label = if is_seq {
                    "Add item"
                } else {
                    "Add (key: value)"
                };
                format!("{label}: {}  [Enter: confirm, Esc: cancel]", self.input)
            }
            InputMode::GoTo => format!("Go to: {}  [Enter: confirm, Esc: cancel]", self.input),
            InputMode::Normal if !self.status.is_empty() => self.status.clone(),
            InputMode::Normal => {
                let pos = format!("{}/{}", self.cursor.saturating_add(1), self.rows.len());
                let mut text = match self.current_id() {
                    Some(id) => format!("{pos} | {}", self.describe(id)),
                    None => pos,
                };
                if !self.matches.is_empty() {
                    text.push_str(&format!(
                        "  [match {}/{}]",
                        self.match_index.saturating_add(1),
                        self.matches.len()
                    ));
                }
                if self.dirty() {
                    text.push_str("  [modified - Ctrl+S to save]");
                }
                text
            }
        }
    }

    /// Footer description of a node: its path (and document) and type.
    fn describe(&self, id: usize) -> String {
        let node = self.node(id);
        match node.kind {
            NodeKind::Stream => format!("{} documents", node.children.len()),
            NodeKind::Document => document_label(node),
            _ => {
                let mut text = self.display_path(id);
                if self.is_stream() {
                    text.push_str(&format!(" (doc {})", self.location(id).0 + 1));
                }
                if node.kind == NodeKind::Scalar {
                    text.push_str(&format!(" <{}>", node.type_label()));
                }
                text
            }
        }
    }

    // ---- tree indexing -------------------------------------------------------

    /// Re-numbers the tree after a change and rebuilds the visible rows.
    fn reindex(&mut self) {
        self.nodes.clear();
        index_nodes(view_root(&self.root), None, 0, 0, &mut self.nodes);
        self.rebuild_rows();
    }

    fn rebuild_rows(&mut self) {
        #[cfg(test)]
        {
            self.row_builds += 1;
        }
        self.rows.clear();
        let mut id = 0;
        while id < self.nodes.len() {
            self.rows.push(id);
            id = if self.collapsed.contains(&id) {
                self.nodes[id].end
            } else {
                id + 1
            };
        }
    }

    /// Child indices leading from the viewed root to `id`.
    fn child_indices(&self, mut id: usize) -> Vec<usize> {
        let mut indices = Vec::new();
        while let Some(parent) = self.nodes[id].parent {
            indices.push(self.nodes[id].child_index);
            id = parent;
        }
        indices.reverse();
        indices
    }

    fn node(&self, id: usize) -> &YamNode {
        self.child_indices(id)
            .into_iter()
            .fold(view_root(&self.root), |node, idx| &node.children[idx])
    }

    fn node_mut(&mut self, id: usize) -> &mut YamNode {
        let indices = self.child_indices(id);
        indices
            .into_iter()
            .fold(view_root_mut(&mut self.root), |node, idx| {
                &mut node.children[idx]
            })
    }

    /// Document index and path (relative to that document) of a node.
    fn location(&self, id: usize) -> (usize, Vec<PathPart>) {
        let mut doc = 0;
        let mut parts = Vec::new();
        let mut node = view_root(&self.root);
        for idx in self.child_indices(id) {
            match node.kind {
                NodeKind::Stream => doc = idx,
                NodeKind::Mapping => parts.push(PathPart::Key(
                    node.children[idx].key.clone().unwrap_or_default(),
                )),
                NodeKind::Sequence => parts.push(PathPart::Index(idx)),
                _ => {}
            }
            node = &node.children[idx];
        }
        (doc, parts)
    }

    fn display_path(&self, id: usize) -> String {
        format_path(&self.location(id).1)
    }

    /// Identifies a node across re-indexing.
    fn node_key(&self, id: usize) -> String {
        let (doc, parts) = self.location(id);
        match self.node(id).kind {
            NodeKind::Stream => "stream".into(),
            NodeKind::Document => format!("{doc}:doc"),
            _ => format!("{doc}:{}", format_path(&parts)),
        }
    }

    /// Finds the id of the node at a document and path.
    fn id_for(&self, doc: usize, parts: &[PathPart]) -> Option<usize> {
        let mut id = 0;
        if self.is_stream() {
            id = self.child_id(0, doc)?; // document
            id = self.child_id(id, 0)?; // its content
        }
        for part in parts {
            let node = self.node(id);
            let pos = match part {
                PathPart::Index(i) => *i,
                PathPart::Key(k) => node
                    .children
                    .iter()
                    .position(|c| c.key.as_deref() == Some(k))?,
            };
            id = self.child_id(id, pos)?;
        }
        Some(id)
    }

    fn id_for_key(&self, key: &str) -> Option<usize> {
        if key == "stream" {
            return Some(0);
        }
        let (doc, rest) = key.split_once(':')?;
        let doc: usize = doc.parse().ok()?;
        if rest == "doc" {
            return self.child_id(0, doc);
        }
        let parts = parse_parts(rest)?;
        self.id_for(doc, &parts)
    }

    /// Id of the `pos`-th child of `id`.
    fn child_id(&self, id: usize, pos: usize) -> Option<usize> {
        let end = self.nodes[id].end;
        let mut child = id + 1;
        for _ in 0..pos {
            if child >= end {
                return None;
            }
            child = self.nodes[child].end;
        }
        (child < end).then_some(child)
    }

    fn render_node_line(&self, id: usize) -> String {
        let node = self.node(id);
        let level = self.nodes[id].level;
        let indent = "  ".repeat(level.saturating_sub(1));
        let indicator = if node.has_children() {
            let (collapsed, expanded) = match self.style {
                TreeStyle::Unicode => ("▶ ", "▼ "),
                TreeStyle::Ascii | TreeStyle::Indent => ("+ ", "- "),
            };
            if self.collapsed.contains(&id) {
                collapsed
            } else {
                expanded
            }
        } else {
            "  "
        };
        let parent_kind = self.nodes[id].parent.map(|p| self.node(p).kind);
        let name = match (&node.key, parent_kind) {
            (Some(key), Some(NodeKind::Mapping)) => format!("{key}:"),
            (_, Some(NodeKind::Sequence)) => format!("[{}]", node.index),
            _ => String::new(),
        };
        let value = match node.kind {
            NodeKind::Stream => format!("{} documents", node.children.len()),
            NodeKind::Document => format!("--- {}", document_label(node)),
            NodeKind::Scalar => {
                let mut rendered = node.value.clone().unwrap_or_default();
                if self.show_types {
                    rendered.push_str(&format!(" <{}>", node.type_label()));
                }
                rendered
            }
            NodeKind::Mapping | NodeKind::Sequence => {
                let collapsed = self.collapsed.contains(&id);
                let is_top = name.is_empty();
                match node.kind {
                    _ if node.children.is_empty() => if node.kind == NodeKind::Mapping {
                        "{}"
                    } else {
                        "[]"
                    }
                    .to_string(),
                    NodeKind::Mapping if collapsed || is_top => {
                        format!("{{{} keys}}", node.children.len())
                    }
                    NodeKind::Sequence if collapsed || is_top => {
                        format!("[{} items]", node.children.len())
                    }
                    _ => String::new(),
                }
            }
        };
        let sep = if name.is_empty() || value.is_empty() {
            ""
        } else {
            " "
        };
        format!("{indent}{indicator}{name}{sep}{value}")
    }

    fn is_match(&self, id: usize) -> bool {
        self.matches.binary_search(&id).is_ok()
    }

    fn move_cursor(&mut self, delta: isize) {
        let len = self.rows.len();
        if len == 0 {
            self.cursor = 0;
            self.offset = 0;
            return;
        }
        self.cursor =
            (self.cursor as isize + delta).clamp(0, len.saturating_sub(1) as isize) as usize;
        self.adjust_offset();
    }

    fn clamp_cursor(&mut self) {
        let len = self.rows.len();
        if len == 0 {
            self.cursor = 0;
        } else if self.cursor >= len {
            self.cursor = len - 1;
        }
    }

    fn adjust_offset(&mut self) {
        let vh = self.viewport_height();
        if self.cursor < self.offset {
            self.offset = self.cursor;
        } else if self.cursor >= self.offset + vh {
            self.offset = self.cursor.saturating_sub(vh.saturating_sub(1));
        }
    }

    fn viewport_height(&self) -> usize {
        terminal::size()
            .map(|(_, h)| viewport_rows(h))
            .unwrap_or(20)
    }

    fn current_id(&self) -> Option<usize> {
        self.rows.get(self.cursor).copied()
    }

    fn toggle_current(&mut self) {
        let Some(id) = self.current_id() else {
            return;
        };
        let node = self.node(id);
        if !node.is_container() || !node.has_children() {
            return;
        }
        if !self.collapsed.remove(&id) {
            self.collapsed.insert(id);
        }
        self.rebuild_rows();
    }

    fn collapse_all(&mut self) {
        // A subtree with more than one node is a container with children; the
        // viewed root (id 0) stays expanded. Documents of a stream collapse
        // too, which gives an overview of the documents.
        self.collapsed = (1..self.nodes.len())
            .filter(|&id| self.nodes[id].end > id + 1)
            .collect();
        self.cursor = 0;
        self.offset = 0;
        self.rebuild_rows();
    }

    // ---- editing -------------------------------------------------------------

    fn check_writable(&mut self) -> bool {
        if self.filename.is_none() {
            self.status = "Cannot edit: read-only (stdin)".into();
            return false;
        }
        true
    }

    fn start_edit(&mut self) {
        let Some(id) = self.current_id() else {
            return;
        };
        if self.node(id).kind != NodeKind::Scalar {
            self.status = "Cannot edit: not a scalar value (a: add, D: delete)".into();
            return;
        }
        if !self.check_writable() {
            return;
        }
        let value = self.node(id).value.clone().unwrap_or_default();
        self.open_prompt(InputMode::Edit, value, Some(id));
    }

    fn start_rename(&mut self) {
        let Some(id) = self.current_id() else {
            return;
        };
        let in_mapping = self.nodes[id]
            .parent
            .is_some_and(|p| self.node(p).kind == NodeKind::Mapping);
        if !in_mapping {
            self.status = "Cannot rename: not a mapping key".into();
            return;
        }
        if !self.check_writable() {
            return;
        }
        let key = self.node(id).key.clone().unwrap_or_default();
        self.open_prompt(InputMode::Rename, key, Some(id));
    }

    /// Where "add" inserts: the container under the cursor (appending), or
    /// right after the current entry in its parent.
    fn add_target(&self) -> Option<(usize, Option<usize>)> {
        let id = self.current_id()?;
        let node = self.node(id);
        let is_container = matches!(node.kind, NodeKind::Mapping | NodeKind::Sequence);
        if node.kind == NodeKind::Document {
            let c = self.child_id(id, 0)?;
            return matches!(self.node(c).kind, NodeKind::Mapping | NodeKind::Sequence)
                .then_some((c, None));
        }
        let parent = self.nodes[id].parent;
        let parent_is_container = parent
            .is_some_and(|p| matches!(self.node(p).kind, NodeKind::Mapping | NodeKind::Sequence));
        if is_container
            && (!parent_is_container || !self.collapsed.contains(&id) || node.children.is_empty())
        {
            return Some((id, None));
        }
        if parent_is_container {
            return Some((parent?, Some(self.nodes[id].child_index + 1)));
        }
        None
    }

    fn start_add(&mut self) {
        if !self.check_writable() {
            return;
        }
        match self.add_target() {
            Some((container, _)) => {
                self.open_prompt(InputMode::Add, String::new(), Some(container))
            }
            None => self.status = "Cannot add here: not a mapping or a list".into(),
        }
    }

    /// Records an undo snapshot, applies `f` to the tree and re-indexes.
    /// `f` returns the key of the node to focus.
    fn mutate(&mut self, desc: &str, f: impl FnOnce(&mut Self) -> Result<Option<String>>) {
        let snapshot = self.snapshot(desc);
        let collapsed: HashSet<String> =
            self.collapsed.iter().map(|&id| self.node_key(id)).collect();
        match f(self) {
            Err(err) => {
                self.root = snapshot.root;
                self.reindex();
                self.status = format!("Error: {err:#}");
            }
            Ok(focus) => {
                self.undo_stack.push(snapshot);
                if self.undo_stack.len() > MAX_UNDO {
                    self.undo_stack.remove(0);
                }
                self.redo_stack.clear();
                self.next_version += 1;
                self.version = self.next_version;
                renumber_tree(&mut self.root);
                self.restore_view(&collapsed, focus.as_deref());
                if let Some(key) = focus {
                    self.modified.insert(key);
                }
            }
        }
    }

    fn snapshot(&self, desc: &str) -> Snapshot {
        Snapshot {
            root: self.root.clone(),
            modified: self.modified.clone(),
            cursor: self.current_id().map(|id| self.node_key(id)),
            version: self.version,
            desc: desc.to_string(),
        }
    }

    /// Re-indexes and restores folds (by key) and the cursor.
    fn restore_view(&mut self, collapsed: &HashSet<String>, focus: Option<&str>) {
        let cursor = self.cursor;
        self.collapsed.clear();
        self.nodes.clear();
        index_nodes(view_root(&self.root), None, 0, 0, &mut self.nodes);
        self.collapsed = collapsed
            .iter()
            .filter_map(|key| self.id_for_key(key))
            .collect();
        self.rebuild_rows();
        self.cursor = cursor;
        if let Some(id) = focus.and_then(|key| self.id_for_key(key)) {
            self.jump_to(id);
        }
        self.clamp_cursor();
        self.adjust_offset();
    }

    fn confirm_edit(&mut self, target: Option<usize>, input: &str) {
        let Some(id) = target else {
            return;
        };
        if self.node(id).value.as_deref() == Some(input) {
            return;
        }
        let key = self.node_key(id);
        let desc = format!("edit {}", self.display_path(id));
        self.mutate(&desc, |app| {
            retag_scalar(app.node_mut(id), input);
            Ok(Some(key))
        });
    }

    fn confirm_rename(&mut self, target: Option<usize>, input: &str) {
        let Some(id) = target else {
            return;
        };
        if self.node(id).key.as_deref() == Some(input) {
            return;
        }
        if input.is_empty() {
            self.status = "Key cannot be empty".into();
            return;
        }
        let parent = self.nodes[id].parent.expect("renamed node has a parent");
        if self
            .node(parent)
            .children
            .iter()
            .any(|c| c.key.as_deref() == Some(input))
        {
            self.status = format!("Key {input:?} already exists");
            return;
        }
        let (doc, mut parts) = self.location(id);
        parts.pop();
        parts.push(PathPart::Key(input.to_string()));
        let focus = format!("{doc}:{}", format_path(&parts));
        let desc = format!("rename {}", self.display_path(id));
        let new_key = input.to_string();
        self.mutate(&desc, |app| {
            app.node_mut(id).key = Some(new_key);
            Ok(Some(focus))
        });
    }

    fn confirm_add(&mut self, target: Option<usize>, input: &str) {
        let Some(container) = target else {
            return;
        };
        if input.trim().is_empty() {
            return;
        }
        let position = match self.add_target() {
            Some((c, pos)) if c == container => pos,
            _ => None,
        };
        let (doc, parts) = self.location(container);
        let is_seq = self.node(container).kind == NodeKind::Sequence;
        let len = self.node(container).children.len();
        let pos = position.unwrap_or(len).min(len);

        let (key, mut value) = if is_seq {
            match parse_value(input, false) {
                Ok(v) => (None, v),
                Err(err) => {
                    self.status = format!("Error: {err:#}");
                    return;
                }
            }
        } else {
            match parse_key_value(input) {
                Ok((k, v)) => (Some(k), v),
                Err(err) => {
                    self.status = format!("Error: {err:#}");
                    return;
                }
            }
        };
        if let Some(k) = &key {
            if self
                .node(container)
                .children
                .iter()
                .any(|c| c.key.as_deref() == Some(k))
            {
                self.status = format!("Key {k:?} already exists (e: edit)");
                return;
            }
        }
        let mut new_parts = parts.clone();
        new_parts.push(match &key {
            Some(k) => PathPart::Key(k.clone()),
            None => PathPart::Index(pos),
        });
        let focus = format!("{doc}:{}", format_path(&new_parts));
        value.key = key;
        self.mutate("add", |app| {
            app.node_mut(container).children.insert(pos, value);
            Ok(Some(focus))
        });
        // Show the new entry even when the container was folded.
        self.collapsed.remove(&container);
        self.rebuild_rows();
        if let Some(id) = self.id_for(doc, &new_parts) {
            self.jump_to(id);
        }
    }

    fn delete_current(&mut self) {
        let Some(id) = self.current_id() else {
            return;
        };
        if !self.check_writable() {
            return;
        }
        let Some(parent) = self.nodes[id].parent else {
            self.status = "Cannot delete the document root".into();
            return;
        };
        if !matches!(
            self.node(parent).kind,
            NodeKind::Mapping | NodeKind::Sequence
        ) {
            self.status = "Cannot delete the document root".into();
            return;
        }
        let path = self.display_path(id);
        let child_index = self.nodes[id].child_index;
        let cursor = self.cursor;
        self.mutate(&format!("delete {path}"), |app| {
            app.node_mut(parent).children.remove(child_index);
            Ok(None)
        });
        self.cursor = cursor;
        self.clamp_cursor();
        self.adjust_offset();
        self.status = format!("Deleted {path} (u to undo)");
    }

    fn save_file(&mut self) -> Result<()> {
        let Some(path) = self.filename.clone() else {
            self.status = "Cannot save: read-only (stdin)".into();
            return Ok(());
        };
        if !self.dirty() {
            self.status = "No changes to save".into();
            return Ok(());
        }
        let output = serialize(&self.root, self.json, self.indent)?;
        if let Err(err) = write_atomic(&path, output.as_bytes()) {
            self.status = format!("Error: {err:#}");
            return Ok(());
        }
        self.saved_version = self.version;
        self.quit_armed = false;
        self.modified.clear();
        self.status = format!("Saved {}", path.display());
        Ok(())
    }

    fn undo(&mut self) {
        let Some(entry) = self.undo_stack.pop() else {
            self.status = "Nothing to undo".into();
            return;
        };
        let current = self.snapshot(&entry.desc);
        self.redo_stack.push(current);
        self.status = format!("Undo: {}", entry.desc);
        self.restore(entry);
    }

    fn redo(&mut self) {
        let Some(entry) = self.redo_stack.pop() else {
            self.status = "Nothing to redo".into();
            return;
        };
        let current = self.snapshot(&entry.desc);
        self.undo_stack.push(current);
        self.status = format!("Redo: {}", entry.desc);
        self.restore(entry);
    }

    fn restore(&mut self, snapshot: Snapshot) {
        let collapsed: HashSet<String> =
            self.collapsed.iter().map(|&id| self.node_key(id)).collect();
        self.root = snapshot.root;
        self.version = snapshot.version;
        self.modified = if self.dirty() {
            snapshot.modified
        } else {
            HashSet::new()
        };
        self.restore_view(&collapsed, snapshot.cursor.as_deref());
    }

    // ---- navigation helpers ----------------------------------------------------

    fn go_to(&mut self, input: &str) {
        let input = input.trim();
        if input.is_empty() {
            return;
        }
        let path = if input.starts_with('.') || input.starts_with('[') {
            input.to_string()
        } else {
            format!(".{input}")
        };
        let current_doc = self.current_id().map(|id| self.location(id).0).unwrap_or(0);
        let found = {
            let docs = documents(&self.root);
            let mut found: Result<Option<(usize, Vec<PathPart>)>> = Ok(None);
            for doc in std::iter::once(current_doc).chain(0..docs.len()) {
                let Some(doc_node) = docs.get(doc) else {
                    continue;
                };
                match query(doc_node, &path) {
                    Err(err) => {
                        found = Err(err);
                        break;
                    }
                    Ok(hits) => {
                        if let Some(hit) = hits.into_iter().next() {
                            found = Ok(Some((doc, hit.path)));
                            break;
                        }
                    }
                }
            }
            found
        };
        match found {
            Err(err) => {
                self.status = format!("Go to failed: {err:#}");
                return;
            }
            Ok(Some((doc, parts))) => {
                if let Some(id) = self.id_for(doc, &parts) {
                    self.jump_to(id);
                    self.status = format!("→ {}", format_path(&parts));
                    return;
                }
            }
            Ok(None) => {}
        }
        self.status = format!("Go to failed: path not found: {path}");
    }

    fn yank_path(&mut self) {
        let Some(id) = self.current_id() else {
            return;
        };
        let path = self.display_path(id);
        self.status = format!("Copied path {path}");
        self.pending_copy = Some(path);
    }

    fn yank_value(&mut self) {
        let Some(id) = self.current_id() else {
            return;
        };
        let node = self.node(id);
        let text = if node.kind == NodeKind::Scalar {
            node.value.clone().unwrap_or_default()
        } else {
            let mut shown = content(node).clone();
            shown.key = None;
            renumber(&mut shown, 0);
            match format_yaml(&shown, FormatOptions::default()) {
                Ok(text) => text.trim_end().to_string(),
                Err(err) => {
                    self.status = format!("Copy failed: {err:#}");
                    return;
                }
            }
        };
        let lines = text.lines().count();
        self.status = if lines > 1 {
            format!("Copied value ({lines} lines)")
        } else {
            "Copied value".into()
        };
        self.pending_copy = Some(text);
    }

    fn update_search_matches(&mut self) {
        self.matches.clear();
        if self.input.is_empty() {
            return;
        }
        let mut next_id = 0;
        collect_matches(
            view_root(&self.root),
            &self.input,
            &mut next_id,
            &mut self.matches,
        );
    }

    /// The matches are already current: every edit of the query recomputes them.
    fn finish_search(&mut self) {
        let Some(&target) = self.matches.first() else {
            self.status = "No matches".into();
            return;
        };
        self.match_index = 0;
        self.jump_to(target);
    }

    fn next_match(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        self.match_index = (self.match_index + 1) % self.matches.len();
        self.jump_to(self.matches[self.match_index]);
    }

    fn prev_match(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        self.match_index = if self.match_index == 0 {
            self.matches.len() - 1
        } else {
            self.match_index - 1
        };
        self.jump_to(self.matches[self.match_index]);
    }

    /// Expands the ancestors of `id` and moves the cursor onto it.
    fn jump_to(&mut self, id: usize) {
        let mut expanded = false;
        let mut ancestor = self.nodes[id].parent;
        while let Some(parent) = ancestor {
            expanded |= self.collapsed.remove(&parent);
            ancestor = self.nodes[parent].parent;
        }
        if expanded {
            self.rebuild_rows();
        }
        if let Ok(index) = self.rows.binary_search(&id) {
            self.cursor = index;
            self.adjust_offset();
        }
    }
}

/// Parses "key: value" (value optional, parsed as YAML).
fn parse_key_value(text: &str) -> Result<(String, YamNode)> {
    let text = text.trim();
    let (key, value) = match text.split_once(": ") {
        Some((k, v)) => (k.trim(), v.trim()),
        None => (text.strip_suffix(':').unwrap_or(text).trim(), ""),
    };
    let key = key.trim_matches(|c| c == '"' || c == '\'');
    if key.is_empty() {
        anyhow::bail!("key cannot be empty (format: key: value)");
    }
    let value = if value.is_empty() {
        let mut null = string_node("null");
        null.tag = Some("!!null".into());
        null
    } else {
        parse_value(value, false)?
    };
    Ok((key.to_string(), value))
}

/// Parses a path produced by `format_path` back into parts.
fn parse_parts(path: &str) -> Option<Vec<PathPart>> {
    let segs = crate::path::parse_path(path).ok()?;
    segs.into_iter()
        .map(|s| match s.kind {
            crate::path::SegKind::Key(k) => Some(PathPart::Key(k)),
            crate::path::SegKind::Index(i) if i >= 0 => Some(PathPart::Index(i as usize)),
            _ => None,
        })
        .collect()
}

fn renumber_tree(root: &mut YamNode) {
    if root.kind == NodeKind::Stream {
        for (i, doc) in root.children.iter_mut().enumerate() {
            doc.index = i;
            renumber(doc, 0);
        }
    } else {
        renumber(root, 0);
    }
}

fn base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[derive(Debug, Clone)]
struct DiffRow {
    left: String,
    right: String,
    diff_type: DiffType,
    changed: bool,
    /// The row or one of its descendants differs.
    subtree_changed: bool,
}

struct DiffApp {
    all_rows: Vec<DiffRow>,
    rows: Vec<DiffRow>,
    changes_only: bool,
    left_file: String,
    right_file: String,
    summary: String,
    cursor: usize,
    offset: usize,
    show_help: bool,
    status: String,
}

impl DiffApp {
    fn new(result: &DiffResult<'_>) -> Self {
        let mut rows = Vec::new();
        if let Some(root) = &result.root {
            collect_diff_rows(root, &mut rows);
        }
        Self {
            all_rows: rows.clone(),
            rows,
            changes_only: false,
            left_file: if result.left_file.is_empty() {
                "(left)".into()
            } else {
                result.left_file.clone()
            },
            right_file: if result.right_file.is_empty() {
                "(right)".into()
            } else {
                result.right_file.clone()
            },
            summary: format!(
                "+ {}  - {}  ~ {}",
                result.summary.added, result.summary.removed, result.summary.modified
            ),
            cursor: 0,
            offset: 0,
            show_help: false,
            status: String::new(),
        }
    }

    fn run(&mut self) -> Result<()> {
        loop {
            self.draw()?;
            // Block until the next event; any event, a resize included, redraws.
            if let Event::Key(key) = event::read()? {
                if self.handle_key(key) {
                    break;
                }
            }
        }
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        self.status.clear();
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return true,
            KeyCode::Char('c') if ctrl => return true,
            KeyCode::Char('u') if ctrl => self.move_cursor(-(self.viewport_height() as isize)),
            KeyCode::Char('d') if ctrl => self.move_cursor(self.viewport_height() as isize),
            _ if ctrl => {}
            KeyCode::Char('?') => self.show_help = !self.show_help,
            KeyCode::Up | KeyCode::Char('k') => self.move_cursor(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_cursor(1),
            KeyCode::PageUp | KeyCode::Char('b') => {
                self.move_cursor(-(self.viewport_height() as isize))
            }
            KeyCode::PageDown | KeyCode::Char('f') | KeyCode::Char(' ') => {
                self.move_cursor(self.viewport_height() as isize)
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.cursor = 0;
                self.offset = 0;
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.cursor = self.rows.len().saturating_sub(1);
                self.adjust_offset();
            }
            KeyCode::Char('n' | ']') => self.next_diff(),
            KeyCode::Char('N' | '[') => self.prev_diff(),
            KeyCode::Char('c') => self.toggle_changes_only(),
            _ => {}
        }
        false
    }

    /// Hides (or shows again) the rows whose subtree has no differences.
    fn toggle_changes_only(&mut self) {
        self.changes_only = !self.changes_only;
        self.rows = if self.changes_only {
            self.all_rows
                .iter()
                .filter(|r| r.subtree_changed)
                .cloned()
                .collect()
        } else {
            self.all_rows.clone()
        };
        self.cursor = 0;
        self.offset = 0;
        self.status = if self.changes_only {
            "Showing changes only (c: show all)".into()
        } else {
            "Showing all nodes".into()
        };
    }

    fn draw(&mut self) -> Result<()> {
        let (width, height) = terminal::size()?;
        self.clamp_cursor();
        self.adjust_offset();

        let mut stdout = io::stdout();
        queue!(stdout, MoveTo(0, 0), Clear(ClearType::All))?;
        draw_bar(
            &mut stdout,
            0,
            width,
            &format!(" ← OLD: {}  │  NEW: {} →", self.left_file, self.right_file),
            Color::Blue,
        )?;

        let pane = width.saturating_sub(3) / 2;
        let viewport = self
            .viewport_height()
            .min(height.saturating_sub(3) as usize);
        for line_idx in 0..viewport {
            let row_idx = self.offset + line_idx;
            queue!(stdout, MoveTo(0, (line_idx + 1) as u16))?;
            if row_idx >= self.rows.len() {
                continue;
            }
            let row = &self.rows[row_idx];
            if row_idx == self.cursor {
                queue!(stdout, SetAttribute(Attribute::Reverse))?;
            }
            let color = match row.diff_type {
                DiffType::Added => Color::Green,
                DiffType::Removed => Color::Red,
                DiffType::Modified => Color::Yellow,
                DiffType::Unchanged => Color::DarkGrey,
            };
            queue!(stdout, SetForegroundColor(color))?;
            let left_prefix = match row.diff_type {
                DiffType::Removed => "- ",
                DiffType::Modified => "~ ",
                _ => "  ",
            };
            let right_prefix = match row.diff_type {
                DiffType::Added => "+ ",
                DiffType::Modified => "~ ",
                _ => "  ",
            };
            let left = truncate(&format!("{left_prefix}{}", row.left), pane as usize);
            let right = truncate(&format!("{right_prefix}{}", row.right), pane as usize);
            queue!(
                stdout,
                Print(pad(&left, pane as usize)),
                ResetColor,
                Print(" │ "),
                SetForegroundColor(color),
                Print(pad(&right, pane as usize)),
                ResetColor,
                SetAttribute(Attribute::Reset)
            )?;
        }

        let footer = if self.status.is_empty() {
            format!(
                "{}/{}  |  {}",
                self.cursor.saturating_add(1),
                self.rows.len(),
                self.summary
            )
        } else {
            self.status.clone()
        };
        draw_bar(
            &mut stdout,
            height.saturating_sub(2),
            width,
            &footer,
            Color::DarkGrey,
        )?;
        queue!(stdout, MoveTo(0, height.saturating_sub(1)))?;
        let help = if self.show_help {
            "↑/k ↓/j PgUp/b PgDn/f Ctrl+U/D g/G n/] N/[ c changes-only ? q/Esc"
        } else {
            "↑/k ↓/j move  n/] next diff  N/[ prev diff  c changes only  ? help  q quit"
        };
        queue!(stdout, Print(truncate(help, width as usize)))?;
        stdout.flush()?;
        Ok(())
    }

    fn move_cursor(&mut self, delta: isize) {
        if self.rows.is_empty() {
            self.cursor = 0;
            return;
        }
        self.cursor = (self.cursor as isize + delta)
            .clamp(0, self.rows.len().saturating_sub(1) as isize) as usize;
        self.adjust_offset();
    }

    fn next_diff(&mut self) {
        for idx in self.cursor.saturating_add(1)..self.rows.len() {
            if self.rows[idx].changed {
                self.cursor = idx;
                self.adjust_offset();
                return;
            }
        }
        self.status = "No more diffs below".into();
    }

    fn prev_diff(&mut self) {
        for idx in (0..self.cursor).rev() {
            if self.rows[idx].changed {
                self.cursor = idx;
                self.adjust_offset();
                return;
            }
        }
        self.status = "No more diffs above".into();
    }

    fn clamp_cursor(&mut self) {
        if self.rows.is_empty() {
            self.cursor = 0;
        } else if self.cursor >= self.rows.len() {
            self.cursor = self.rows.len() - 1;
        }
    }

    fn adjust_offset(&mut self) {
        let vh = self.viewport_height();
        if self.cursor < self.offset {
            self.offset = self.cursor;
        } else if self.cursor >= self.offset + vh {
            self.offset = self.cursor.saturating_sub(vh.saturating_sub(1));
        }
    }

    fn viewport_height(&self) -> usize {
        terminal::size()
            .map(|(_, h)| viewport_rows(h))
            .unwrap_or(20)
    }
}

fn diff_subtree_changed(node: &DiffNode<'_>) -> bool {
    node.diff_type != DiffType::Unchanged || node.children.iter().any(diff_subtree_changed)
}

fn collect_diff_rows(node: &DiffNode<'_>, rows: &mut Vec<DiffRow>) {
    let is_document = node.left.is_some_and(|n| n.kind == NodeKind::Document)
        || node.right.is_some_and(|n| n.kind == NodeKind::Document);
    let is_root_container = node.label.is_none()
        && node.path.is_empty()
        && node.left.is_none_or(|n| n.depth == 0 && n.key.is_none())
        && node.right.is_none_or(|n| n.depth == 0 && n.key.is_none());
    if let Some(label) = &node.label {
        rows.push(DiffRow {
            left: node
                .left
                .map(|_| format!("=== {label}"))
                .unwrap_or_default(),
            right: node
                .right
                .map(|_| format!("=== {label}"))
                .unwrap_or_default(),
            diff_type: node.diff_type,
            changed: matches!(node.diff_type, DiffType::Added | DiffType::Removed),
            subtree_changed: diff_subtree_changed(node),
        });
    } else if !is_document && !(is_root_container && !node.children.is_empty()) {
        rows.push(DiffRow {
            left: node.left.map(format_diff_node).unwrap_or_default(),
            right: node.right.map(format_diff_node).unwrap_or_default(),
            diff_type: node.diff_type,
            changed: node.diff_type != DiffType::Unchanged,
            subtree_changed: diff_subtree_changed(node),
        });
    }
    for child in &node.children {
        collect_diff_rows(child, rows);
    }
}

fn format_diff_node(node: &YamNode) -> String {
    let indent = "  ".repeat(node.depth.saturating_sub(1));
    let key = node.key.clone().unwrap_or_else(|| {
        if node.depth > 0 {
            format!("[{}]", node.index)
        } else {
            String::new()
        }
    });
    match node.kind {
        NodeKind::Mapping | NodeKind::Sequence if !node.children.is_empty() => {
            if key.is_empty() {
                format!("{indent}{}", display_value(Some(node)))
            } else {
                format!("{indent}{key}:")
            }
        }
        NodeKind::Document | NodeKind::Stream => String::new(),
        _ => {
            if key.is_empty() {
                format!("{indent}{}", display_value(Some(node)))
            } else {
                format!("{indent}{key}: {}", display_value(Some(node)))
            }
        }
    }
}

fn draw_bar(stdout: &mut io::Stdout, y: u16, width: u16, text: &str, color: Color) -> Result<()> {
    queue!(
        stdout,
        MoveTo(0, y),
        SetForegroundColor(color),
        SetAttribute(Attribute::Bold),
        Print(pad(&truncate(text, width as usize), width as usize)),
        ResetColor,
        SetAttribute(Attribute::Reset)
    )?;
    Ok(())
}

fn truncate(input: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let count = input.chars().count();
    if count <= width {
        input.to_string()
    } else if width == 1 {
        "…".into()
    } else {
        let mut output = input.chars().take(width - 1).collect::<String>();
        output.push('…');
        output
    }
}

fn pad(input: &str, width: usize) -> String {
    let count = input.chars().count();
    if count >= width {
        input.to_string()
    } else {
        format!("{input}{}", " ".repeat(width - count))
    }
}

/// Rows left for content below the header and above the footer and help
/// lines; at least one, so paging still moves on a tiny terminal.
fn viewport_rows(terminal_height: u16) -> usize {
    terminal_height.saturating_sub(3).max(1) as usize
}

/// The node shown as the tree's first row: the content of a single
/// document, or the stream itself.
fn view_root(root: &YamNode) -> &YamNode {
    if root.kind == NodeKind::Document {
        root.children.first().unwrap_or(root)
    } else {
        root
    }
}

fn view_root_mut(root: &mut YamNode) -> &mut YamNode {
    if root.kind == NodeKind::Document && !root.children.is_empty() {
        &mut root.children[0]
    } else {
        root
    }
}

/// Numbers the subtree in preorder, matching the ids `collect_matches` assigns.
fn index_nodes(
    node: &YamNode,
    parent: Option<usize>,
    child_index: usize,
    level: usize,
    nodes: &mut Vec<NodeMeta>,
) {
    let id = nodes.len();
    nodes.push(NodeMeta {
        parent,
        child_index,
        end: 0,
        level,
    });
    for (idx, child) in node.children.iter().enumerate() {
        index_nodes(child, Some(id), idx, level + 1, nodes);
    }
    nodes[id].end = nodes.len();
}

fn collect_matches(node: &YamNode, needle: &str, next_id: &mut usize, matches: &mut Vec<usize>) {
    let id = *next_id;
    *next_id += 1;
    let hit = |text: &Option<String>| {
        text.as_deref()
            .is_some_and(|text| contains_ignore_ascii_case(text, needle))
    };
    let searchable = !matches!(node.kind, NodeKind::Document | NodeKind::Stream);
    if searchable && (hit(&node.key) || hit(&node.value)) {
        matches.push(id);
    }
    for child in &node.children {
        collect_matches(child, needle, next_id, matches);
    }
}

fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    let (haystack, needle) = (haystack.as_bytes(), needle.as_bytes());
    needle.is_empty()
        || haystack
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_yaml_str;
    use crate::path::get_by_path;
    use std::fs;

    fn app_with(yaml: &str, filename: Option<PathBuf>, json: bool) -> TreeApp {
        let root = if json {
            crate::parser::parse_json(yaml.as_bytes()).unwrap()
        } else {
            parse_yaml_str(yaml).unwrap()
        };
        TreeApp::new(TreeOptions {
            root,
            filename,
            display_name: "x.yaml".into(),
            json,
            style: TreeStyle::Unicode,
            show_types: false,
            focus: None,
        })
    }

    fn app(yaml: &str) -> TreeApp {
        app_with(yaml, Some(PathBuf::from("x.yaml")), false)
    }

    fn press(app: &mut TreeApp, code: KeyCode) {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
            .unwrap();
    }

    fn ctrl(app: &mut TreeApp, ch: char) {
        app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL))
            .unwrap();
    }

    fn type_text(app: &mut TreeApp, text: &str) {
        ctrl(app, 'u');
        for ch in text.chars() {
            press(app, KeyCode::Char(ch));
        }
    }

    fn goto(app: &mut TreeApp, path: &str) {
        press(app, KeyCode::Char(':'));
        type_text(app, path);
        press(app, KeyCode::Enter);
        let id = app.current_id().unwrap();
        assert_eq!(app.display_path(id), path, "status: {}", app.status);
    }

    fn temp_file(name: &str, content: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yam-tui-{}-{name}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn tree_app_flattens_collapsed_nodes() {
        let mut app = app("a:\n  b: 1\nc: 2\n");
        assert_eq!(app.rows.len(), 4);
        app.cursor = 1;
        app.toggle_current();
        assert_eq!(app.rows.len(), 3);
        assert_eq!(app.render_node_line(1), "▶ a: {1 keys}");
    }

    #[test]
    fn edits_scalars_with_undo_and_redo() {
        let mut app = app("a: old\nn: 1\n");
        goto(&mut app, ".a");
        press(&mut app, KeyCode::Char('e'));
        type_text(&mut app, "new");
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            get_by_path(&app.root, ".a").unwrap().value.as_deref(),
            Some("new")
        );
        assert!(app.dirty());
        assert!(app.modified.contains("0:.a"));

        goto(&mut app, ".n");
        press(&mut app, KeyCode::Char('e'));
        type_text(&mut app, "true");
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            get_by_path(&app.root, ".n").unwrap().tag.as_deref(),
            Some("!!bool")
        );

        press(&mut app, KeyCode::Char('u'));
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(
            get_by_path(&app.root, ".a").unwrap().value.as_deref(),
            Some("old")
        );
        assert!(!app.dirty(), "undo back to the original is clean");
        ctrl(&mut app, 'r');
        assert!(app.dirty());
        assert_eq!(
            get_by_path(&app.root, ".a").unwrap().value.as_deref(),
            Some("new")
        );
    }

    #[test]
    fn adds_renames_and_deletes() {
        let mut app = app("app:\n  name: demo\n  port: 8080\n  tags:\n    - a\n    - b\n");

        goto(&mut app, ".app.name");
        press(&mut app, KeyCode::Char('a'));
        type_text(&mut app, "debug: true");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.display_path(app.current_id().unwrap()), ".app.debug");

        goto(&mut app, ".app.tags");
        press(&mut app, KeyCode::Char('a'));
        type_text(&mut app, "c");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.display_path(app.current_id().unwrap()), ".app.tags[2]");

        goto(&mut app, ".app.port");
        press(&mut app, KeyCode::Char('r'));
        type_text(&mut app, "listen");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.display_path(app.current_id().unwrap()), ".app.listen");

        press(&mut app, KeyCode::Char('r'));
        type_text(&mut app, "name");
        press(&mut app, KeyCode::Enter);
        assert!(app.status.contains("already exists"), "{}", app.status);

        goto(&mut app, ".app.tags[0]");
        press(&mut app, KeyCode::Char('D'));
        assert!(app.status.starts_with("Deleted"), "{}", app.status);

        assert_eq!(
            format_yaml(&app.root, FormatOptions::default()).unwrap(),
            "app:\n  name: demo\n  debug: true\n  listen: 8080\n  tags:\n  - b\n  - c\n"
        );

        // Undo restores the deleted item.
        press(&mut app, KeyCode::Char('u'));
        assert!(get_by_path(&app.root, ".app.tags[2]").is_ok());
    }

    #[test]
    fn saves_json_as_json_and_keeps_documents() {
        let path = temp_file("c.json", r#"{"z": 1, "a": {"b": "x"}}"#);
        let mut app = app_with(
            &fs::read_to_string(&path).unwrap(),
            Some(path.clone()),
            true,
        );
        goto(&mut app, ".a.b");
        press(&mut app, KeyCode::Char('e'));
        type_text(&mut app, "y");
        press(&mut app, KeyCode::Enter);
        ctrl(&mut app, 's');
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\n  \"z\": 1,\n  \"a\": {\n    \"b\": \"y\"\n  }\n}\n"
        );
        assert!(!app.dirty());

        let path = temp_file("m.yaml", "kind: A\nname: one\n---\nkind: B\nname: two\n");
        let mut app = app_with(
            &fs::read_to_string(&path).unwrap(),
            Some(path.clone()),
            false,
        );
        assert_eq!(app.root.kind, NodeKind::Stream);
        press(&mut app, KeyCode::Char('G'));
        assert_eq!(app.location(app.current_id().unwrap()).0, 1);
        press(&mut app, KeyCode::Char('e'));
        type_text(&mut app, "zwei");
        press(&mut app, KeyCode::Enter);
        ctrl(&mut app, 's');
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "kind: A\nname: one\n---\nkind: B\nname: zwei\n"
        );

        press(&mut app, KeyCode::Char('C'));
        let lines: Vec<String> = app
            .rows
            .iter()
            .map(|&id| app.render_node_line(id))
            .collect();
        assert_eq!(
            lines,
            ["▼ 2 documents", "▶ --- document 1", "▶ --- document 2"]
        );
    }

    #[test]
    fn read_only_input_cannot_be_edited() {
        let mut app = app_with("a: 1\n", None, false);
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('e'));
        assert_eq!(app.mode, InputMode::Normal);
        assert!(app.status.contains("read-only"));
        press(&mut app, KeyCode::Char('D'));
        assert!(app.status.contains("read-only"));
    }

    #[test]
    fn quitting_with_changes_needs_confirmation() {
        let mut app = app("a: 1\nb: 2\n");
        goto(&mut app, ".a");
        press(&mut app, KeyCode::Char('D'));
        assert!(!app
            .handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
            .unwrap());
        assert!(app.dirty());
        assert!(app
            .handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
            .unwrap());
    }

    #[test]
    fn yanks_paths_and_values() {
        let mut app = app("a:\n  \"b.c\":\n    - x\n");
        goto(&mut app, r#".a["b.c"][0]"#);
        press(&mut app, KeyCode::Char('y'));
        assert_eq!(app.pending_copy.as_deref(), Some(r#".a["b.c"][0]"#));
        goto(&mut app, r#".a["b.c"]"#);
        press(&mut app, KeyCode::Char('Y'));
        assert_eq!(app.pending_copy.as_deref(), Some("- x"));
        assert_eq!(base64(b"hello"), "aGVsbG8=");
        assert_eq!(base64(b"hi!"), "aGkh");
    }

    #[test]
    fn focus_option_places_the_cursor() {
        let root = parse_yaml_str("a:\n  b:\n    - x\n    - y\n").unwrap();
        let app = TreeApp::new(TreeOptions {
            root,
            filename: None,
            display_name: "x".into(),
            json: false,
            style: TreeStyle::Unicode,
            show_types: false,
            focus: Some((
                0,
                vec![
                    PathPart::Key("a".into()),
                    PathPart::Key("b".into()),
                    PathPart::Index(1),
                ],
            )),
        });
        assert_eq!(app.display_path(app.current_id().unwrap()), ".a.b[1]");
    }

    #[test]
    fn navigation_reuses_rows_regardless_of_document_size() {
        for size in [10, 10_000] {
            let yaml: String = (0..size).map(|i| format!("k{i}:\n  v: {i}\n")).collect();
            let mut app = app(&yaml);
            assert_eq!(app.rows.len(), 1 + 2 * size);
            let builds = app.row_builds;
            for _ in 0..100 {
                press(&mut app, KeyCode::Char('j'));
            }
            press(&mut app, KeyCode::Char('G'));
            press(&mut app, KeyCode::Char('g'));
            assert_eq!(app.row_builds, builds, "size {size}");

            press(&mut app, KeyCode::Char('j'));
            press(&mut app, KeyCode::Enter);
            assert_eq!(app.row_builds, builds + 1, "size {size}");
            assert_eq!(app.rows.len(), 2 * size);
        }
    }

    #[test]
    fn search_is_case_insensitive_and_jumps_through_collapsed_nodes() {
        let mut app = app("Name: x\nitems:\n  - name: first\n  - other: NAMED\n");
        app.collapse_all();
        press(&mut app, KeyCode::Char('/'));
        for ch in "nAmE".chars() {
            press(&mut app, KeyCode::Char(ch));
        }
        // Ids: 0 root, 1 Name, 2 items, 3 [0], 4 name, 5 [1], 6 other.
        assert_eq!(app.matches, vec![1, 4, 6]);
        assert!(app.is_match(4) && !app.is_match(3));

        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.current_id(), Some(4));
        assert_eq!(app.display_path(4), ".items[0].name");
    }

    #[test]
    fn viewport_rows_keep_one_row_on_tiny_terminals() {
        for (height, rows) in [
            (0, 1),
            (1, 1),
            (2, 1),
            (3, 1),
            (4, 1),
            (5, 2),
            (u16::MAX, 65532),
        ] {
            assert_eq!(viewport_rows(height), rows, "height {height}");
        }
    }

    #[test]
    fn empty_document_is_a_single_null_row() {
        let mut app = app("");
        assert_eq!(app.rows, vec![0]);
        assert_eq!(app.render_node_line(0), "  null");
        let builds = app.row_builds;
        for code in [
            KeyCode::Char('j'),
            KeyCode::Char('k'),
            KeyCode::Char('G'),
            KeyCode::Enter,
        ] {
            press(&mut app, code);
        }
        assert_eq!(app.cursor, 0);
        assert_eq!(app.row_builds, builds, "a scalar root cannot be folded");
    }

    #[test]
    fn row_names_follow_depth_boundaries() {
        let app = app("- a\n- [b]\n");
        let lines: Vec<String> = app
            .rows
            .iter()
            .map(|&id| app.render_node_line(id))
            .collect();
        assert_eq!(lines, ["▼ [2 items]", "  [0] a", "▼ [1]", "    [0] b"]);

        assert_eq!(self::app("42\n").render_node_line(0), "  42");
        assert_eq!(self::app("a: 1\n").render_node_line(1), "  a: 1");
    }

    #[test]
    fn diff_rows_can_show_changes_only() {
        let left = parse_yaml_str("a: 1\nb:\n  c: 1\n  d: 1\ne: 1\n").unwrap();
        let right = parse_yaml_str("a: 1\nb:\n  c: 1\n  d: 2\ne: 1\n").unwrap();
        let result = crate::diff::compare(&left, &right);
        let mut app = DiffApp::new(&result);
        let all = app.rows.len();
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        assert!(app.rows.len() < all);
        assert!(app.rows.iter().all(|r| r.subtree_changed));
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        assert_eq!(app.rows.len(), all);
    }
}
