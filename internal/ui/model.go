package ui

import (
	"fmt"
	"os"
	"strings"

	"github.com/charmbracelet/bubbles/help"
	"github.com/charmbracelet/bubbles/key"
	"github.com/charmbracelet/bubbles/textinput"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"github.com/simota/yam/internal/parser"
	"github.com/simota/yam/internal/renderer"
	"gopkg.in/yaml.v3"
)

// maxUndoStackSize is the maximum number of undo entries to keep
const maxUndoStackSize = 10

// UndoEntry represents a single undoable edit action
type UndoEntry struct {
	Node     *parser.YamNode
	OldValue string
	NewValue string
}

// Model represents the TUI application state
type Model struct {
	root      *parser.YamNode
	rawRoot   *yaml.Node // original yaml.Node for saving
	flatNodes []*parser.YamNode
	cursor    int
	offset    int
	width     int
	height    int
	filename  string
	renderer  *renderer.Renderer
	keyMap    KeyMap
	help      help.Model
	showHelp  bool

	// Search state
	searchMode  bool
	searchInput textinput.Model
	matches     []int // indices in flatNodes that match
	matchIndex  int   // current position in matches

	// Edit state
	editMode      bool
	editInput     textinput.Model
	editNode      *parser.YamNode
	originalValue string

	// Dirty state
	modified      bool
	modifiedNodes map[*parser.YamNode]bool
	statusMessage string // temporary status message

	// Undo/Redo state
	undoStack []UndoEntry
	redoStack []UndoEntry
}

// NewModel creates a new TUI model
func NewModel(root *parser.YamNode, filename string, treeStyle renderer.TreeStyle, showTypes bool) Model {
	opts := renderer.DefaultOptions()
	opts.TreeStyle = treeStyle
	opts.Interactive = true
	opts.ShowTypes = showTypes

	searchTi := textinput.New()
	searchTi.Placeholder = "search..."
	searchTi.Prompt = "/"
	searchTi.CharLimit = 100

	editTi := textinput.New()
	editTi.Placeholder = ""
	editTi.Prompt = "Edit: "
	editTi.CharLimit = 500

	m := Model{
		root:          root,
		rawRoot:       root.Raw,
		filename:      filename,
		renderer:      renderer.New(nil, opts),
		keyMap:        DefaultKeyMap(),
		help:          help.New(),
		searchInput:   searchTi,
		editInput:     editTi,
		modifiedNodes: make(map[*parser.YamNode]bool),
	}
	m.rebuildFlatList()
	return m
}

func (m *Model) rebuildFlatList() {
	m.flatNodes = parser.FlattenVisible(m.root)
	// Skip document node if present
	if len(m.flatNodes) > 0 && m.flatNodes[0].Kind() == parser.KindDocument {
		m.flatNodes = m.flatNodes[1:]
	}
}

// Init implements tea.Model
func (m Model) Init() tea.Cmd {
	return nil
}

