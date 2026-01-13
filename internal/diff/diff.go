package diff

import (
	"fmt"

	"github.com/simota/yam/internal/parser"
)

// Compare compares two YamNode trees and returns a DiffResult.
// It handles nil inputs gracefully and produces a structured diff tree.
func Compare(left, right *parser.YamNode) *DiffResult {
	// Handle nil inputs
	if left == nil && right == nil {
		return &DiffResult{
			Root:    nil,
			Summary: DiffSummary{},
		}
	}

	// Create root DiffNode by comparing the nodes
	root := compareNodes(left, right, "$")

	// Calculate summary by walking the diff tree
	summary := calculateSummary(root)

	return &DiffResult{
		Root:      root,
		Summary:   summary,
		LeftFile:  "",
		RightFile: "",
	}
}

// compareNodes recursively compares two YamNodes and returns a DiffNode.
// The path parameter represents the JSONPath-like path to the current node.
func compareNodes(left, right *parser.YamNode, path string) *DiffNode {
	// Handle nil cases
	if left == nil && right == nil {
		return nil
	}

	if left == nil {
		// Node was added (exists only in right)
		return &DiffNode{
			Left:  nil,
			Right: right,
			Type:  DiffAdded,
			Path:  path,
		}
	}

	if right == nil {
		// Node was removed (exists only in left)
		return &DiffNode{
			Left:  left,
			Right: nil,
			Type:  DiffRemoved,
			Path:  path,
		}
	}

	// Both nodes exist - compare based on kind
	if left.Kind() == parser.KindMapping && right.Kind() == parser.KindMapping {
		// Build maps of children by key for efficient lookup
		leftByKey := make(map[string]*parser.YamNode)
		for _, child := range left.Children {
			leftByKey[child.Key] = child
		}

		rightByKey := make(map[string]*parser.YamNode)
		for _, child := range right.Children {
			rightByKey[child.Key] = child
		}

		// Collect all unique keys from both maps
		allKeys := make(map[string]bool)
		for k := range leftByKey {
			allKeys[k] = true
		}
		for k := range rightByKey {
			allKeys[k] = true
		}

		// Compare each key
		var children []*DiffNode
		hasChanges := false
		for key := range allKeys {
			leftChild := leftByKey[key]
			rightChild := rightByKey[key]
			childPath := path + "." + key
			childDiff := compareNodes(leftChild, rightChild, childPath)
			if childDiff != nil {
				children = append(children, childDiff)
				if childDiff.Type != DiffUnchanged {
					hasChanges = true
				}
			}
		}

		// Determine parent DiffType based on children
		diffType := DiffUnchanged
		if hasChanges {
			diffType = DiffModified
		}

		return &DiffNode{
			Left:     left,
			Right:    right,
			Type:     diffType,
			Children: children,
			Path:     path,
		}
	}

	// Sequence comparison
	if left.Kind() == parser.KindSequence && right.Kind() == parser.KindSequence {
		maxLen := len(left.Children)
		if len(right.Children) > maxLen {
			maxLen = len(right.Children)
		}

		var children []*DiffNode
		hasChanges := false

		for i := 0; i < maxLen; i++ {
			var leftChild, rightChild *parser.YamNode
			if i < len(left.Children) {
				leftChild = left.Children[i]
			}
			if i < len(right.Children) {
				rightChild = right.Children[i]
			}

			childPath := fmt.Sprintf("%s[%d]", path, i)
			childDiff := compareNodes(leftChild, rightChild, childPath)
			if childDiff != nil {
				children = append(children, childDiff)
				if childDiff.Type != DiffUnchanged {
					hasChanges = true
				}
			}
		}

		diffType := DiffUnchanged
		if hasChanges {
			diffType = DiffModified
		}

		return &DiffNode{
			Left:     left,
			Right:    right,
			Type:     diffType,
			Children: children,
			Path:     path,
		}
	}

	// Handle Scalar nodes
	if left.Kind() == parser.KindScalar && right.Kind() == parser.KindScalar {
		diffType := DiffUnchanged
		if left.Value() != right.Value() {
			diffType = DiffModified
		}
		return &DiffNode{
			Left:  left,
			Right: right,
			Type:  diffType,
			Path:  path,
		}
	}

	// Handle type mismatch (different node kinds)
	// e.g., left is Mapping but right is Scalar
	if left.Kind() != right.Kind() {
		return &DiffNode{
			Left:  left,
			Right: right,
			Type:  DiffModified,
			Path:  path,
		}
	}

	// Handle Document nodes (compare their children)
	if left.Kind() == parser.KindDocument && right.Kind() == parser.KindDocument {
		// Compare first child of each document
		var leftChild, rightChild *parser.YamNode
		if len(left.Children) > 0 {
			leftChild = left.Children[0]
		}
		if len(right.Children) > 0 {
			rightChild = right.Children[0]
		}
		return compareNodes(leftChild, rightChild, path)
	}

	// Fallback for any other cases
	return &DiffNode{
		Left:  left,
		Right: right,
		Type:  DiffUnchanged,
		Path:  path,
	}
}

// calculateSummary walks the DiffNode tree and counts differences.
func calculateSummary(root *DiffNode) DiffSummary {
	if root == nil {
		return DiffSummary{}
	}

	var summary DiffSummary
	walkDiffTree(root, &summary)

	summary.Total = summary.Added + summary.Removed + summary.Modified
	return summary
}

// walkDiffTree recursively traverses the DiffNode tree and accumulates counts.
func walkDiffTree(node *DiffNode, summary *DiffSummary) {
	if node == nil {
		return
	}

	switch node.Type {
	case DiffAdded:
		summary.Added++
	case DiffRemoved:
		summary.Removed++
	case DiffModified:
		summary.Modified++
	}

	// Recursively process children
	for _, child := range node.Children {
		walkDiffTree(child, summary)
	}
}
