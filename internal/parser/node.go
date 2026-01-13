package parser

import (
	"strings"

	"gopkg.in/yaml.v3"
)

// NodeKind represents the type of YAML node
type NodeKind int

const (
	KindDocument NodeKind = iota
	KindMapping
	KindSequence
	KindScalar
	KindAlias
)

// YamNode wraps yaml.Node with additional metadata for rendering and TUI
type YamNode struct {
	Raw       *yaml.Node // Original yaml.Node
	Parent    *YamNode   // Parent node reference
	Children  []*YamNode // Child nodes (for Mapping/Sequence)
	Key       string     // Key name (for mapping entries)
	Depth     int        // Nesting depth
	Path      []string   // JSONPath-style path
	Collapsed bool       // Collapse state for TUI
	Index     int        // Index in parent (for sequences)
}

// Kind returns the NodeKind for this node
func (n *YamNode) Kind() NodeKind {
	if n.Raw == nil {
		return KindDocument
	}
	switch n.Raw.Kind {
	case yaml.DocumentNode:
		return KindDocument
	case yaml.MappingNode:
		return KindMapping
	case yaml.SequenceNode:
		return KindSequence
	case yaml.ScalarNode:
		return KindScalar
	case yaml.AliasNode:
		return KindAlias
	default:
		return KindScalar
	}
}

// Value returns the scalar value as string
func (n *YamNode) Value() string {
	if n.Raw == nil {
		return ""
	}
	return n.Raw.Value
}

// Tag returns the YAML tag (!!str, !!int, etc.)
func (n *YamNode) Tag() string {
	if n.Raw == nil {
		return ""
	}
	return n.Raw.Tag
}

// Line returns the line number in the original YAML
func (n *YamNode) Line() int {
	if n.Raw == nil {
		return 0
	}
	return n.Raw.Line
}

// Column returns the column number in the original YAML
func (n *YamNode) Column() int {
	if n.Raw == nil {
		return 0
	}
	return n.Raw.Column
}

// HeadComment returns the head comment
func (n *YamNode) HeadComment() string {
	if n.Raw == nil {
		return ""
	}
	return n.Raw.HeadComment
}

// LineComment returns the line comment
func (n *YamNode) LineComment() string {
	if n.Raw == nil {
		return ""
	}
	return n.Raw.LineComment
}

// FootComment returns the foot comment
func (n *YamNode) FootComment() string {
	if n.Raw == nil {
		return ""
	}
	return n.Raw.FootComment
}

// Anchor returns the anchor name if any
func (n *YamNode) Anchor() string {
	if n.Raw == nil {
		return ""
	}
	return n.Raw.Anchor
}

// PathString returns the JSONPath-style path as string
func (n *YamNode) PathString() string {
	if len(n.Path) == 0 {
		return "$"
	}
	return "$." + strings.Join(n.Path, ".")
}

// HasChildren returns true if the node has children
func (n *YamNode) HasChildren() bool {
	return len(n.Children) > 0
}

// IsContainer returns true if the node can contain children
func (n *YamNode) IsContainer() bool {
	kind := n.Kind()
	return kind == KindMapping || kind == KindSequence || kind == KindDocument
}

// ScalarType returns the inferred type of scalar value
type ScalarType int

const (
	TypeString ScalarType = iota
	TypeNumber
	TypeBoolean
	TypeNull
	TypeTimestamp
)

// InferType infers the type of a scalar node
func (n *YamNode) InferType() ScalarType {
	if n.Raw == nil || n.Kind() != KindScalar {
		return TypeString
	}

	tag := n.Raw.Tag
	value := n.Raw.Value

	switch tag {
	case "!!null":
		return TypeNull
	case "!!bool":
		return TypeBoolean
	case "!!int", "!!float":
		return TypeNumber
	case "!!timestamp":
		return TypeTimestamp
	case "!!str":
		return TypeString
	}

	// Auto-detect if tag is not explicit
	if tag == "" || tag == "!" {
		switch strings.ToLower(value) {
		case "null", "~", "":
			return TypeNull
		case "true", "false", "yes", "no", "on", "off":
			return TypeBoolean
		}

		if isNumber(value) {
			return TypeNumber
		}
	}

	return TypeString
}

// Helper functions to create yaml.Node for JSON parsing
func makeMappingRaw() *yaml.Node {
	return &yaml.Node{Kind: yaml.MappingNode}
}

func makeSequenceRaw() *yaml.Node {
	return &yaml.Node{Kind: yaml.SequenceNode}
}

func makeScalarRaw(value, tag string) *yaml.Node {
	return &yaml.Node{Kind: yaml.ScalarNode, Value: value, Tag: tag}
}

func isNumber(s string) bool {
	if s == "" {
		return false
	}

	start := 0
	if s[0] == '-' || s[0] == '+' {
		start = 1
		if len(s) == 1 {
			return false
		}
	}

	hasDigit := false
	hasDot := false
	hasE := false

	for i := start; i < len(s); i++ {
		c := s[i]
		switch {
		case c >= '0' && c <= '9':
			hasDigit = true
		case c == '.':
			if hasDot || hasE {
				return false
			}
			hasDot = true
		case c == 'e' || c == 'E':
			if hasE || !hasDigit {
				return false
			}
			hasE = true
			hasDigit = false
		case c == '+' || c == '-':
			if i == 0 || (s[i-1] != 'e' && s[i-1] != 'E') {
				return false
			}
		case c == '_':
			continue
		default:
			return false
		}
	}

	return hasDigit
}
