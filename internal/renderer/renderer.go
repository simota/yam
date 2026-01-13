package renderer

import (
	"fmt"
	"strings"

	"github.com/simota/yam/internal/parser"
)

// Options configures rendering behavior
type Options struct {
	ShowLineNumbers bool
	TreeStyle       TreeStyle
	IndentSize      int
	MaxWidth        int
	Interactive     bool // Show fold indicators (▼/▶) for TUI mode
	ShowTypes       bool // Show type annotations like <str>, <int>
}

// DefaultOptions returns default rendering options
func DefaultOptions() Options {
	return Options{
		ShowLineNumbers: false,
		TreeStyle:       TreeStyleUnicode,
		IndentSize:      2,
		MaxWidth:        0,
		Interactive:     false,
	}
}

// Renderer converts YamNode to styled string
type Renderer struct {
	theme   *Theme
	options Options
	chars   TreeChars
}

// New creates a new Renderer
func New(theme *Theme, opts Options) *Renderer {
	if theme == nil {
		theme = DefaultTheme()
	}
	return &Renderer{
		theme:   theme,
		options: opts,
		chars:   GetTreeChars(opts.TreeStyle),
	}
}

// Render converts a YamNode tree to a styled string
func (r *Renderer) Render(root *parser.YamNode) string {
	var buf strings.Builder
	r.renderNode(&buf, root, "", true)
	return buf.String()
}

// RenderVisible renders only visible nodes (respecting collapse state)
func (r *Renderer) RenderVisible(root *parser.YamNode) string {
	var buf strings.Builder
	r.renderNodeVisible(&buf, root, "", true)
	return buf.String()
}

func (r *Renderer) renderNode(buf *strings.Builder, node *parser.YamNode, prefix string, isLast bool) {
	if node.Kind() == parser.KindDocument {
		for i, child := range node.Children {
			r.renderNode(buf, child, prefix, i == len(node.Children)-1)
		}
		return
	}

	r.renderSingleNode(buf, node, prefix, isLast)

	if node.HasChildren() {
		newPrefix := r.getChildPrefix(prefix, isLast, node.Depth)
		for i, child := range node.Children {
			r.renderNode(buf, child, newPrefix, i == len(node.Children)-1)
		}
	}
}

func (r *Renderer) renderNodeVisible(buf *strings.Builder, node *parser.YamNode, prefix string, isLast bool) {
	if node.Kind() == parser.KindDocument {
		for i, child := range node.Children {
			r.renderNodeVisible(buf, child, prefix, i == len(node.Children)-1)
		}
		return
	}

	r.renderSingleNode(buf, node, prefix, isLast)

	if node.HasChildren() && !node.Collapsed {
		newPrefix := r.getChildPrefix(prefix, isLast, node.Depth)
		for i, child := range node.Children {
			r.renderNodeVisible(buf, child, newPrefix, i == len(node.Children)-1)
		}
	}
}

func (r *Renderer) renderSingleNode(buf *strings.Builder, node *parser.YamNode, prefix string, isLast bool) {
	// Build the tree prefix
	var line strings.Builder

	if node.Depth > 0 {
		line.WriteString(prefix)
		if isLast {
			line.WriteString(r.theme.TreeBranch.Render(r.chars.Corner + r.chars.Horizontal + " "))
		} else {
			line.WriteString(r.theme.TreeBranch.Render(r.chars.Tee + r.chars.Horizontal + " "))
		}
	}

	// Collapse indicator for containers (only in interactive/TUI mode)
	if r.options.Interactive && node.IsContainer() && node.HasChildren() {
		if node.Collapsed {
			line.WriteString(r.theme.TreeBranch.Render(r.chars.Collapsed + " "))
		} else {
			line.WriteString(r.theme.TreeBranch.Render(r.chars.Expanded + " "))
		}
	}

	// Key (for mapping entries)
	if node.Key != "" {
		line.WriteString(r.theme.Key.Render(node.Key))
		line.WriteString(r.theme.KeySeparator.Render(": "))
	}

	// Value rendering based on node type
	switch node.Kind() {
	case parser.KindMapping:
		if node.Collapsed {
			line.WriteString(r.theme.Collapsed.Render("{...}"))
		}
	case parser.KindSequence:
		if node.Collapsed {
			count := len(node.Children)
			line.WriteString(r.theme.Collapsed.Render(fmt.Sprintf("[%d items]", count)))
		} else if node.Key == "" {
			line.WriteString(r.theme.TreeBranch.Render("-"))
		}
	case parser.KindScalar:
		line.WriteString(r.renderValue(node))
	case parser.KindAlias:
		line.WriteString(r.theme.Alias.Render("*" + node.Value()))
	}

	// Anchor
	if anchor := node.Anchor(); anchor != "" {
		line.WriteString(" ")
		line.WriteString(r.theme.Anchor.Render("&" + anchor))
	}

	// Line comment
	if comment := node.LineComment(); comment != "" {
		line.WriteString(" ")
		line.WriteString(r.theme.Comment.Render(comment))
	}

	buf.WriteString(line.String())
	buf.WriteString("\n")
}

func (r *Renderer) renderValue(node *parser.YamNode) string {
	value := node.Value()
	scalarType := node.InferType()

	var rendered string
	switch scalarType {
	case parser.TypeNull:
		if value == "" || value == "~" {
			rendered = r.theme.Null.Render("null")
		} else {
			rendered = r.theme.Null.Render(value)
		}
	case parser.TypeBoolean:
		rendered = r.theme.Boolean.Render(value)
	case parser.TypeNumber:
		rendered = r.theme.Number.Render(value)
	case parser.TypeTimestamp:
		rendered = r.theme.Timestamp.Render(value)
	default:
		// Quote strings that might be confusing
		if needsQuoting(value) {
			rendered = r.theme.String.Render(fmt.Sprintf("%q", value))
		} else {
			rendered = r.theme.String.Render(value)
		}
	}

	// Add type annotation if enabled
	if r.options.ShowTypes {
		typeLabel := r.getTypeLabel(scalarType)
		rendered += " " + r.theme.TypeLabel.Render(typeLabel)
	}

	return rendered
}

func (r *Renderer) getTypeLabel(t parser.ScalarType) string {
	switch t {
	case parser.TypeString:
		return "<str>"
	case parser.TypeNumber:
		return "<int>"
	case parser.TypeBoolean:
		return "<bool>"
	case parser.TypeNull:
		return "<null>"
	case parser.TypeTimestamp:
		return "<time>"
	default:
		return ""
	}
}

func (r *Renderer) getChildPrefix(prefix string, isLast bool, depth int) string {
	if depth == 0 {
		return ""
	}
	if isLast {
		return prefix + "    "
	}
	return prefix + r.theme.TreeBranch.Render(r.chars.Vertical) + "   "
}

func needsQuoting(s string) bool {
	if s == "" {
		return true
	}
	// Check for special characters or values that might be confusing
	switch strings.ToLower(s) {
	case "true", "false", "yes", "no", "on", "off", "null", "~":
		return true
	}
	// Check for leading/trailing whitespace
	if s != strings.TrimSpace(s) {
		return true
	}
	// Check for special characters
	for _, c := range s {
		if c == ':' || c == '#' || c == '\n' || c == '\t' {
			return true
		}
	}
	return false
}
