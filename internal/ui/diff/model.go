package diff

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/bubbles/help"
	"github.com/charmbracelet/bubbles/key"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"github.com/simota/yam/internal/diff"
	"github.com/simota/yam/internal/parser"
)

// Model represents the diff TUI application state
type Model struct {
	result    *diff.DiffResult
	leftRoot  *parser.YamNode
	rightRoot *parser.YamNode

	// Flattened diff nodes for navigation
	diffNodes []*diff.DiffNode
	cursor    int
	offset    int

	// Window dimensions
	width  int
	height int

	// Key bindings and help
	keyMap   KeyMap
	help     help.Model
	showHelp bool
}

// NewModel creates a new diff TUI model
func NewModel(result *diff.DiffResult, left, right *parser.YamNode) Model {
	m := Model{
		result:    result,
		leftRoot:  left,
		rightRoot: right,
		keyMap:    DefaultKeyMap(),
		help:      help.New(),
	}
	m.flattenDiffNodes()
	return m
}

// flattenDiffNodes builds a flat list of diff nodes for navigation
func (m *Model) flattenDiffNodes() {
	m.diffNodes = nil
	if m.result == nil || m.result.Root == nil {
		return
	}
	m.walkDiffTree(m.result.Root)
}

func (m *Model) walkDiffTree(node *diff.DiffNode) {
	if node == nil {
		return
	}

	// Skip document nodes, add others
	if !isDocumentNode(node) {
		m.diffNodes = append(m.diffNodes, node)
	}

	for _, child := range node.Children {
		m.walkDiffTree(child)
	}
}

func isDocumentNode(node *diff.DiffNode) bool {
	if node.Left != nil && node.Left.Kind() == parser.KindDocument {
		return true
	}
	if node.Right != nil && node.Right.Kind() == parser.KindDocument {
		return true
	}
	return false
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

		case key.Matches(msg, m.keyMap.Top):
			m.cursor = 0
			m.offset = 0

		case key.Matches(msg, m.keyMap.Bottom):
			if len(m.diffNodes) > 0 {
				m.cursor = len(m.diffNodes) - 1
				m.adjustOffset()
			}

		case key.Matches(msg, m.keyMap.NextDiff):
			m.nextDiff()

		case key.Matches(msg, m.keyMap.PrevDiff):
			m.prevDiff()
		}
	}

	return m, nil
}

func (m *Model) moveCursor(delta int) {
	m.cursor += delta
	if m.cursor < 0 {
		m.cursor = 0
	}
	if m.cursor >= len(m.diffNodes) {
		m.cursor = len(m.diffNodes) - 1
	}
	if m.cursor < 0 {
		m.cursor = 0
	}
	m.adjustOffset()
}

func (m *Model) adjustOffset() {
	vh := m.viewportHeight()
	if vh <= 0 {
		return
	}
	if m.cursor < m.offset {
		m.offset = m.cursor
	}
	if m.cursor >= m.offset+vh {
		m.offset = m.cursor - vh + 1
	}
}

func (m *Model) viewportHeight() int {
	h := m.height - 5 // header + footer + summary + help
	if h < 1 {
		return 1
	}
	return h
}

// nextDiff jumps to next changed node
func (m *Model) nextDiff() {
	for i := m.cursor + 1; i < len(m.diffNodes); i++ {
		if m.diffNodes[i].Type != diff.DiffUnchanged {
			m.cursor = i
			m.adjustOffset()
			return
		}
	}
}

// prevDiff jumps to previous changed node
func (m *Model) prevDiff() {
	for i := m.cursor - 1; i >= 0; i-- {
		if m.diffNodes[i].Type != diff.DiffUnchanged {
			m.cursor = i
			m.adjustOffset()
			return
		}
	}
}

// View implements tea.Model
func (m Model) View() string {
	if m.width == 0 {
		return "Loading..."
	}

	var b strings.Builder

	// Header
	b.WriteString(m.renderHeader())
	b.WriteString("\n")

	// Split view content
	b.WriteString(m.renderSplitView())

	// Footer with summary
	b.WriteString(m.renderFooter())
	b.WriteString("\n")

	// Help
	if m.showHelp {
		b.WriteString(m.help.View(m.keyMap))
	} else {
		b.WriteString(m.help.ShortHelpView(m.keyMap.ShortHelp()))
	}

	return b.String()
}

func (m Model) renderHeader() string {
	headerStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#79C0FF")).
		Background(lipgloss.Color("#21262D")).
		Padding(0, 1).
		Width(m.width)

	leftFile := m.result.LeftFile
	rightFile := m.result.RightFile
	if leftFile == "" {
		leftFile = "(left)"
	}
	if rightFile == "" {
		rightFile = "(right)"
	}

	headerText := fmt.Sprintf(" yam diff: %s ↔ %s", leftFile, rightFile)
	return headerStyle.Render(headerText)
}

