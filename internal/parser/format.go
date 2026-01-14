package parser

import (
	"io"
	"sort"
	"strings"

	"gopkg.in/yaml.v3"
)

// FormatOptions configures YAML formatting behavior
type FormatOptions struct {
	Indent   int  // Indentation width (default: 2)
	SortKeys bool // Sort mapping keys alphabetically
}

// DefaultFormatOptions returns sensible defaults
func DefaultFormatOptions() FormatOptions {
	return FormatOptions{
		Indent:   2,
		SortKeys: false,
	}
}

// FormatTo formats a yaml.Node and writes to the given writer
func FormatTo(node *yaml.Node, w io.Writer, opts FormatOptions) error {
	// Pre-process: normalize the node
	normalizeNode(node)

	if opts.SortKeys {
		SortMappingKeys(node)
	}

	encoder := yaml.NewEncoder(w)
	encoder.SetIndent(opts.Indent)
	defer encoder.Close()

	return encoder.Encode(node)
}

// FormatString formats a yaml.Node and returns as string
func FormatString(node *yaml.Node, opts FormatOptions) (string, error) {
	var buf strings.Builder
	if err := FormatTo(node, &buf, opts); err != nil {
		return "", err
	}
	return buf.String(), nil
}

// normalizeNode recursively normalizes a yaml.Node
// - Removes trailing whitespace from values
// - Normalizes quote style where safe
func normalizeNode(node *yaml.Node) {
	if node == nil {
		return
	}

	switch node.Kind {
	case yaml.DocumentNode:
		for _, child := range node.Content {
			normalizeNode(child)
		}

	case yaml.MappingNode:
		for _, child := range node.Content {
			normalizeNode(child)
		}

	case yaml.SequenceNode:
		for _, child := range node.Content {
			normalizeNode(child)
		}

	case yaml.ScalarNode:
		// Remove trailing whitespace from value
		node.Value = strings.TrimRight(node.Value, " \t")

		// Normalize quote style: prefer unquoted when safe
		if canBeUnquoted(node.Value, node.Tag) {
			node.Style = 0 // Reset to default (unquoted)
		}

	case yaml.AliasNode:
		// Nothing to normalize
	}

	// Normalize comments (trim trailing whitespace)
	node.HeadComment = normalizeComment(node.HeadComment)
	node.LineComment = normalizeComment(node.LineComment)
	node.FootComment = normalizeComment(node.FootComment)
}

// normalizeComment trims trailing whitespace from each line
func normalizeComment(comment string) string {
	if comment == "" {
		return ""
	}
	lines := strings.Split(comment, "\n")
	for i, line := range lines {
		lines[i] = strings.TrimRight(line, " \t")
	}
	return strings.Join(lines, "\n")
}

// canBeUnquoted determines if a value can safely be unquoted
func canBeUnquoted(value, tag string) bool {
	if value == "" {
		return false // Empty string needs quotes
	}

	// If explicitly tagged as string, check for special values
	if tag == "!!str" {
		lower := strings.ToLower(value)
		switch lower {
		case "true", "false", "yes", "no", "on", "off", "null", "~":
			return false // These would be parsed as non-string
		}
	}

	// Check for characters that require quoting
	for _, r := range value {
		switch r {
		case ':', '#', '[', ']', '{', '}', ',', '&', '*', '!', '|', '>', '\'', '"', '%', '@', '`', ' ':
			return false
		case '\n', '\r', '\t':
			return false
		}
	}

	// Leading/trailing whitespace requires quotes
	if value != strings.TrimSpace(value) {
		return false
	}

	// Starts with special characters
	if len(value) > 0 {
		first := value[0]
		if first == '-' || first == '?' || first == ':' || first == ' ' {
			return false
		}
	}

	return true
}

// SortMappingKeys recursively sorts all mapping keys alphabetically
func SortMappingKeys(node *yaml.Node) {
	if node == nil {
		return
	}

	switch node.Kind {
	case yaml.DocumentNode:
		for _, child := range node.Content {
			SortMappingKeys(child)
		}

	case yaml.MappingNode:
		sortMappingContent(node)
		for _, child := range node.Content {
			SortMappingKeys(child)
		}

	case yaml.SequenceNode:
		for _, child := range node.Content {
			SortMappingKeys(child)
		}
	}
}

// sortMappingContent sorts the key-value pairs in a mapping node
func sortMappingContent(mapping *yaml.Node) {
	if len(mapping.Content) < 4 {
		return // Need at least 2 pairs to sort
	}

	// Build pairs of (key, value)
	pairs := make([]struct {
		key   *yaml.Node
		value *yaml.Node
	}, len(mapping.Content)/2)

	for i := 0; i < len(mapping.Content); i += 2 {
		pairs[i/2] = struct {
			key   *yaml.Node
			value *yaml.Node
		}{
			key:   mapping.Content[i],
			value: mapping.Content[i+1],
		}
	}

	// Sort by key value
	sort.SliceStable(pairs, func(i, j int) bool {
		return pairs[i].key.Value < pairs[j].key.Value
	})

	// Rebuild content
	for i, pair := range pairs {
		mapping.Content[i*2] = pair.key
		mapping.Content[i*2+1] = pair.value
	}
}