// Update implements tea.Model
func (m Model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	var cmd tea.Cmd

	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		m.width = msg.Width
		m.height = msg.Height
		m.help.Width = msg.Width

	case tea.KeyMsg:
		// Clear status message on any key press
		m.statusMessage = ""

		// Edit mode handling
		if m.editMode {
			switch msg.Type {
			case tea.KeyEnter:
				// Confirm edit
				m.confirmEdit()
				return m, nil
			case tea.KeyEsc:
				// Cancel edit
				m.editMode = false
				m.editInput.Blur()
				m.editNode = nil
				return m, nil
			default:
				// Update text input
				m.editInput, cmd = m.editInput.Update(msg)
				return m, cmd
			}
		}

		// Search mode handling
		if m.searchMode {
			switch msg.Type {
			case tea.KeyEnter:
				// Confirm search, jump to first match
				m.searchMode = false
				m.searchInput.Blur()
				if len(m.matches) > 0 {
					m.matchIndex = 0
					m.cursor = m.matches[0]
					m.adjustOffset()
				}
				return m, nil
			case tea.KeyEsc:
				// Cancel search
				m.searchMode = false
				m.searchInput.Blur()
				m.clearSearch()
				return m, nil
			default:
				// Update text input and perform incremental search
				m.searchInput, cmd = m.searchInput.Update(msg)
				m.search(m.searchInput.Value())
				return m, cmd
			}
		}

		// Normal mode handling
		switch {
		case key.Matches(msg, m.keyMap.Quit):
			// Confirm quit if modified
			if m.modified {
				m.statusMessage = "Unsaved changes! Press q again to quit, or Ctrl+S to save"
				m.modified = false // Allow quit on next q press
				return m, nil
			}
			return m, tea.Quit

		case key.Matches(msg, m.keyMap.Help):
			m.showHelp = !m.showHelp

		case key.Matches(msg, m.keyMap.Edit):
			m.startEdit()
			if m.editMode {
				return m, textinput.Blink
			}

		case key.Matches(msg, m.keyMap.Save):
			m.saveFile()

		case key.Matches(msg, m.keyMap.Undo):
			m.undo()

		case key.Matches(msg, m.keyMap.Redo):
			m.redo()

		case key.Matches(msg, m.keyMap.Search):
			m.searchMode = true
			m.searchInput.Focus()
			return m, textinput.Blink

		case key.Matches(msg, m.keyMap.NextMatch):
			m.nextMatch()

		case key.Matches(msg, m.keyMap.PrevMatch):
			m.prevMatch()

		case key.Matches(msg, m.keyMap.Up):
			m.moveCursor(-1)

		case key.Matches(msg, m.keyMap.Down):
			m.moveCursor(1)

		case key.Matches(msg, m.keyMap.PageUp):
			m.moveCursor(-m.viewportHeight())

		case key.Matches(msg, m.keyMap.PageDown):
			m.moveCursor(m.viewportHeight())

		case key.Matches(msg, m.keyMap.HalfUp):
			m.moveCursor(-m.viewportHeight() / 2)

		case key.Matches(msg, m.keyMap.HalfDown):
			m.moveCursor(m.viewportHeight() / 2)

		case key.Matches(msg, m.keyMap.Top):
			m.cursor = 0
			m.offset = 0

		case key.Matches(msg, m.keyMap.Bottom):
			m.cursor = len(m.flatNodes) - 1
			m.adjustOffset()

		case key.Matches(msg, m.keyMap.Toggle):
			m.toggleCurrent()

		case key.Matches(msg, m.keyMap.ExpandAll):
			m.expandAll()

		case key.Matches(msg, m.keyMap.CollapseAll):
			m.collapseAll()
		}
	}

	return m, cmd
}

func (m *Model) moveCursor(delta int) {
	m.cursor += delta
	if m.cursor < 0 {
		m.cursor = 0
	}
	if m.cursor >= len(m.flatNodes) {
		m.cursor = len(m.flatNodes) - 1
	}
	m.adjustOffset()
}

func (m *Model) adjustOffset() {
	vh := m.viewportHeight()
	if m.cursor < m.offset {
		m.offset = m.cursor
	}
	if m.cursor >= m.offset+vh {
		m.offset = m.cursor - vh + 1
	}
}

func (m *Model) viewportHeight() int {
	return m.height - 4 // header + footer + help
}

func (m *Model) toggleCurrent() {
	if m.cursor < 0 || m.cursor >= len(m.flatNodes) {
		return
	}
	node := m.flatNodes[m.cursor]
	if node.IsContainer() && node.HasChildren() {
		node.Collapsed = !node.Collapsed
		m.rebuildFlatList()
		// Adjust cursor if it's now out of bounds
		if m.cursor >= len(m.flatNodes) {
			m.cursor = len(m.flatNodes) - 1
		}
	}
}

func (m *Model) expandAll() {
	parser.Walk(m.root, func(n *parser.YamNode) bool {
		n.Collapsed = false
		return true
	})
	m.rebuildFlatList()
}

