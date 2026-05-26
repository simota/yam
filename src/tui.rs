use crate::diff::{DiffNode, DiffResult, DiffType};
use crate::format::{format_yaml, FormatOptions};
use crate::parser::{get_by_segments, get_by_segments_mut, NodeKind, ScalarType, YamNode};
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
use std::time::Duration;

const MAX_UNDO: usize = 10;

#[derive(Debug, Clone)]
struct FlatNode {
    path: Vec<String>,
    line: String,
}

#[derive(Debug, Clone)]
struct UndoEntry {
    path: Vec<String>,
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
    style: TreeStyle,
    show_types: bool,
) -> Result<()> {
    let _guard = TerminalGuard::enter()?;
    TreeApp::new(root, filename, display_name, style, show_types).run()
}

pub fn run_diff(result: &DiffResult<'_>) -> Result<()> {
    let _guard = TerminalGuard::enter()?;
    DiffApp::new(result).run()
}

struct TreeApp {
    root: YamNode,
    filename: Option<PathBuf>,
    display_name: String,
    style: TreeStyle,
    show_types: bool,
    collapsed: HashSet<String>,
    cursor: usize,
    offset: usize,
    show_help: bool,
    mode: InputMode,
    input: String,
    edit_path: Vec<String>,
    edit_original: String,
    modified_paths: HashSet<String>,
    modified: bool,
    quit_armed: bool,
    status: String,
    matches: Vec<Vec<String>>,
    match_index: usize,
    undo_stack: Vec<UndoEntry>,
    redo_stack: Vec<UndoEntry>,
}

