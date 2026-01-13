package diff

import (
	"testing"

	"github.com/simota/yam/internal/parser"
	"gopkg.in/yaml.v3"
)

// Helper functions to build test YamNodes easily

func makeScalarNode(value string) *parser.YamNode {
	return &parser.YamNode{
		Raw: &yaml.Node{Kind: yaml.ScalarNode, Value: value},
	}
}

func makeMappingNode(children ...*parser.YamNode) *parser.YamNode {
	node := &parser.YamNode{
		Raw:      &yaml.Node{Kind: yaml.MappingNode},
		Children: children,
	}
	for _, child := range children {
		child.Parent = node
	}
	return node
}

func makeSequenceNode(children ...*parser.YamNode) *parser.YamNode {
	node := &parser.YamNode{
		Raw:      &yaml.Node{Kind: yaml.SequenceNode},
		Children: children,
	}
	for i, child := range children {
		child.Parent = node
		child.Index = i
	}
	return node
}

func makeKeyedNode(key, value string) *parser.YamNode {
	node := makeScalarNode(value)
	node.Key = key
	return node
}

// Test basic cases

func TestCompare_BothNil(t *testing.T) {
	result := Compare(nil, nil)

	if result == nil {
		t.Fatal("expected non-nil result")
	}
	if result.Root != nil {
		t.Errorf("expected Root to be nil, got %v", result.Root)
	}
	if result.Summary.Added != 0 {
		t.Errorf("expected Added=0, got %d", result.Summary.Added)
	}
	if result.Summary.Removed != 0 {
		t.Errorf("expected Removed=0, got %d", result.Summary.Removed)
	}
	if result.Summary.Modified != 0 {
		t.Errorf("expected Modified=0, got %d", result.Summary.Modified)
	}
	if result.Summary.Total != 0 {
		t.Errorf("expected Total=0, got %d", result.Summary.Total)
	}
}

func TestCompare_LeftNil(t *testing.T) {
	right := makeScalarNode("value")

	result := Compare(nil, right)

	if result == nil {
		t.Fatal("expected non-nil result")
	}
	if result.Root == nil {
		t.Fatal("expected non-nil Root")
	}
	if result.Root.Type != DiffAdded {
		t.Errorf("expected Type=DiffAdded, got %v", result.Root.Type)
	}
	if result.Root.Left != nil {
		t.Errorf("expected Left to be nil")
	}
	if result.Root.Right != right {
		t.Errorf("expected Right to be the input node")
	}
	if result.Summary.Added != 1 {
		t.Errorf("expected Added=1, got %d", result.Summary.Added)
	}
	if result.Summary.Total != 1 {
		t.Errorf("expected Total=1, got %d", result.Summary.Total)
	}
}

func TestCompare_RightNil(t *testing.T) {
	left := makeScalarNode("value")

	result := Compare(left, nil)

	if result == nil {
		t.Fatal("expected non-nil result")
	}
	if result.Root == nil {
		t.Fatal("expected non-nil Root")
	}
	if result.Root.Type != DiffRemoved {
		t.Errorf("expected Type=DiffRemoved, got %v", result.Root.Type)
	}
	if result.Root.Left != left {
		t.Errorf("expected Left to be the input node")
	}
	if result.Root.Right != nil {
		t.Errorf("expected Right to be nil")
	}
	if result.Summary.Removed != 1 {
		t.Errorf("expected Removed=1, got %d", result.Summary.Removed)
	}
	if result.Summary.Total != 1 {
		t.Errorf("expected Total=1, got %d", result.Summary.Total)
	}
}

// Test Scalar comparison

func TestCompare_ScalarUnchanged(t *testing.T) {
	left := makeScalarNode("same")
	right := makeScalarNode("same")

	result := Compare(left, right)

	if result == nil {
		t.Fatal("expected non-nil result")
	}
	if result.Root == nil {
		t.Fatal("expected non-nil Root")
	}
	if result.Root.Type != DiffUnchanged {
		t.Errorf("expected Type=DiffUnchanged, got %v", result.Root.Type)
	}
	if result.Summary.Added != 0 || result.Summary.Removed != 0 || result.Summary.Modified != 0 {
		t.Errorf("expected no changes, got Added=%d, Removed=%d, Modified=%d",
			result.Summary.Added, result.Summary.Removed, result.Summary.Modified)
	}
	if result.Summary.Total != 0 {
		t.Errorf("expected Total=0, got %d", result.Summary.Total)
	}
}

