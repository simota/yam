package diff

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/lipgloss"
	"github.com/simota/yam/internal/parser"
)

// Style definitions for diff output
var (
	addedStyle     = lipgloss.NewStyle().Foreground(lipgloss.Color("#A6E3A1")) // Green
	removedStyle   = lipgloss.NewStyle().Foreground(lipgloss.Color("#F38BA8")) // Red
	modifiedStyle  = lipgloss.NewStyle().Foreground(lipgloss.Color("#F9E2AF")) // Yellow
	unchangedStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("#6C7086")) // Gray
	keyStyle       = lipgloss.NewStyle().Foreground(lipgloss.Color("#89B4FA")) // Blue
)

// Render converts a DiffResult to a colored string for CLI output
func Render(result *DiffResult) string {
	if result == nil {
		return ""
	}

	var buf strings.Builder

	// Render header with file names if present
	if result.LeftFile != "" || result.RightFile != "" {
		buf.WriteString(fmt.Sprintf("--- %s\n", result.LeftFile))
		buf.WriteString(fmt.Sprintf("+++ %s\n", result.RightFile))
		buf.WriteString("\n")
	}

	// Render the diff tree
	if result.Root != nil {
		renderDiffNode(&buf, result.Root, "")
	}

	// Append summary at the end
	if result.Summary.Total > 0 {
		buf.WriteString("\n")
		buf.WriteString(RenderSummary(result.Summary))
		buf.WriteString("\n")
	}

	return buf.String()
}

// hasChanges checks if a DiffNode or any of its children have changes
func hasChanges(node *DiffNode) bool {
	if node == nil {
		return false
	}
	if node.Type != DiffUnchanged {
		return true
	}
	for _, child := range node.Children {
		if hasChanges(child) {
			return true
		}
	}
	return false
}

// renderDiffNode recursively renders a DiffNode and its children
func renderDiffNode(buf *strings.Builder, node *DiffNode, indent string) {
	if node == nil {
		return
	}

	// Skip unchanged nodes that have no changed children
	if node.Type == DiffUnchanged && !hasChanges(node) {
		return
	}

	// Get the prefix and style based on diff type
	prefix, style := getDiffPrefixAndStyle(node.Type)

	// Get key from either Left or Right node
	key := getNodeKey(node)

	// Skip rendering the root document node itself, just render children
	if isDocumentNode(node) {
		for _, child := range node.Children {
			renderDiffNode(buf, child, indent)
		}
		return
	}

	// Skip rendering if key is empty (root-level container without key)
	if key == "" && isContainerNode(node) {
		// Just render children without a header line
		for _, child := range node.Children {
			renderDiffNode(buf, child, indent)
		}
		return
	}

	// Render based on node type
	if node.Type == DiffModified && isScalarNode(node) {
		// Modified scalar: show "oldValue → newValue"
		oldValue := getScalarValue(node.Left)
		newValue := getScalarValue(node.Right)
		line := fmt.Sprintf("%s%s%s: %s → %s", prefix, indent, keyStyle.Render(key), oldValue, newValue)
		buf.WriteString(style.Render(line))
		buf.WriteString("\n")
	} else if isContainerNode(node) {
		// Container node (mapping or sequence)
		line := fmt.Sprintf("%s%s%s:", prefix, indent, keyStyle.Render(key))
		buf.WriteString(style.Render(line))
		buf.WriteString("\n")

		// Render children with increased indent
		childIndent := indent + "  "
		for _, child := range node.Children {
			renderDiffNode(buf, child, childIndent)
		}
	} else {
		// Scalar node
		value := getNodeValue(node)
		line := fmt.Sprintf("%s%s%s: %s", prefix, indent, keyStyle.Render(key), value)
		buf.WriteString(style.Render(line))
		buf.WriteString("\n")
	}
}

// RenderSummary returns a summary string like "Summary: 3 added, 0 removed, 2 modified"
func RenderSummary(summary DiffSummary) string {
	if summary.Total == 0 {
		return "Summary: no changes"
	}

	// Always show all three categories for clarity
	parts := []string{
		addedStyle.Render(fmt.Sprintf("%d added", summary.Added)),
		removedStyle.Render(fmt.Sprintf("%d removed", summary.Removed)),
		modifiedStyle.Render(fmt.Sprintf("%d modified", summary.Modified)),
	}

	return "Summary: " + strings.Join(parts, ", ")
}

// getDiffPrefixAndStyle returns the prefix string and lipgloss style for a diff type
func getDiffPrefixAndStyle(diffType DiffType) (string, lipgloss.Style) {
	switch diffType {
	case DiffAdded:
		return "+ ", addedStyle
	case DiffRemoved:
		return "- ", removedStyle
	case DiffModified:
		return "~ ", modifiedStyle
	default:
		return "  ", unchangedStyle
	}
}

// getNodeKey extracts the key from a DiffNode
func getNodeKey(node *DiffNode) string {
	if node.Left != nil && node.Left.Key != "" {
		return node.Left.Key
	}
	if node.Right != nil && node.Right.Key != "" {
		return node.Right.Key
	}
	// For sequence items, use index - but only if parent is a sequence
	if node.Left != nil && node.Left.Parent != nil && node.Left.Parent.Kind() == parser.KindSequence {
		return fmt.Sprintf("[%d]", node.Left.Index)
	}
	if node.Right != nil && node.Right.Parent != nil && node.Right.Parent.Kind() == parser.KindSequence {
		return fmt.Sprintf("[%d]", node.Right.Index)
	}
	return ""
}

// getNodeValue returns the value from Left or Right node (whichever exists)
func getNodeValue(node *DiffNode) string {
	var yamNode *parser.YamNode
	if node.Right != nil {
		yamNode = node.Right
	} else if node.Left != nil {
		yamNode = node.Left
	}

	if yamNode == nil {
		return ""
	}

	return formatNodeValue(yamNode)
}

// getScalarValue returns the scalar value of a YamNode
func getScalarValue(yamNode *parser.YamNode) string {
	if yamNode == nil {
		return ""
	}
	return yamNode.Value()
}

// formatNodeValue formats a YamNode's value for display
func formatNodeValue(yamNode *parser.YamNode) string {
	if yamNode == nil {
		return ""
	}

	switch yamNode.Kind() {
	case parser.KindMapping:
		return "{...}"
	case parser.KindSequence:
		count := len(yamNode.Children)
		return fmt.Sprintf("[%d items]", count)
	default:
		return yamNode.Value()
	}
}

// isDocumentNode checks if a DiffNode represents a document root
func isDocumentNode(node *DiffNode) bool {
	if node.Left != nil && node.Left.Kind() == parser.KindDocument {
		return true
	}
	if node.Right != nil && node.Right.Kind() == parser.KindDocument {
		return true
	}
	return false
}

// isContainerNode checks if a DiffNode represents a container (mapping or sequence)
func isContainerNode(node *DiffNode) bool {
	var yamNode *parser.YamNode
	if node.Right != nil {
		yamNode = node.Right
	} else if node.Left != nil {
		yamNode = node.Left
	}

	if yamNode == nil {
		return len(node.Children) > 0
	}

	kind := yamNode.Kind()
	return kind == parser.KindMapping || kind == parser.KindSequence
}

// isScalarNode checks if a DiffNode represents a scalar value
func isScalarNode(node *DiffNode) bool {
	// For modified nodes, both should be scalars
	if node.Left != nil && node.Left.Kind() != parser.KindScalar {
		return false
	}
	if node.Right != nil && node.Right.Kind() != parser.KindScalar {
		return false
	}
	return node.Left != nil || node.Right != nil
}
