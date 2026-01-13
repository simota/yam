package diff

import "github.com/simota/yam/internal/parser"

// DiffType represents the type of difference between two nodes
type DiffType int

const (
	DiffUnchanged DiffType = iota
	DiffAdded
	DiffRemoved
	DiffModified
)

// DiffNode represents a node in the diff tree structure
type DiffNode struct {
	Left     *parser.YamNode // Node from file1 (nil if Added)
	Right    *parser.YamNode // Node from file2 (nil if Removed)
	Type     DiffType
	Children []*DiffNode
	Path     string // JSONPath-like path
}