func TestCompare_ScalarModified(t *testing.T) {
	left := makeScalarNode("old")
	right := makeScalarNode("new")

	result := Compare(left, right)

	if result == nil {
		t.Fatal("expected non-nil result")
	}
	if result.Root == nil {
		t.Fatal("expected non-nil Root")
	}
	if result.Root.Type != DiffModified {
		t.Errorf("expected Type=DiffModified, got %v", result.Root.Type)
	}
	if result.Summary.Modified != 1 {
		t.Errorf("expected Modified=1, got %d", result.Summary.Modified)
	}
	if result.Summary.Total != 1 {
		t.Errorf("expected Total=1, got %d", result.Summary.Total)
	}
}

// Test Mapping comparison

func TestCompare_MappingUnchanged(t *testing.T) {
	left := makeMappingNode(
		makeKeyedNode("key1", "value1"),
		makeKeyedNode("key2", "value2"),
	)
	right := makeMappingNode(
		makeKeyedNode("key1", "value1"),
		makeKeyedNode("key2", "value2"),
	)

	result := Compare(left, right)

	if result == nil {
		t.Fatal("expected non-nil result")
	}
	if result.Root == nil {
		t.Fatal("expected non-nil Root")
	}
	if result.Root.Type != DiffUnchanged {
		t.Errorf("expected Type=DiffUnchanged, got %v", result.Root.Type)
	}
	if result.Summary.Total != 0 {
		t.Errorf("expected Total=0, got %d", result.Summary.Total)
	}
}

func TestCompare_MappingAddedKey(t *testing.T) {
	left := makeMappingNode(
		makeKeyedNode("key1", "value1"),
	)
	right := makeMappingNode(
		makeKeyedNode("key1", "value1"),
		makeKeyedNode("key2", "value2"),
	)

	result := Compare(left, right)

	if result == nil {
		t.Fatal("expected non-nil result")
	}
	if result.Root == nil {
		t.Fatal("expected non-nil Root")
	}
	if result.Root.Type != DiffModified {
		t.Errorf("expected Root Type=DiffModified, got %v", result.Root.Type)
	}
	if result.Summary.Added != 1 {
		t.Errorf("expected Added=1, got %d", result.Summary.Added)
	}
	// Note: Total includes the parent Modified count plus Added count
	if result.Summary.Total != 2 {
		t.Errorf("expected Total=2 (1 Modified parent + 1 Added), got %d", result.Summary.Total)
	}

	// Verify the added child
	var addedChild *DiffNode
	for _, child := range result.Root.Children {
		if child.Type == DiffAdded {
			addedChild = child
			break
		}
	}
	if addedChild == nil {
		t.Fatal("expected to find an added child")
	}
	if addedChild.Right.Key != "key2" {
		t.Errorf("expected added key to be 'key2', got '%s'", addedChild.Right.Key)
	}
}

func TestCompare_MappingRemovedKey(t *testing.T) {
	left := makeMappingNode(
		makeKeyedNode("key1", "value1"),
		makeKeyedNode("key2", "value2"),
	)
	right := makeMappingNode(
		makeKeyedNode("key1", "value1"),
	)

	result := Compare(left, right)

	if result == nil {
		t.Fatal("expected non-nil result")
	}
	if result.Root == nil {
		t.Fatal("expected non-nil Root")
	}
	if result.Root.Type != DiffModified {
		t.Errorf("expected Root Type=DiffModified, got %v", result.Root.Type)
	}
	if result.Summary.Removed != 1 {
		t.Errorf("expected Removed=1, got %d", result.Summary.Removed)
	}
	// Note: Total includes the parent Modified count plus Removed count
	if result.Summary.Total != 2 {
		t.Errorf("expected Total=2 (1 Modified parent + 1 Removed), got %d", result.Summary.Total)
	}

	// Verify the removed child
	var removedChild *DiffNode
	for _, child := range result.Root.Children {
		if child.Type == DiffRemoved {
			removedChild = child
			break
		}
	}
	if removedChild == nil {
		t.Fatal("expected to find a removed child")
	}
	if removedChild.Left.Key != "key2" {
		t.Errorf("expected removed key to be 'key2', got '%s'", removedChild.Left.Key)
	}
}

