package renderer

import "github.com/charmbracelet/lipgloss"

// Theme defines colors for YAML elements
type Theme struct {
	// Keys
	Key          lipgloss.Style
	KeySeparator lipgloss.Style

	// Values by type
	String    lipgloss.Style
	Number    lipgloss.Style
	Boolean   lipgloss.Style
	Null      lipgloss.Style
	Timestamp lipgloss.Style

	// Structure
	Anchor lipgloss.Style
	Alias  lipgloss.Style
	Tag    lipgloss.Style

	// Meta
	Comment    lipgloss.Style
	LineNumber lipgloss.Style

	// Tree
	TreeBranch lipgloss.Style
	Collapsed  lipgloss.Style
}

// DefaultTheme returns the default color theme
func DefaultTheme() *Theme {
	return &Theme{
		Key: lipgloss.NewStyle().
			Foreground(lipgloss.AdaptiveColor{Light: "#0550AE", Dark: "#79C0FF"}).
			Bold(true),
		KeySeparator: lipgloss.NewStyle().
			Foreground(lipgloss.AdaptiveColor{Light: "#24292F", Dark: "#C9D1D9"}),
		String: lipgloss.NewStyle().
			Foreground(lipgloss.AdaptiveColor{Light: "#0A3069", Dark: "#A5D6FF"}),
		Number: lipgloss.NewStyle().
			Foreground(lipgloss.AdaptiveColor{Light: "#0550AE", Dark: "#79C0FF"}),
		Boolean: lipgloss.NewStyle().
			Foreground(lipgloss.AdaptiveColor{Light: "#CF222E", Dark: "#FF7B72"}),
		Null: lipgloss.NewStyle().
			Foreground(lipgloss.AdaptiveColor{Light: "#6E7781", Dark: "#8B949E"}).
			Italic(true),
		Timestamp: lipgloss.NewStyle().
			Foreground(lipgloss.AdaptiveColor{Light: "#8250DF", Dark: "#D2A8FF"}),
		Anchor: lipgloss.NewStyle().
			Foreground(lipgloss.AdaptiveColor{Light: "#953800", Dark: "#FFA657"}),
		Alias: lipgloss.NewStyle().
			Foreground(lipgloss.AdaptiveColor{Light: "#953800", Dark: "#FFA657"}).
			Italic(true),
		Tag: lipgloss.NewStyle().
			Foreground(lipgloss.AdaptiveColor{Light: "#6E7781", Dark: "#8B949E"}),
		Comment: lipgloss.NewStyle().
			Foreground(lipgloss.AdaptiveColor{Light: "#6E7781", Dark: "#8B949E"}).
			Italic(true),
		LineNumber: lipgloss.NewStyle().
			Foreground(lipgloss.AdaptiveColor{Light: "#6E7781", Dark: "#6E7681"}).
			Width(4).
			Align(lipgloss.Right),
		TreeBranch: lipgloss.NewStyle().
			Foreground(lipgloss.AdaptiveColor{Light: "#6E7781", Dark: "#484F58"}),
		Collapsed: lipgloss.NewStyle().
			Foreground(lipgloss.AdaptiveColor{Light: "#6E7781", Dark: "#8B949E"}),
	}
}

// TreeStyle defines tree drawing characters
type TreeStyle int

const (
	TreeStyleUnicode TreeStyle = iota
	TreeStyleASCII
	TreeStyleIndent
)

// TreeChars holds the characters for tree drawing
type TreeChars struct {
	Vertical   string // │
	Horizontal string // ─
	Corner     string // └
	Tee        string // ├
	Collapsed  string // ▶
	Expanded   string // ▼
}

// GetTreeChars returns the tree characters for a style
func GetTreeChars(style TreeStyle) TreeChars {
	switch style {
	case TreeStyleASCII:
		return TreeChars{
			Vertical:   "|",
			Horizontal: "-",
			Corner:     "`",
			Tee:        "+",
			Collapsed:  "+",
			Expanded:   "-",
		}
	case TreeStyleIndent:
		return TreeChars{
			Vertical:   " ",
			Horizontal: " ",
			Corner:     " ",
			Tee:        " ",
			Collapsed:  "+",
			Expanded:   "-",
		}
	default: // TreeStyleUnicode
		return TreeChars{
			Vertical:   "│",
			Horizontal: "─",
			Corner:     "└",
			Tee:        "├",
			Collapsed:  "▶",
			Expanded:   "▼",
		}
	}
}
