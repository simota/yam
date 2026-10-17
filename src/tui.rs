use crate::diff::{DiffNode, DiffResult, DiffType};
use crate::format::{format_yaml, FormatOptions};
use crate::parser::{NodeKind, ScalarType, YamNode};
use crate::renderer::TreeStyle;
use anyhow::{Context, Result};
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, queue};
use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

const MAX_UNDO: usize = 10;

/// A node's place in the preorder walk of the viewed tree. Its index in
/// `TreeApp::nodes` is the node's id. Edits only replace scalar values, so the
/// tree's shape and therefore every id stay fixed for the whole session.
#[derive(Debug, Clone, Copy)]
struct NodeMeta {
    parent: Option<usize>,
    child_index: usize,
    /// Id one past the last node of this subtree.
    end: usize,
}

#[derive(Debug, Clone)]
struct UndoEntry {
    id: usize,
    old_value: String,
    new_value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputMode {
    Normal,
    Search,
    Edit,
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

pub fn run_tree(
    root: YamNode,
    filename: Option<PathBuf>,
    display_name: String,
    base_path: Vec<String>,
    style: TreeStyle,
    show_types: bool,
) -> Result<()> {
    let _guard = TerminalGuard::enter()?;
    TreeApp::new(root, filename, display_name, base_path, style, show_types).run()
}

pub fn run_diff(result: &DiffResult<'_>) -> Result<()> {
    let _guard = TerminalGuard::enter()?;
    DiffApp::new(result).run()
}

struct TreeApp {
    root: YamNode,
    filename: Option<PathBuf>,
    display_name: String,
    /// Path of the viewed node inside the file, shown before every row path.
    base_path: Vec<String>,
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
    edit_id: Option<usize>,
    edit_original: String,
    modified_ids: HashSet<usize>,
    modified: bool,
    quit_armed: bool,
    status: String,
    /// Ids of the search hits in ascending order.
    matches: Vec<usize>,
    match_index: usize,
    undo_stack: Vec<UndoEntry>,
    redo_stack: Vec<UndoEntry>,
    #[cfg(test)]
    row_builds: usize,
}

impl TreeApp {
    fn new(
        root: YamNode,
        filename: Option<PathBuf>,
        display_name: String,
        base_path: Vec<String>,
        style: TreeStyle,
        show_types: bool,
    ) -> Self {
        let mut nodes = Vec::new();
        index_nodes(view_root(&root), None, 0, &mut nodes);
        let mut app = Self {
            root,
            filename,
            display_name,
            base_path,
            style,
            show_types,
            nodes,
            rows: Vec::new(),
            collapsed: HashSet::new(),
            cursor: 0,
            offset: 0,
            show_help: false,
            mode: InputMode::Normal,
            input: String::new(),
            edit_id: None,
            edit_original: String::new(),
            modified_ids: HashSet::new(),
            modified: false,
            quit_armed: false,
            status: String::new(),
            matches: Vec::new(),
            match_index: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            #[cfg(test)]
            row_builds: 0,
        };
        app.rebuild_rows();
        app
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
        }

        match self.mode {
            InputMode::Search => return Ok(self.handle_search_key(key)),
            InputMode::Edit => return self.handle_edit_key(key),
            InputMode::Normal => {}
        }

        let is_quit_key = matches!(
            key,
            KeyEvent {
                code: KeyCode::Char('q'),
                ..
            } | KeyEvent {
                code: KeyCode::Char('c'),
                modifiers: KeyModifiers::CONTROL,
                ..
            }
        );
        if !is_quit_key {
            self.quit_armed = false;
        }
        self.status.clear();

        match key {
            KeyEvent {
                code: KeyCode::Char('q'),
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('c'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                if self.modified && !self.quit_armed {
                    self.status =
                        "Unsaved changes! Press q again to quit, or Ctrl+S to save".into();
                    self.quit_armed = true;
                    return Ok(false);
                }
                return Ok(true);
            }
            KeyEvent {
                code: KeyCode::Char('?'),
                ..
            } => self.show_help = !self.show_help,
            KeyEvent {
                code: KeyCode::Up, ..
            }
            | KeyEvent {
                code: KeyCode::Char('k'),
                ..
            } => self.move_cursor(-1),
            KeyEvent {
                code: KeyCode::Down,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('j'),
                ..
            } => self.move_cursor(1),
            KeyEvent {
                code: KeyCode::PageUp,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('b'),
                ..
            } => self.move_cursor(-(self.viewport_height() as isize)),
            KeyEvent {
                code: KeyCode::PageDown,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('f'),
                ..
            }
            | KeyEvent {
                code: KeyCode::Char(' '),
                ..
            } => self.move_cursor(self.viewport_height() as isize),
            KeyEvent {
                code: KeyCode::Char('u'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => self.move_cursor(-((self.viewport_height() / 2) as isize)),
            KeyEvent {
                code: KeyCode::Char('d'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => self.move_cursor((self.viewport_height() / 2) as isize),
            KeyEvent {
                code: KeyCode::Home,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('g'),
                ..
            } => {
                self.cursor = 0;
                self.offset = 0;
            }
            KeyEvent {
                code: KeyCode::End, ..
            }
            | KeyEvent {
                code: KeyCode::Char('G'),
                ..
            } => {
                self.cursor = self.rows.len().saturating_sub(1);
                self.adjust_offset();
            }
            KeyEvent {
                code: KeyCode::Enter,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('o'),
                ..
            } => self.toggle_current(),
            KeyEvent {
                code: KeyCode::Char('O'),
                ..
            } => {
                self.collapsed.clear();
                self.rebuild_rows();
            }
            KeyEvent {
                code: KeyCode::Char('C'),
                ..
            } => self.collapse_all(),
            KeyEvent {
                code: KeyCode::Char('/'),
                ..
            } => {
                self.mode = InputMode::Search;
                self.input.clear();
                self.matches.clear();
            }
            KeyEvent {
                code: KeyCode::Char('n'),
                ..
            } => self.next_match(),
            KeyEvent {
                code: KeyCode::Char('N'),
                ..
            } => self.prev_match(),
            KeyEvent {
                code: KeyCode::Char('e'),
                ..
            } => self.start_edit(),
            KeyEvent {
                code: KeyCode::Char('s'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => self.save_file()?,
            KeyEvent {
                code: KeyCode::Char('u'),
                ..
            } => self.undo(),
            KeyEvent {
                code: KeyCode::Char('r'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => self.redo(),
            _ => {}
        }
        Ok(false)
    }

    fn handle_search_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.mode = InputMode::Normal;
                self.input.clear();
                self.matches.clear();
            }
            KeyCode::Enter => {
                self.finish_search();
                self.mode = InputMode::Normal;
            }
            KeyCode::Backspace => {
                self.input.pop();
                self.update_search_matches();
            }
            KeyCode::Char(ch) => {
                self.input.push(ch);
                self.update_search_matches();
            }
            _ => {}
        }
        false
    }

    fn handle_edit_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key {
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                self.mode = InputMode::Normal;
                self.input.clear();
                self.edit_id = None;
                self.edit_original.clear();
            }
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => {
                self.confirm_edit();
                self.mode = InputMode::Normal;
            }
            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } => {
                self.input.pop();
            }
            KeyEvent {
                code: KeyCode::Char('u'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => self.input.clear(),
            KeyEvent {
                code: KeyCode::Char(ch),
                ..
            } => self.input.push(ch),
            _ => {}
        }
        Ok(false)
    }

    fn draw(&mut self) -> Result<()> {
        let (width, height) = terminal::size()?;
        self.clamp_cursor();
        self.adjust_offset();

        let mut stdout = io::stdout();
        queue!(stdout, MoveTo(0, 0), Clear(ClearType::All))?;

        let mut header = format!(" yam - {}", self.display_name);
        if self.modified {
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
            } else if self.modified_ids.contains(&id) {
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
            "↑/k ↓/j PgUp/b PgDn/f Ctrl+U/D g/G Enter/o O C / n/N e u Ctrl+R Ctrl+S ? q"
        } else {
            "↑/k ↓/j move  Enter/o fold  e edit  / search  ? help  q quit"
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
            InputMode::Normal if !self.status.is_empty() => self.status.clone(),
            InputMode::Normal => {
                let pos = format!("{}/{}", self.cursor.saturating_add(1), self.rows.len());
                let path = self
                    .current_id()
                    .map(|id| self.display_path(id))
                    .unwrap_or_else(|| "$".into());
                let mut text = format!("{pos} | {path}");
                if !self.matches.is_empty() {
                    text.push_str(&format!(
                        "  [match {}/{}]",
                        self.match_index.saturating_add(1),
                        self.matches.len()
                    ));
                }
                if self.modified {
                    text.push_str("  [modified - Ctrl+S to save]");
                }
                text
            }
        }
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

    fn display_path(&self, id: usize) -> String {
        let mut segments = self.base_path.clone();
        let mut node = view_root(&self.root);
        for idx in self.child_indices(id) {
            node = &node.children[idx];
            segments.push(node.key.clone().unwrap_or_else(|| idx.to_string()));
        }
        if segments.is_empty() {
            "$".into()
        } else {
            format!("$.{}", segments.join("."))
        }
    }

    fn render_node_line(&self, id: usize) -> String {
        let node = self.node(id);
        let depth = node.depth.saturating_sub(1);
        let indent = "  ".repeat(depth);
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
        let name = if let Some(key) = &node.key {
            format!("{key}:")
        } else if node.depth > 0 {
            format!("[{}]", node.index)
        } else {
            String::new()
        };
        let value = if node.kind == NodeKind::Scalar {
            let mut rendered = node.value.clone().unwrap_or_default();
            if self.show_types {
                rendered.push(' ');
                rendered.push_str(type_label(node.infer_type()));
            }
            if name.is_empty() {
                rendered
            } else {
                format!(" {rendered}")
            }
        } else {
            String::new()
        };
        format!("{indent}{indicator}{name}{value}")
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
        // viewed root (id 0) stays expanded.
        self.collapsed = (1..self.nodes.len())
            .filter(|&id| self.nodes[id].end > id + 1)
            .collect();
        self.cursor = 0;
        self.offset = 0;
        self.rebuild_rows();
    }

    fn start_edit(&mut self) {
        let Some(id) = self.current_id() else {
            return;
        };
        let node = self.node(id);
        if node.kind != NodeKind::Scalar {
            self.status = "Cannot edit: not a scalar value".into();
            return;
        }
        if self.filename.is_none() {
            self.status = "Cannot edit: read-only".into();
            return;
        }
        self.edit_original = node.value.clone().unwrap_or_default();
        self.mode = InputMode::Edit;
        self.edit_id = Some(id);
        self.input = self.edit_original.clone();
    }

    fn confirm_edit(&mut self) {
        let input = std::mem::take(&mut self.input);
        let original = std::mem::take(&mut self.edit_original);
        let Some(id) = self.edit_id.take() else {
            return;
        };
        if input == original {
            return;
        }
        self.node_mut(id).value = Some(input.clone());
        self.push_undo(UndoEntry {
            id,
            old_value: original,
            new_value: input,
        });
        self.modified = true;
        self.modified_ids.insert(id);
    }

    fn save_file(&mut self) -> Result<()> {
        let Some(path) = &self.filename else {
            self.status = "Cannot save: read-only".into();
            return Ok(());
        };
        if !self.modified {
            self.status = "No changes to save".into();
            return Ok(());
        }
        let output = format_yaml(&self.root, FormatOptions::default())?;
        fs::write(path, output)
            .with_context(|| format!("failed to write file: {}", path.display()))?;
        self.modified = false;
        self.modified_ids.clear();
        self.status = "Saved!".into();
        Ok(())
    }

    fn push_undo(&mut self, entry: UndoEntry) {
        self.undo_stack.push(entry);
        if self.undo_stack.len() > MAX_UNDO {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    fn undo(&mut self) {
        let Some(entry) = self.undo_stack.pop() else {
            self.status = "Nothing to undo".into();
            return;
        };
        self.node_mut(entry.id).value = Some(entry.old_value.clone());
        self.modified = true;
        self.modified_ids.insert(entry.id);
        self.status = "Undo: restored value".into();
        self.redo_stack.push(entry);
    }

    fn redo(&mut self) {
        let Some(entry) = self.redo_stack.pop() else {
            self.status = "Nothing to redo".into();
            return;
        };
        self.node_mut(entry.id).value = Some(entry.new_value.clone());
        self.modified = true;
        self.modified_ids.insert(entry.id);
        self.status = "Redo: re-applied value".into();
        self.undo_stack.push(entry);
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

#[derive(Debug, Clone)]
struct DiffRow {
    left: String,
    right: String,
    diff_type: DiffType,
    changed: bool,
}

struct DiffApp {
    rows: Vec<DiffRow>,
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
            rows,
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
        match key {
            KeyEvent {
                code: KeyCode::Char('q') | KeyCode::Esc,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('c'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => return true,
            KeyEvent {
                code: KeyCode::Char('?'),
                ..
            } => self.show_help = !self.show_help,
            KeyEvent {
                code: KeyCode::Up, ..
            }
            | KeyEvent {
                code: KeyCode::Char('k'),
                ..
            } => self.move_cursor(-1),
            KeyEvent {
                code: KeyCode::Down,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('j'),
                ..
            } => self.move_cursor(1),
            KeyEvent {
                code: KeyCode::PageUp,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('b'),
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('u'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => self.move_cursor(-(self.viewport_height() as isize)),
            KeyEvent {
                code: KeyCode::PageDown,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('f'),
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('d'),
                modifiers: KeyModifiers::CONTROL,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char(' '),
                ..
            } => self.move_cursor(self.viewport_height() as isize),
            KeyEvent {
                code: KeyCode::Home,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('g'),
                ..
            } => {
                self.cursor = 0;
                self.offset = 0;
            }
            KeyEvent {
                code: KeyCode::End, ..
            }
            | KeyEvent {
                code: KeyCode::Char('G'),
                ..
            } => {
                self.cursor = self.rows.len().saturating_sub(1);
                self.adjust_offset();
            }
            KeyEvent {
                code: KeyCode::Char('n' | ']'),
                ..
            } => self.next_diff(),
            KeyEvent {
                code: KeyCode::Char('N' | '['),
                ..
            } => self.prev_diff(),
            _ => {}
        }
        false
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
            "↑/k ↓/j PgUp/b PgDn/f Ctrl+U/D g/G n/] N/[ ? q/Esc"
        } else {
            "↑/k ↓/j move  n/] next diff  N/[ prev diff  ? help  q quit"
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

fn collect_diff_rows(node: &DiffNode<'_>, rows: &mut Vec<DiffRow>) {
    let is_document = node
        .left
        .is_some_and(|node| node.kind == NodeKind::Document)
        || node
            .right
            .is_some_and(|node| node.kind == NodeKind::Document);
    if !is_document {
        rows.push(DiffRow {
            left: node.left.map(format_diff_node).unwrap_or_default(),
            right: node.right.map(format_diff_node).unwrap_or_default(),
            diff_type: node.diff_type,
            changed: node.diff_type != DiffType::Unchanged,
        });
    }
    for child in &node.children {
        collect_diff_rows(child, rows);
    }
}

fn format_diff_node(node: &YamNode) -> String {
    let indent = "  ".repeat(node.depth);
    let key = node.key.clone().unwrap_or_else(|| {
        if node.depth > 0 {
            format!("[{}]", node.index)
        } else {
            String::new()
        }
    });
    match node.kind {
        NodeKind::Mapping => {
            if key.is_empty() {
                format!("{indent}{{...}}")
            } else {
                format!("{indent}{key}:")
            }
        }
        NodeKind::Sequence => {
            if key.is_empty() {
                format!("{indent}[...]")
            } else {
                format!("{indent}{key}:")
            }
        }
        NodeKind::Scalar => {
            if key.is_empty() {
                format!("{indent}{}", node.value.clone().unwrap_or_default())
            } else {
                format!("{indent}{key}: {}", node.value.clone().unwrap_or_default())
            }
        }
        NodeKind::Document => String::new(),
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
    nodes: &mut Vec<NodeMeta>,
) {
    let id = nodes.len();
    nodes.push(NodeMeta {
        parent,
        child_index,
        end: 0,
    });
    for (idx, child) in node.children.iter().enumerate() {
        index_nodes(child, Some(id), idx, nodes);
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
    if node.kind != NodeKind::Document && (hit(&node.key) || hit(&node.value)) {
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
    use crate::parser::{get_by_path, parse_yaml_str};

    fn app(yaml: &str) -> TreeApp {
        TreeApp::new(
            parse_yaml_str(yaml).unwrap(),
            Some(PathBuf::from("x.yaml")),
            "x.yaml".into(),
            Vec::new(),
            TreeStyle::Unicode,
            false,
        )
    }

    fn press(app: &mut TreeApp, code: KeyCode) {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
            .unwrap();
    }

    #[test]
    fn tree_app_flattens_collapsed_nodes() {
        let mut app = app("a:\n  b: 1\nc: 2\n");
        assert_eq!(app.rows.len(), 4);
        app.cursor = 1;
        app.toggle_current();
        assert_eq!(app.rows.len(), 3);
        assert_eq!(app.render_node_line(1), "▶ a:");
    }

    #[test]
    fn tree_app_edits_scalar_and_tracks_undo() {
        let mut app = app("a: old\n");
        app.edit_id = Some(1);
        app.edit_original = "old".into();
        app.input = "new".into();
        app.confirm_edit();

        assert_eq!(
            get_by_path(&app.root, ".a").unwrap().value.as_deref(),
            Some("new")
        );
        assert!(app.modified_ids.contains(&1));
        app.undo();
        assert_eq!(
            get_by_path(&app.root, ".a").unwrap().value.as_deref(),
            Some("old")
        );
    }

    #[test]
    fn navigation_reuses_rows_regardless_of_document_size() {
        for size in [10, 10_000] {
            let yaml: String = (0..size).map(|i| format!("k{i}:\n  v: {i}\n")).collect();
            let mut app = app(&yaml);
            assert_eq!(app.rows.len(), 1 + 2 * size);
            for _ in 0..100 {
                press(&mut app, KeyCode::Char('j'));
            }
            press(&mut app, KeyCode::Char('G'));
            press(&mut app, KeyCode::Char('g'));
            assert_eq!(app.row_builds, 1, "size {size}");

            press(&mut app, KeyCode::Char('j'));
            press(&mut app, KeyCode::Enter);
            assert_eq!(app.row_builds, 2, "size {size}");
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
        assert_eq!(app.display_path(4), "$.items.0.name");
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
        for code in [
            KeyCode::Char('j'),
            KeyCode::Char('k'),
            KeyCode::Char('G'),
            KeyCode::Enter,
        ] {
            press(&mut app, code);
        }
        assert_eq!(app.cursor, 0);
        assert_eq!(app.row_builds, 1, "a scalar root cannot be folded");
    }

    #[test]
    fn row_names_follow_depth_boundaries() {
        // Depth 0: the root has no key and no index; depth 1 and below show an
        // index for sequence items and indent one level less than their depth.
        let app = app("- a\n- [b]\n");
        let lines: Vec<String> = app
            .rows
            .iter()
            .map(|&id| app.render_node_line(id))
            .collect();
        assert_eq!(lines, ["▼ ", "  [0] a", "▼ [1]", "    [0] b"]);

        assert_eq!(self::app("42\n").render_node_line(0), "  42");
        assert_eq!(self::app("a: 1\n").render_node_line(1), "  a: 1");
    }

    #[test]
    fn display_path_includes_the_queried_base_path() {
        let root = parse_yaml_str("metadata:\n  name: yam\n").unwrap();
        let node = get_by_path(&root, ".metadata").unwrap().clone();
        let app = TreeApp::new(
            node,
            None,
            "x.yaml".into(),
            vec!["metadata".into()],
            TreeStyle::Unicode,
            false,
        );
        assert_eq!(app.display_path(0), "$.metadata");
        assert_eq!(app.display_path(1), "$.metadata.name");
    }
}