func TestCompare_MappingModifiedValue(t *testing.T) {
	left := makeMappingNode(
		makeKeyedNode("key1", "old"),
	)
	right := makeMappingNode(
		makeKeyedNode("key1", "new"),
	)

	result := Compare(left, right)

	if result == nil {
		t.Fatal("expected non-nil result")
	}
	if result.Root == nil {
		t.Fatal("expected non-nil Root")
	}
	if result.Root.Type != DiffModified {
		t.Errorf("expected Root Type=DiffModified, got %v", result.Root.Type)
	}
	// Note: Modified count includes both the parent mapping and the child scalar
	if result.Summary.Modified != 2 {
		t.Errorf("expected Modified=2 (1 parent + 1 child), got %d", result.Summary.Modified)
	}
	if result.Summary.Total != 2 {
		t.Errorf("expected Total=2, got %d", result.Summary.Total)
	}

	// Verify the modified child
	var modifiedChild *DiffNode
	for _, child := range result.Root.Children {
		if child.Type == DiffModified {
			modifiedChild = child
			break
		}
	}
	if modifiedChild == nil {
		t.Fatal("expected to find a modified child")
	}
	if modifiedChild.Left.Value() != "old" {
		t.Errorf("expected left value 'old', got '%s'", modifiedChild.Left.Value())
	}
	if modifiedChild.Right.Value() != "new" {
		t.Errorf("expected right value 'new', got '%s'", modifiedChild.Right.Value())
	}
}

// Test Sequence comparison

func TestCompare_SequenceUnchanged(t *testing.T) {
	left := makeSequenceNode(
		makeScalarNode("item1"),
		makeScalarNode("item2"),
	)
	right := makeSequenceNode(
		makeScalarNode("item1"),
		makeScalarNode("item2"),
	)

	result := Compare(left, right)

	if result == nil {
		t.Fatal("expected non-nil result")
	}
	if result.Root == nil {
		t.Fatal("expected non-nil Root")
	}
	if result.Root.Type != DiffUnchanged {
		t.Errorf("expected Type=DiffUnchanged, got %v", result.Root.Type)
	}
	if result.Summary.Total != 0 {
		t.Errorf("expected Total=0, got %d", result.Summary.Total)
	}
}

func TestCompare_SequenceLongerRight(t *testing.T) {
	left := makeSequenceNode(
		makeScalarNode("item1"),
	)
	right := makeSequenceNode(
		makeScalarNode("item1"),
		makeScalarNode("item2"),
	)

	result := Compare(left, right)

	if result == nil {
		t.Fatal("expected non-nil result")
	}
	if result.Root == nil {
		t.Fatal("expected non-nil Root")
	}
	if result.Root.Type != DiffModified {
		t.Errorf("expected Root Type=DiffModified, got %v", result.Root.Type)
	}
	if result.Summary.Added != 1 {
		t.Errorf("expected Added=1, got %d", result.Summary.Added)
	}
	// Note: Total includes the parent Modified count plus Added count
	if result.Summary.Total != 2 {
		t.Errorf("expected Total=2 (1 Modified parent + 1 Added), got %d", result.Summary.Total)
	}

	// Verify the added element
	if len(result.Root.Children) < 2 {
		t.Fatalf("expected at least 2 children, got %d", len(result.Root.Children))
	}
	addedChild := result.Root.Children[1]
	if addedChild.Type != DiffAdded {
		t.Errorf("expected second child Type=DiffAdded, got %v", addedChild.Type)
	}
	if addedChild.Path != "$[1]" {
		t.Errorf("expected path '$[1]', got '%s'", addedChild.Path)
	}
}