impl TreeApp {
    fn new(
        root: YamNode,
        filename: Option<PathBuf>,
        display_name: String,
        style: TreeStyle,
        show_types: bool,
    ) -> Self {
        Self {
            root,
            filename,
            display_name,
            style,
            show_types,
            collapsed: HashSet::new(),
            cursor: 0,
            offset: 0,
            show_help: false,
            mode: InputMode::Normal,
            input: String::new(),
            edit_path: Vec::new(),
            edit_original: String::new(),
            modified_paths: HashSet::new(),
            modified: false,
            quit_armed: false,
            status: String::new(),
            matches: Vec::new(),
            match_index: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    fn run(&mut self) -> Result<()> {
        loop {
            self.draw()?;
            if !event::poll(Duration::from_millis(250))? {
                continue;
            }
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

        let rows = self.flat_nodes();
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
                self.cursor = rows.len().saturating_sub(1);
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
            } => self.collapsed.clear(),
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
                self.edit_path.clear();
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
        let rows = self.flat_nodes();
        self.clamp_cursor(rows.len());
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
            if row_idx >= rows.len() {
                continue;
            }
            let row = &rows[row_idx];
            if row_idx == self.cursor {
                queue!(stdout, SetAttribute(Attribute::Reverse))?;
            } else if self.modified_paths.contains(&path_key(&row.path)) {
                queue!(stdout, SetForegroundColor(Color::Yellow))?;
            } else if self.matches.iter().any(|path| path == &row.path) {
                queue!(stdout, SetForegroundColor(Color::DarkYellow))?;
            }
            queue!(stdout, Print(truncate(&row.line, width as usize)))?;
            queue!(stdout, ResetColor, SetAttribute(Attribute::Reset))?;
        }

        let footer_y = height.saturating_sub(2);
        let footer = self.footer_text(rows.len());
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

    fn footer_text(&self, total: usize) -> String {
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
                let rows = self.flat_nodes();
                let pos = format!("{}/{}", self.cursor.saturating_add(1), total);
                let path = rows
                    .get(self.cursor)
                    .map(|row| display_path(&row.path))
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

    fn flat_nodes(&self) -> Vec<FlatNode> {
        let mut rows = Vec::new();
        let root = if self.root.kind == NodeKind::Document {
            self.root.children.first()
        } else {
            Some(&self.root)
        };
        if let Some(root) = root {
            self.collect_visible(root, &mut rows);
        }
        rows
    }

    fn collect_visible(&self, node: &YamNode, rows: &mut Vec<FlatNode>) {
        rows.push(FlatNode {
            path: node.path.clone(),
            line: self.render_node_line(node),
        });
        if node.has_children() && !self.collapsed.contains(&path_key(&node.path)) {
            for child in &node.children {
                self.collect_visible(child, rows);
            }
        }
    }

    fn render_node_line(&self, node: &YamNode) -> String {
        let depth = node.depth.saturating_sub(1);
        let indent = "  ".repeat(depth);
        let indicator = if node.has_children() {
            let (collapsed, expanded) = match self.style {
                TreeStyle::Unicode => ("▶ ", "▼ "),
                TreeStyle::Ascii | TreeStyle::Indent => ("+ ", "- "),
            };
            if self.collapsed.contains(&path_key(&node.path)) {
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

    fn move_cursor(&mut self, delta: isize) {
        let len = self.flat_nodes().len();
        if len == 0 {
            self.cursor = 0;
            self.offset = 0;
            return;
        }
        self.cursor =
            (self.cursor as isize + delta).clamp(0, len.saturating_sub(1) as isize) as usize;
        self.adjust_offset();
    }

    fn clamp_cursor(&mut self, len: usize) {
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
            .map(|(_, h)| h.saturating_sub(3).max(1) as usize)
            .unwrap_or(20)
    }

    fn current_path(&self) -> Option<Vec<String>> {
        self.flat_nodes()
            .get(self.cursor)
            .map(|row| row.path.clone())
    }

    fn toggle_current(&mut self) {
        let Some(path) = self.current_path() else {
            return;
        };
        let Some(node) = get_by_segments(&self.root, &path) else {
            return;
        };
        if !node.is_container() || !node.has_children() {
            return;
        }
        let key = path_key(&path);
        if !self.collapsed.remove(&key) {
            self.collapsed.insert(key);
        }
    }

    fn collapse_all(&mut self) {
        self.collapsed.clear();
        let mut paths = Vec::new();
        collect_container_paths(&self.root, &mut paths);
        for path in paths {
            if !path.is_empty() {
                self.collapsed.insert(path_key(&path));
            }
        }
        self.cursor = 0;
        self.offset = 0;
    }

    fn start_edit(&mut self) {
        let Some(path) = self.current_path() else {
            return;
        };
        let Some(node) = get_by_segments(&self.root, &path) else {
            return;
        };
        if node.kind != NodeKind::Scalar {
            self.status = "Cannot edit: not a scalar value".into();
            return;
        }
        if self.filename.is_none() {
            self.status = "Cannot edit: read-only (stdin)".into();
            return;
        }
        self.mode = InputMode::Edit;
        self.edit_path = path;
        self.edit_original = node.value.clone().unwrap_or_default();
        self.input = self.edit_original.clone();
    }

    fn confirm_edit(&mut self) {
        if self.input == self.edit_original {
            self.input.clear();
            self.edit_path.clear();
            self.edit_original.clear();
            return;
        }
        if let Some(node) = get_by_segments_mut(&mut self.root, &self.edit_path) {
            node.value = Some(self.input.clone());
            let entry = UndoEntry {
                path: self.edit_path.clone(),
                old_value: self.edit_original.clone(),
                new_value: self.input.clone(),
            };
            self.push_undo(entry);
            self.modified = true;
            self.modified_paths.insert(path_key(&self.edit_path));
        }
        self.input.clear();
        self.edit_path.clear();
        self.edit_original.clear();
    }

    fn save_file(&mut self) -> Result<()> {
        let Some(path) = &self.filename else {
            self.status = "Cannot save: read-only (stdin)".into();
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
        self.modified_paths.clear();
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
        if let Some(node) = get_by_segments_mut(&mut self.root, &entry.path) {
            node.value = Some(entry.old_value.clone());
            self.modified = true;
            self.modified_paths.insert(path_key(&entry.path));
            self.status = "Undo: restored value".into();
            self.redo_stack.push(entry);
        }
    }

    fn redo(&mut self) {
        let Some(entry) = self.redo_stack.pop() else {
            self.status = "Nothing to redo".into();
            return;
        };
        if let Some(node) = get_by_segments_mut(&mut self.root, &entry.path) {
            node.value = Some(entry.new_value.clone());
            self.modified = true;
            self.modified_paths.insert(path_key(&entry.path));
            self.status = "Redo: re-applied value".into();
            self.undo_stack.push(entry);
        }
    }

    fn update_search_matches(&mut self) {
        self.matches.clear();
        if self.input.is_empty() {
            return;
        }
        let needle = self.input.to_ascii_lowercase();
        collect_matching_paths(&self.root, &needle, &mut self.matches);
    }

    fn finish_search(&mut self) {
        self.update_search_matches();
        if self.matches.is_empty() {
            self.status = "No matches".into();
            return;
        }
        self.match_index = 0;
        let target = self.matches[0].clone();
        self.expand_ancestors(&target);
        self.jump_to_path(&target);
    }

    fn next_match(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        self.match_index = (self.match_index + 1) % self.matches.len();
        let target = self.matches[self.match_index].clone();
        self.expand_ancestors(&target);
        self.jump_to_path(&target);
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
        let target = self.matches[self.match_index].clone();
        self.expand_ancestors(&target);
        self.jump_to_path(&target);
    }

    fn expand_ancestors(&mut self, path: &[String]) {
        for len in 0..path.len() {
            self.collapsed.remove(&path_key(&path[..len]));
        }
    }

    fn jump_to_path(&mut self, target: &[String]) {
        if let Some(index) = self
            .flat_nodes()
            .iter()
            .position(|row| row.path.as_slice() == target)
        {
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
            if !event::poll(Duration::from_millis(250))? {
                continue;
            }
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
            .map(|(_, h)| h.saturating_sub(3).max(1) as usize)
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

fn collect_container_paths(node: &YamNode, paths: &mut Vec<Vec<String>>) {
    if node.is_container() && node.has_children() {
        paths.push(node.path.clone());
    }
    for child in &node.children {
        collect_container_paths(child, paths);
    }
}

fn collect_matching_paths(node: &YamNode, needle: &str, matches: &mut Vec<Vec<String>>) {
    if node.kind != NodeKind::Document {
        let key_hit = node
            .key
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .contains(needle);
        let value_hit = node
            .value
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .contains(needle);
        if key_hit || value_hit {
            matches.push(node.path.clone());
        }
    }
    for child in &node.children {
        collect_matching_paths(child, needle, matches);
    }
}

fn path_key(path: &[String]) -> String {
    path.join("\u{1f}")
}

fn display_path(path: &[String]) -> String {
    if path.is_empty() {
        "$".into()
    } else {
        format!("$.{}", path.join("."))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_yaml_str;

    #[test]
    fn tree_app_flattens_collapsed_nodes() {
        let root = parse_yaml_str("a:\n  b: 1\nc: 2\n").unwrap();
        let mut app = TreeApp::new(root, None, "stdin".into(), TreeStyle::Unicode, false);
        assert_eq!(app.flat_nodes().len(), 4);
        app.collapsed.insert(path_key(&["a".to_string()]));
        assert_eq!(app.flat_nodes().len(), 3);
    }

    #[test]
    fn tree_app_edits_scalar_and_tracks_undo() {
        let root = parse_yaml_str("a: old\n").unwrap();
        let mut app = TreeApp::new(
            root,
            Some(PathBuf::from("x.yaml")),
            "x.yaml".into(),
            TreeStyle::Unicode,
            false,
        );
        app.edit_path = vec!["a".into()];
        app.edit_original = "old".into();
        app.input = "new".into();
        app.confirm_edit();

        assert_eq!(
            get_by_segments(&app.root, &["a".into()])
                .unwrap()
                .value
                .as_deref(),
            Some("new")
        );
        app.undo();
        assert_eq!(
            get_by_segments(&app.root, &["a".into()])
                .unwrap()
                .value
                .as_deref(),
            Some("old")
        );
    }
}
