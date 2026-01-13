package diff

import (
	tea "github.com/charmbracelet/bubbletea"
	"github.com/simota/yam/internal/diff"
	"github.com/simota/yam/internal/parser"
)

// Run starts the diff TUI application
func Run(result *diff.DiffResult, left, right *parser.YamNode) error {
	m := NewModel(result, left, right)
	p := tea.NewProgram(m, tea.WithAltScreen(), tea.WithMouseCellMotion())
	_, err := p.Run()
	return err
}
