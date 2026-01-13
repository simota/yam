package ui

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/bubbles/help"
	"github.com/charmbracelet/bubbles/key"
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
}

// NewModel creates a new TUI model
func NewModel(root *parser.YamNode, filename string, treeStyle renderer.TreeStyle) Model {
	opts := renderer.DefaultOptions()
	opts.TreeStyle = treeStyle
	opts.Interactive = true

	m := Model{
		root:     root,
		filename: filename,
		renderer: renderer.New(nil, opts),
		keyMap:   DefaultKeyMap(),
		help:     help.New(),
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
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		m.width = msg.Width
		m.height = msg.Height
		m.help.Width = msg.Width

	case tea.KeyMsg:
		switch {
		case key.Matches(msg, m.keyMap.Quit):
			return m, tea.Quit

		case key.Matches(msg, m.keyMap.Help):
			m.showHelp = !m.showHelp

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

	return m, nil
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

	// Content
	vh := m.viewportHeight()
	contentLines := m.renderContent()

	// Pad or truncate to viewport height
	for i := 0; i < vh; i++ {
		idx := m.offset + i
		if idx < len(contentLines) {
			line := contentLines[idx]
			// Highlight current line
			if idx == m.cursor {
				line = lipgloss.NewStyle().
					Background(lipgloss.Color("#30363D")).
					Width(m.width).
					Render(line)
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

	position := fmt.Sprintf("%d/%d", m.cursor+1, len(m.flatNodes))
	if m.cursor >= 0 && m.cursor < len(m.flatNodes) {
		node := m.flatNodes[m.cursor]
		position += " | " + node.PathString()
	}
	b.WriteString(footerStyle.Render(position))
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