func (m Model) renderSplitView() string {
	vh := m.viewportHeight()
	halfWidth := (m.width - 3) / 2 // -3 for separator

	// Styles
	leftPaneStyle := lipgloss.NewStyle().Width(halfWidth)
	rightPaneStyle := lipgloss.NewStyle().Width(halfWidth)
	separatorStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#30363D")).
		SetString(" │ ")

	cursorStyle := lipgloss.NewStyle().
		Background(lipgloss.Color("#30363D"))

	addedStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("#A6E3A1"))
	removedStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("#F38BA8"))
	modifiedStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("#F9E2AF"))
	unchangedStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("#8B949E"))

	var lines []string

	for i := 0; i < vh; i++ {
		idx := m.offset + i
		if idx >= len(m.diffNodes) {
			// Empty line
			leftContent := strings.Repeat(" ", halfWidth)
			rightContent := strings.Repeat(" ", halfWidth)
			lines = append(lines, leftContent+separatorStyle.String()+rightContent)
			continue
		}

		node := m.diffNodes[idx]
		isCursor := idx == m.cursor

		// Get display content for each side
		leftText := m.renderNodeLeft(node, halfWidth)
		rightText := m.renderNodeRight(node, halfWidth)

		// Apply diff styling
		var style lipgloss.Style
		switch node.Type {
		case diff.DiffAdded:
			style = addedStyle
		case diff.DiffRemoved:
			style = removedStyle
		case diff.DiffModified:
			style = modifiedStyle
		default:
			style = unchangedStyle
		}

		// Get prefix based on diff type
		leftPrefix, rightPrefix := m.getDiffPrefixes(node.Type)

		// Build left side
		leftDisplay := leftPrefix + leftText
		if len(leftDisplay) > halfWidth {
			leftDisplay = leftDisplay[:halfWidth-1] + "…"
		}
		leftDisplay = style.Render(leftDisplay)
		leftDisplay = padRight(leftDisplay, halfWidth)

		// Build right side
		rightDisplay := rightPrefix + rightText
		if len(rightDisplay) > halfWidth {
			rightDisplay = rightDisplay[:halfWidth-1] + "…"
		}
		rightDisplay = style.Render(rightDisplay)
		rightDisplay = padRight(rightDisplay, halfWidth)

		// Apply cursor style
		if isCursor {
			leftDisplay = cursorStyle.Render(leftPaneStyle.Render(leftDisplay))
			rightDisplay = cursorStyle.Render(rightPaneStyle.Render(rightDisplay))
		}

		lines = append(lines, leftDisplay+separatorStyle.String()+rightDisplay)
	}

	return strings.Join(lines, "\n") + "\n"
}

func (m Model) getDiffPrefixes(diffType diff.DiffType) (left, right string) {
	switch diffType {
	case diff.DiffAdded:
		return "  ", "+ "
	case diff.DiffRemoved:
		return "- ", "  "
	case diff.DiffModified:
		return "~ ", "~ "
	default:
		return "  ", "  "
	}
}

func (m Model) renderNodeLeft(node *diff.DiffNode, maxWidth int) string {
	if node.Left == nil {
		return ""
	}
	return m.formatNode(node.Left)
}

func (m Model) renderNodeRight(node *diff.DiffNode, maxWidth int) string {
	if node.Right == nil {
		return ""
	}
	return m.formatNode(node.Right)
}

func (m Model) formatNode(yamNode *parser.YamNode) string {
	indent := strings.Repeat("  ", yamNode.Depth)
	key := yamNode.Key
	if key == "" && yamNode.Parent != nil && yamNode.Parent.Kind() == parser.KindSequence {
		key = fmt.Sprintf("[%d]", yamNode.Index)
	}

	switch yamNode.Kind() {
	case parser.KindMapping:
		if key != "" {
			return indent + key + ":"
		}
		return indent + "{...}"
	case parser.KindSequence:
		if key != "" {
			return indent + key + ":"
		}
		return indent + "[...]"
	default:
		if key != "" {
			return indent + key + ": " + yamNode.Value()
		}
		return indent + yamNode.Value()
	}
}

func (m Model) renderFooter() string {
	footerStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#8B949E")).
		Background(lipgloss.Color("#21262D")).
		Padding(0, 1).
		Width(m.width)

	// Position info
	position := fmt.Sprintf("%d/%d", m.cursor+1, len(m.diffNodes))

	// Summary
	summary := diff.RenderSummary(m.result.Summary)

	footerText := position + "  |  " + summary
	return footerStyle.Render(footerText)
}

// padRight pads a string with spaces to reach the target width
func padRight(s string, width int) string {
	// Count visible width (approximate - ANSI codes make this tricky)
	visibleLen := lipgloss.Width(s)
	if visibleLen >= width {
		return s
	}
	return s + strings.Repeat(" ", width-visibleLen)
}