func (m *Model) collapseAll() {
	parser.Walk(m.root, func(n *parser.YamNode) bool {
		if n.IsContainer() && n.HasChildren() && n.Depth > 0 {
			n.Collapsed = true
		}
		return true
	})
	m.rebuildFlatList()
	if m.cursor >= len(m.flatNodes) {
		m.cursor = len(m.flatNodes) - 1
	}
	m.offset = 0
}

// search searches all nodes (including collapsed) and auto-expands parents of matches
func (m *Model) search(query string) {
	m.matches = nil
	m.matchIndex = 0
	if query == "" {
		return
	}
	query = strings.ToLower(query)

	// Walk entire tree (including collapsed nodes)
	var matchedNodes []*parser.YamNode
	parser.Walk(m.root, func(node *parser.YamNode) bool {
		if node.Kind() == parser.KindDocument {
			return true
		}
		// Search in key
		if strings.Contains(strings.ToLower(node.Key), query) {
			matchedNodes = append(matchedNodes, node)
			return true
		}
		// Search in value
		if strings.Contains(strings.ToLower(node.Value()), query) {
			matchedNodes = append(matchedNodes, node)
		}
		return true
	})

	// Auto-expand ancestors of matched nodes
	for _, node := range matchedNodes {
		m.expandAncestors(node)
	}

	// Rebuild flat list to reflect expanded state
	m.rebuildFlatList()

	// Map matched nodes to their indices in flatNodes
	for i, node := range m.flatNodes {
		for _, matched := range matchedNodes {
			if node == matched {
				m.matches = append(m.matches, i)
				break
			}
		}
	}
}

// expandAncestors expands all ancestors of a node
func (m *Model) expandAncestors(node *parser.YamNode) {
	for p := node.Parent; p != nil; p = p.Parent {
		p.Collapsed = false
	}
}

// nextMatch moves to the next search match
func (m *Model) nextMatch() {
	if len(m.matches) == 0 {
		return
	}
	m.matchIndex = (m.matchIndex + 1) % len(m.matches)
	m.cursor = m.matches[m.matchIndex]
	m.adjustOffset()
}

// prevMatch moves to the previous search match
func (m *Model) prevMatch() {
	if len(m.matches) == 0 {
		return
	}
	m.matchIndex--
	if m.matchIndex < 0 {
		m.matchIndex = len(m.matches) - 1
	}
	m.cursor = m.matches[m.matchIndex]
	m.adjustOffset()
}

// isMatchIndex returns true if the given index is in the matches list
func (m *Model) isMatchIndex(idx int) bool {
	for _, i := range m.matches {
		if i == idx {
			return true
		}
	}
	return false
}

// clearSearch clears search state
func (m *Model) clearSearch() {
	m.matches = nil
	m.matchIndex = 0
	m.searchInput.SetValue("")
}

// startEdit starts editing the current node if it's a scalar value
func (m *Model) startEdit() {
	if m.cursor < 0 || m.cursor >= len(m.flatNodes) {
		return
	}

	node := m.flatNodes[m.cursor]

	// Check if node is editable (scalar value only)
	if !m.isEditable(node) {
		m.statusMessage = "Cannot edit: not a scalar value"
		return
	}

	// Check if file is from stdin
	if m.filename == "stdin" || m.filename == "-" {
		m.statusMessage = "Cannot edit: read-only (stdin)"
		return
	}

	m.editMode = true
	m.editNode = node
	m.originalValue = node.Value()
	m.editInput.SetValue(node.Value())
	m.editInput.Focus()
	m.editInput.CursorEnd()
}

// isEditable checks if a node can be edited (scalar values only)
func (m *Model) isEditable(node *parser.YamNode) bool {
	if node == nil || node.Raw == nil {
		return false
	}
	kind := node.Kind()
	return kind == parser.KindScalar
}

