package diff

import "github.com/charmbracelet/bubbles/key"

// KeyMap defines key bindings for diff TUI
type KeyMap struct {
	Up       key.Binding
	Down     key.Binding
	PageUp   key.Binding
	PageDown key.Binding
	Top      key.Binding
	Bottom   key.Binding
	NextDiff key.Binding
	PrevDiff key.Binding
	Help     key.Binding
	Quit     key.Binding
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
			key.WithKeys("pgup", "b", "ctrl+u"),
			key.WithHelp("PgUp/b", "page up"),
		),
		PageDown: key.NewBinding(
			key.WithKeys("pgdown", "f", "ctrl+d", " "),
			key.WithHelp("PgDn/f", "page down"),
		),
		Top: key.NewBinding(
			key.WithKeys("g", "home"),
			key.WithHelp("g", "go to top"),
		),
		Bottom: key.NewBinding(
			key.WithKeys("G", "end"),
			key.WithHelp("G", "go to bottom"),
		),
		NextDiff: key.NewBinding(
			key.WithKeys("n", "]"),
			key.WithHelp("n/]", "next diff"),
		),
		PrevDiff: key.NewBinding(
			key.WithKeys("N", "["),
			key.WithHelp("N/[", "prev diff"),
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
	return []key.Binding{k.Up, k.Down, k.NextDiff, k.PrevDiff, k.Help, k.Quit}
}

// FullHelp returns keybindings for the expanded help view
func (k KeyMap) FullHelp() [][]key.Binding {
	return [][]key.Binding{
		{k.Up, k.Down, k.PageUp, k.PageDown},
		{k.Top, k.Bottom},
		{k.NextDiff, k.PrevDiff},
		{k.Help, k.Quit},
	}
}
