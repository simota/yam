package parser

import (
	"fmt"
	"io"
	"os"
	"strconv"

	"gopkg.in/yaml.v3"
)

// Parser parses YAML content into YamNode tree
type Parser struct{}

// New creates a new Parser
func New() *Parser {
	return &Parser{}
}

// ParseFile parses a YAML file and returns the root YamNode
func (p *Parser) ParseFile(path string) (*YamNode, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, fmt.Errorf("failed to open file: %w", err)
	}
	defer f.Close()

	return p.Parse(f)
}

// Parse parses YAML from a reader and returns the root YamNode
func (p *Parser) Parse(r io.Reader) (*YamNode, error) {
	var node yaml.Node
	decoder := yaml.NewDecoder(r)
	if err := decoder.Decode(&node); err != nil {
		if err == io.EOF {
			return nil, fmt.Errorf("empty YAML document")
		}
		return nil, fmt.Errorf("failed to parse YAML: %w", err)
	}

	root := p.convertNode(&node, nil, nil, 0)
	return root, nil
}

// ParseString parses a YAML string and returns the root YamNode
func (p *Parser) ParseString(content string) (*YamNode, error) {
	var node yaml.Node
	if err := yaml.Unmarshal([]byte(content), &node); err != nil {
		return nil, fmt.Errorf("failed to parse YAML: %w", err)
	}

	root := p.convertNode(&node, nil, nil, 0)
	return root, nil
}

// convertNode converts yaml.Node to YamNode recursively
func (p *Parser) convertNode(raw *yaml.Node, parent *YamNode, path []string, depth int) *YamNode {
	node := &YamNode{
		Raw:    raw,
		Parent: parent,
		Path:   path,
		Depth:  depth,
	}

	switch raw.Kind {
	case yaml.DocumentNode:
		if len(raw.Content) > 0 {
			child := p.convertNode(raw.Content[0], node, path, depth)
			node.Children = []*YamNode{child}
		}

	case yaml.MappingNode:
		for i := 0; i < len(raw.Content); i += 2 {
			keyNode := raw.Content[i]
			valueNode := raw.Content[i+1]

			key := keyNode.Value
			childPath := append(append([]string{}, path...), key)

			child := p.convertNode(valueNode, node, childPath, depth+1)
			child.Key = key
			child.Index = i / 2
			node.Children = append(node.Children, child)
		}

	case yaml.SequenceNode:
		for i, item := range raw.Content {
			childPath := append(append([]string{}, path...), strconv.Itoa(i))
			child := p.convertNode(item, node, childPath, depth+1)
			child.Index = i
			node.Children = append(node.Children, child)
		}
	}

	return node
}

// Walk traverses all nodes in depth-first order
func Walk(node *YamNode, fn func(*YamNode) bool) {
	if !fn(node) {
		return
	}
	for _, child := range node.Children {
		Walk(child, fn)
	}
}

// WalkVisible traverses only visible nodes (respecting collapse state)
func WalkVisible(node *YamNode, fn func(*YamNode) bool) {
	if !fn(node) {
		return
	}
	if node.Collapsed {
		return
	}
	for _, child := range node.Children {
		WalkVisible(child, fn)
	}
}

// Flatten returns a flat list of all nodes
func Flatten(root *YamNode) []*YamNode {
	var nodes []*YamNode
	Walk(root, func(n *YamNode) bool {
		nodes = append(nodes, n)
		return true
	})
	return nodes
}

// FlattenVisible returns a flat list of visible nodes
func FlattenVisible(root *YamNode) []*YamNode {
	var nodes []*YamNode
	WalkVisible(root, func(n *YamNode) bool {
		nodes = append(nodes, n)
		return true
	})
	return nodes
}