// confirmEdit confirms the edit and updates the node value
func (m *Model) confirmEdit() {
	if m.editNode == nil {
		return
	}

	newValue := m.editInput.Value()

	// Only mark as modified if value actually changed
	if newValue != m.originalValue {
		// Push to undo stack before modifying
		entry := UndoEntry{
			Node:     m.editNode,
			OldValue: m.originalValue,
			NewValue: newValue,
		}
		m.pushUndo(entry)

		// Update the yaml.Node value
		m.editNode.Raw.Value = newValue

		// Mark as modified
		m.modified = true
		m.modifiedNodes[m.editNode] = true
	}

	// Exit edit mode
	m.editMode = false
	m.editInput.Blur()
	m.editNode = nil
	m.originalValue = ""
}

// saveFile saves the modified YAML to the original file
func (m *Model) saveFile() {
	// Check if file is from stdin
	if m.filename == "stdin" || m.filename == "-" {
		m.statusMessage = "Cannot save: read-only (stdin)"
		return
	}

	if !m.modified && len(m.modifiedNodes) == 0 {
		m.statusMessage = "No changes to save"
		return
	}

	// Open file for writing
	file, err := os.Create(m.filename)
	if err != nil {
		m.statusMessage = fmt.Sprintf("Error: %v", err)
		return
	}
	defer file.Close()

	// Encode with yaml.v3
	encoder := yaml.NewEncoder(file)
	encoder.SetIndent(2)
	defer encoder.Close()

	if err := encoder.Encode(m.rawRoot); err != nil {
		m.statusMessage = fmt.Sprintf("Error: %v", err)
		return
	}

	// Clear modified state
	m.modified = false
	m.modifiedNodes = make(map[*parser.YamNode]bool)
	m.statusMessage = "Saved!"
}

// isModifiedNode checks if a node has been modified
func (m *Model) isModifiedNode(node *parser.YamNode) bool {
	return m.modifiedNodes[node]
}

// pushUndo adds an entry to the undo stack
func (m *Model) pushUndo(entry UndoEntry) {
	m.undoStack = append(m.undoStack, entry)
	if len(m.undoStack) > maxUndoStackSize {
		m.undoStack = m.undoStack[1:]
	}
	// Clear redo stack when new edit is made
	m.redoStack = nil
}

// undo reverts the last edit
func (m *Model) undo() {
	if len(m.undoStack) == 0 {
		m.statusMessage = "Nothing to undo"
		return
	}

	// Pop from undo stack
	entry := m.undoStack[len(m.undoStack)-1]
	m.undoStack = m.undoStack[:len(m.undoStack)-1]

	// Restore old value
	entry.Node.Raw.Value = entry.OldValue

	// Push to redo stack
	m.redoStack = append(m.redoStack, entry)

	// Update modified state
	m.updateModifiedState()

	m.statusMessage = "Undo: restored value"
}

// redo re-applies a previously undone edit
func (m *Model) redo() {
	if len(m.redoStack) == 0 {
		m.statusMessage = "Nothing to redo"
		return
	}

	// Pop from redo stack
	entry := m.redoStack[len(m.redoStack)-1]
	m.redoStack = m.redoStack[:len(m.redoStack)-1]

	// Re-apply new value
	entry.Node.Raw.Value = entry.NewValue

	// Push back to undo stack
	m.undoStack = append(m.undoStack, entry)

	// Update modified state
	m.modified = true
	m.modifiedNodes[entry.Node] = true

	m.statusMessage = "Redo: re-applied value"
}

// updateModifiedState recalculates the modified state based on undo history
func (m *Model) updateModifiedState() {
	// Check if any nodes are still modified
	// A node is modified if it appears in undoStack with a different current value
	m.modifiedNodes = make(map[*parser.YamNode]bool)
	for _, entry := range m.undoStack {
		// Node is modified if current value differs from original
		if entry.Node.Raw.Value != entry.OldValue {
			m.modifiedNodes[entry.Node] = true
		}
	}
	m.modified = len(m.modifiedNodes) > 0
}

