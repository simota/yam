package ui

import "github.com/charmbracelet/bubbles/key"

// KeyMap defines all key bindings
type KeyMap struct {
	Up         key.Binding
	Down       key.Binding
	PageUp     key.Binding
	PageDown   key.Binding
	HalfUp     key.Binding
	HalfDown   key.Binding
	Top        key.Binding
	Bottom     key.Binding
	Toggle     key.Binding
	ExpandAll  key.Binding
	CollapseAll key.Binding
	Help       key.Binding
	Quit       key.Binding
}

// DefaultKeyMap returns the default key bindings
func DefaultKeyMap() KeyMap {
	return KeyMap{
		Up: key.NewBinding(
			key.WithKeys("up", "k"),
			key.WithHelp("↑/k", "up"),
		),
		Down: key.NewBinding(
			key.WithKeys("down", "j"),
			key.WithHelp("↓/j", "down"),
		),
		PageUp: key.NewBinding(
			key.WithKeys("pgup", "b"),
			key.WithHelp("PgUp/b", "page up"),
		),
		PageDown: key.NewBinding(
			key.WithKeys("pgdown", "f", " "),
			key.WithHelp("PgDn/f", "page down"),
		),
		HalfUp: key.NewBinding(
			key.WithKeys("ctrl+u"),
			key.WithHelp("Ctrl+U", "half page up"),
		),
		HalfDown: key.NewBinding(
			key.WithKeys("ctrl+d"),
			key.WithHelp("Ctrl+D", "half page down"),
		),
		Top: key.NewBinding(
			key.WithKeys("g", "home"),
			key.WithHelp("g", "go to top"),
		),
		Bottom: key.NewBinding(
			key.WithKeys("G", "end"),
			key.WithHelp("G", "go to bottom"),
		),
		Toggle: key.NewBinding(
			key.WithKeys("enter", "o"),
			key.WithHelp("Enter/o", "toggle fold"),
		),
		ExpandAll: key.NewBinding(
			key.WithKeys("e"),
			key.WithHelp("e", "expand all"),
		),
		CollapseAll: key.NewBinding(
			key.WithKeys("E"),
			key.WithHelp("E", "collapse all"),
		),
		Help: key.NewBinding(
			key.WithKeys("?"),
			key.WithHelp("?", "help"),
		),
		Quit: key.NewBinding(
			key.WithKeys("q", "ctrl+c", "esc"),
			key.WithHelp("q", "quit"),
		),
	}
}

// ShortHelp returns keybindings to be shown in the mini help view
func (k KeyMap) ShortHelp() []key.Binding {
	return []key.Binding{k.Up, k.Down, k.Toggle, k.Help, k.Quit}
}

// FullHelp returns keybindings for the expanded help view
func (k KeyMap) FullHelp() [][]key.Binding {
	return [][]key.Binding{
		{k.Up, k.Down, k.PageUp, k.PageDown},
		{k.HalfUp, k.HalfDown, k.Top, k.Bottom},
		{k.Toggle, k.ExpandAll, k.CollapseAll},
		{k.Help, k.Quit},
	}
}
