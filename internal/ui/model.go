package ui

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/bubbles/help"
	"github.com/charmbracelet/bubbles/key"
	"github.com/charmbracelet/bubbles/textinput"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"github.com/simota/yam/internal/parser"
	"github.com/simota/yam/internal/renderer"
)

// Model represents the TUI application state
type Model struct {
	root      *parser.YamNode
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
}

// NewModel creates a new TUI model
func NewModel(root *parser.YamNode, filename string, treeStyle renderer.TreeStyle, showTypes bool) Model {
	opts := renderer.DefaultOptions()
	opts.TreeStyle = treeStyle
	opts.Interactive = true
	opts.ShowTypes = showTypes

	ti := textinput.New()
	ti.Placeholder = "search..."
	ti.Prompt = "/"
	ti.CharLimit = 100

	m := Model{
		root:        root,
		filename:    filename,
		renderer:    renderer.New(nil, opts),
		keyMap:      DefaultKeyMap(),
		help:        help.New(),
		searchInput: ti,
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
			return m, tea.Quit

		case key.Matches(msg, m.keyMap.Help):
			m.showHelp = !m.showHelp

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
	b.WriteString(headerStyle.Render(fmt.Sprintf(" yam - %s", m.filename)))
	b.WriteString("\n")

	// Styles for content
	cursorStyle := lipgloss.NewStyle().
		Background(lipgloss.Color("#30363D")).
		Width(m.width)
	matchStyle := lipgloss.NewStyle().
		Background(lipgloss.Color("#3D3200")).
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

			// Apply styles: cursor takes priority over match
			if isCursor {
				line = cursorStyle.Render(line)
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

	if m.searchMode {
		// Search input display
		searchLine := m.searchInput.View()
		if len(m.matches) > 0 {
			searchLine += fmt.Sprintf("  [%d/%d]", m.matchIndex+1, len(m.matches))
		} else if m.searchInput.Value() != "" {
			searchLine += "  [no matches]"
		}
		b.WriteString(footerStyle.Render(searchLine))
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