func TestCompare_SequenceLongerLeft(t *testing.T) {
	left := makeSequenceNode(
		makeScalarNode("item1"),
		makeScalarNode("item2"),
	)
	right := makeSequenceNode(
		makeScalarNode("item1"),
	)

	result := Compare(left, right)

	if result == nil {
		t.Fatal("expected non-nil result")
	}
	if result.Root == nil {
		t.Fatal("expected non-nil Root")
	}
	if result.Root.Type != DiffModified {
		t.Errorf("expected Root Type=DiffModified, got %v", result.Root.Type)
	}
	if result.Summary.Removed != 1 {
		t.Errorf("expected Removed=1, got %d", result.Summary.Removed)
	}
	// Note: Total includes the parent Modified count plus Removed count
	if result.Summary.Total != 2 {
		t.Errorf("expected Total=2 (1 Modified parent + 1 Removed), got %d", result.Summary.Total)
	}

	// Verify the removed element
	if len(result.Root.Children) < 2 {
		t.Fatalf("expected at least 2 children, got %d", len(result.Root.Children))
	}
	removedChild := result.Root.Children[1]
	if removedChild.Type != DiffRemoved {
		t.Errorf("expected second child Type=DiffRemoved, got %v", removedChild.Type)
	}
	if removedChild.Path != "$[1]" {
		t.Errorf("expected path '$[1]', got '%s'", removedChild.Path)
	}
}

func TestCompare_SequenceModifiedElement(t *testing.T) {
	left := makeSequenceNode(
		makeScalarNode("old"),
	)
	right := makeSequenceNode(
		makeScalarNode("new"),
	)

	result := Compare(left, right)

	if result == nil {
		t.Fatal("expected non-nil result")
	}
	if result.Root == nil {
		t.Fatal("expected non-nil Root")
	}
	if result.Root.Type != DiffModified {
		t.Errorf("expected Root Type=DiffModified, got %v", result.Root.Type)
	}
	// Note: Modified count includes both the parent sequence and the child element
	if result.Summary.Modified != 2 {
		t.Errorf("expected Modified=2 (1 parent + 1 child), got %d", result.Summary.Modified)
	}
	if result.Summary.Total != 2 {
		t.Errorf("expected Total=2, got %d", result.Summary.Total)
	}

	// Verify the modified element
	if len(result.Root.Children) != 1 {
		t.Fatalf("expected 1 child, got %d", len(result.Root.Children))
	}
	modifiedChild := result.Root.Children[0]
	if modifiedChild.Type != DiffModified {
		t.Errorf("expected child Type=DiffModified, got %v", modifiedChild.Type)
	}
	if modifiedChild.Left.Value() != "old" {
		t.Errorf("expected left value 'old', got '%s'", modifiedChild.Left.Value())
	}
	if modifiedChild.Right.Value() != "new" {
		t.Errorf("expected right value 'new', got '%s'", modifiedChild.Right.Value())
	}
}

// Test Summary calculation

func TestCompare_SummaryCalculation(t *testing.T) {
	// Create a complex structure with multiple types of changes:
	// - added key
	// - removed key
	// - modified value
	left := makeMappingNode(
		makeKeyedNode("unchanged", "same"),
		makeKeyedNode("modified", "old"),
		makeKeyedNode("removed", "gone"),
	)
	right := makeMappingNode(
		makeKeyedNode("unchanged", "same"),
		makeKeyedNode("modified", "new"),
		makeKeyedNode("added", "here"),
	)

	result := Compare(left, right)

	if result == nil {
		t.Fatal("expected non-nil result")
	}
	if result.Summary.Added != 1 {
		t.Errorf("expected Added=1, got %d", result.Summary.Added)
	}
	if result.Summary.Removed != 1 {
		t.Errorf("expected Removed=1, got %d", result.Summary.Removed)
	}
	// Note: Modified count includes parent mapping + child modified value
	if result.Summary.Modified != 2 {
		t.Errorf("expected Modified=2 (1 parent + 1 child), got %d", result.Summary.Modified)
	}
	// Total = 1 Added + 1 Removed + 2 Modified
	if result.Summary.Total != 4 {
		t.Errorf("expected Total=4, got %d", result.Summary.Total)
	}
}

// Test Path generation

