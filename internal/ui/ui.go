package ui

import (
	tea "github.com/charmbracelet/bubbletea"
	"github.com/simota/yam/internal/parser"
	"github.com/simota/yam/internal/renderer"
)

// Run starts the TUI application
func Run(root *parser.YamNode, filename string, treeStyle renderer.TreeStyle) error {
	m := NewModel(root, filename, treeStyle)
	p := tea.NewProgram(m, tea.WithAltScreen(), tea.WithMouseCellMotion())
	_, err := p.Run()
	return err
}