// View implements tea.Model
func (m Model) View() string {
	if m.width == 0 {
		return "Loading..."
	}

	var b strings.Builder

	// Header
	headerStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#79C0FF")).
		Background(lipgloss.Color("#21262D")).
		Padding(0, 1).
		Width(m.width)

	headerText := fmt.Sprintf(" yam - %s", m.filename)
	if m.modified || len(m.modifiedNodes) > 0 {
		headerText += " [modified]"
	}
	if m.filename == "stdin" || m.filename == "-" {
		headerText += " [read-only]"
	}
	b.WriteString(headerStyle.Render(headerText))
	b.WriteString("\n")

	// Styles for content
	cursorStyle := lipgloss.NewStyle().
		Background(lipgloss.Color("#30363D")).
		Width(m.width)
	matchStyle := lipgloss.NewStyle().
		Background(lipgloss.Color("#3D3200")).
		Width(m.width)
	modifiedStyle := lipgloss.NewStyle().
		Background(lipgloss.Color("#3D2800")).
		Width(m.width)

	// Content
	vh := m.viewportHeight()
	contentLines := m.renderContent()

	// Pad or truncate to viewport height
	for i := 0; i < vh; i++ {
		idx := m.offset + i
		if idx < len(contentLines) {
			line := contentLines[idx]
			isMatch := m.isMatchIndex(idx)
			isCursor := idx == m.cursor
			isModified := idx < len(m.flatNodes) && m.isModifiedNode(m.flatNodes[idx])

			// Apply styles: cursor takes priority, then modified, then match
			if isCursor {
				line = cursorStyle.Render(line)
			} else if isModified {
				line = modifiedStyle.Render(line)
			} else if isMatch {
				line = matchStyle.Render(line)
			}
			b.WriteString(line)
		}
		b.WriteString("\n")
	}

	// Footer
	footerStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#8B949E")).
		Background(lipgloss.Color("#21262D")).
		Padding(0, 1).
		Width(m.width)

	if m.editMode {
		// Edit input display
		editLine := m.editInput.View() + "  [Enter: confirm, Esc: cancel]"
		b.WriteString(footerStyle.Render(editLine))
	} else if m.searchMode {
		// Search input display
		searchLine := m.searchInput.View()
		if len(m.matches) > 0 {
			searchLine += fmt.Sprintf("  [%d/%d]", m.matchIndex+1, len(m.matches))
		} else if m.searchInput.Value() != "" {
			searchLine += "  [no matches]"
		}
		b.WriteString(footerStyle.Render(searchLine))
	} else if m.statusMessage != "" {
		// Status message display
		b.WriteString(footerStyle.Render(m.statusMessage))
	} else {
		// Normal footer
		position := fmt.Sprintf("%d/%d", m.cursor+1, len(m.flatNodes))
		if m.cursor >= 0 && m.cursor < len(m.flatNodes) {
			node := m.flatNodes[m.cursor]
			position += " | " + node.PathString()
		}
		// Show match info if matches exist
		if len(m.matches) > 0 {
			position += fmt.Sprintf("  [match %d/%d]", m.matchIndex+1, len(m.matches))
		}
		// Show modified indicator with save hint
		if m.modified || len(m.modifiedNodes) > 0 {
			position += "  [modified - Ctrl+S to save]"
		}
		b.WriteString(footerStyle.Render(position))
	}
	b.WriteString("\n")

	// Help
	if m.showHelp {
		b.WriteString(m.help.View(m.keyMap))
	} else {
		b.WriteString(m.help.ShortHelpView(m.keyMap.ShortHelp()))
	}

	return b.String()
}

func (m Model) renderContent() []string {
	output := m.renderer.RenderVisible(m.root)
	lines := strings.Split(output, "\n")
	// Remove empty last line
	if len(lines) > 0 && lines[len(lines)-1] == "" {
		lines = lines[:len(lines)-1]
	}
	return lines
}