func TestCompare_PathGeneration(t *testing.T) {
	left := makeMappingNode(
		makeKeyedNode("parent", ""),
	)
	left.Children[0].Raw = &yaml.Node{Kind: yaml.MappingNode}
	left.Children[0].Children = []*parser.YamNode{makeKeyedNode("child", "value")}

	right := makeMappingNode(
		makeKeyedNode("parent", ""),
	)
	right.Children[0].Raw = &yaml.Node{Kind: yaml.MappingNode}
	right.Children[0].Children = []*parser.YamNode{makeKeyedNode("child", "changed")}

	result := Compare(left, right)

	if result == nil {
		t.Fatal("expected non-nil result")
	}
	if result.Root == nil {
		t.Fatal("expected non-nil Root")
	}
	if result.Root.Path != "$" {
		t.Errorf("expected root path '$', got '%s'", result.Root.Path)
	}

	// Find the parent child
	var parentDiff *DiffNode
	for _, child := range result.Root.Children {
		if child.Path == "$.parent" {
			parentDiff = child
			break
		}
	}
	if parentDiff == nil {
		t.Fatal("expected to find parent diff node")
	}

	// Find the nested child
	var childDiff *DiffNode
	for _, child := range parentDiff.Children {
		if child.Path == "$.parent.child" {
			childDiff = child
			break
		}
	}
	if childDiff == nil {
		t.Fatal("expected to find child diff node")
	}
	if childDiff.Type != DiffModified {
		t.Errorf("expected child Type=DiffModified, got %v", childDiff.Type)
	}
}

// Test type mismatch

func TestCompare_TypeMismatch(t *testing.T) {
	left := makeScalarNode("scalar")
	right := makeMappingNode(makeKeyedNode("key", "value"))

	result := Compare(left, right)

	if result == nil {
		t.Fatal("expected non-nil result")
	}
	if result.Root == nil {
		t.Fatal("expected non-nil Root")
	}
	if result.Root.Type != DiffModified {
		t.Errorf("expected Type=DiffModified for type mismatch, got %v", result.Root.Type)
	}
}

// Table-driven tests for scalar comparisons

func TestCompare_ScalarValues(t *testing.T) {
	tests := []struct {
		name         string
		leftValue    string
		rightValue   string
		expectedType DiffType
	}{
		{
			name:         "empty strings equal",
			leftValue:    "",
			rightValue:   "",
			expectedType: DiffUnchanged,
		},
		{
			name:         "same strings",
			leftValue:    "hello",
			rightValue:   "hello",
			expectedType: DiffUnchanged,
		},
		{
			name:         "different strings",
			leftValue:    "hello",
			rightValue:   "world",
			expectedType: DiffModified,
		},
		{
			name:         "empty to non-empty",
			leftValue:    "",
			rightValue:   "value",
			expectedType: DiffModified,
		},
		{
			name:         "non-empty to empty",
			leftValue:    "value",
			rightValue:   "",
			expectedType: DiffModified,
		},
		{
			name:         "whitespace matters",
			leftValue:    "hello",
			rightValue:   "hello ",
			expectedType: DiffModified,
		},
		{
			name:         "case sensitive",
			leftValue:    "Hello",
			rightValue:   "hello",
			expectedType: DiffModified,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			left := makeScalarNode(tt.leftValue)
			right := makeScalarNode(tt.rightValue)

			result := Compare(left, right)

			if result.Root.Type != tt.expectedType {
				t.Errorf("expected Type=%v, got %v", tt.expectedType, result.Root.Type)
			}
		})
	}
}

// Test empty containers

func TestCompare_EmptyMapping(t *testing.T) {
	left := makeMappingNode()
	right := makeMappingNode()

	result := Compare(left, right)

	if result == nil {
		t.Fatal("expected non-nil result")
	}
	if result.Root.Type != DiffUnchanged {
		t.Errorf("expected Type=DiffUnchanged for empty mappings, got %v", result.Root.Type)
	}
	if result.Summary.Total != 0 {
		t.Errorf("expected Total=0, got %d", result.Summary.Total)
	}
}

func TestCompare_EmptySequence(t *testing.T) {
	left := makeSequenceNode()
	right := makeSequenceNode()

	result := Compare(left, right)

	if result == nil {
		t.Fatal("expected non-nil result")
	}
	if result.Root.Type != DiffUnchanged {
		t.Errorf("expected Type=DiffUnchanged for empty sequences, got %v", result.Root.Type)
	}
	if result.Summary.Total != 0 {
		t.Errorf("expected Total=0, got %d", result.Summary.Total)
	}
}
