package parser

import (
	"encoding/json"
	"fmt"
	"io"
	"strconv"
)

// ToJSON converts a YamNode tree to JSON bytes
func ToJSON(node *YamNode, indent bool) ([]byte, error) {
	v := nodeToInterface(node)
	if indent {
		return json.MarshalIndent(v, "", "  ")
	}
	return json.Marshal(v)
}

// nodeToInterface converts YamNode to native Go types for JSON marshaling
func nodeToInterface(node *YamNode) interface{} {
	if node == nil {
		return nil
	}

	switch node.Kind() {
	case KindDocument:
		if len(node.Children) > 0 {
			return nodeToInterface(node.Children[0])
		}
		return nil

	case KindMapping:
		m := make(map[string]interface{})
		for _, child := range node.Children {
			m[child.Key] = nodeToInterface(child)
		}
		return m

	case KindSequence:
		arr := make([]interface{}, len(node.Children))
		for i, child := range node.Children {
			arr[i] = nodeToInterface(child)
		}
		return arr

	case KindScalar:
		return scalarToInterface(node)

	case KindAlias:
		return node.Value()

	default:
		return node.Value()
	}
}

// scalarToInterface converts scalar node to appropriate Go type
func scalarToInterface(node *YamNode) interface{} {
	value := node.Value()
	scalarType := node.InferType()

	switch scalarType {
	case TypeNull:
		return nil
	case TypeBoolean:
		return value == "true" || value == "yes" || value == "on"
	case TypeNumber:
		if i, err := strconv.ParseInt(value, 10, 64); err == nil {
			return i
		}
		if f, err := strconv.ParseFloat(value, 64); err == nil {
			return f
		}
		return value
	default:
		return value
	}
}

// ParseJSON parses JSON from a reader and returns a YamNode tree
func (p *Parser) ParseJSON(r io.Reader) (*YamNode, error) {
	var data interface{}
	decoder := json.NewDecoder(r)
	decoder.UseNumber()
	if err := decoder.Decode(&data); err != nil {
		if err == io.EOF {
			return nil, fmt.Errorf("empty JSON document")
		}
		return nil, fmt.Errorf("failed to parse JSON: %w", err)
	}

	root := interfaceToNode(data, nil, nil, 0)
	// Wrap in document node for consistency
	doc := &YamNode{
		Depth:    0,
		Children: []*YamNode{root},
	}
	root.Parent = doc
	return doc, nil
}

// interfaceToNode converts native Go types to YamNode
func interfaceToNode(data interface{}, parent *YamNode, path []string, depth int) *YamNode {
	node := &YamNode{
		Parent: parent,
		Path:   path,
		Depth:  depth,
	}

	switch v := data.(type) {
	case map[string]interface{}:
		node.Raw = makeMappingRaw()
		i := 0
		for key, val := range v {
			childPath := append(append([]string{}, path...), key)
			child := interfaceToNode(val, node, childPath, depth+1)
			child.Key = key
			child.Index = i
			node.Children = append(node.Children, child)
			i++
		}

	case []interface{}:
		node.Raw = makeSequenceRaw()
		for i, item := range v {
			childPath := append(append([]string{}, path...), strconv.Itoa(i))
			child := interfaceToNode(item, node, childPath, depth+1)
			child.Index = i
			node.Children = append(node.Children, child)
		}

	case json.Number:
		node.Raw = makeScalarRaw(v.String(), inferNumberTag(v.String()))

	case string:
		node.Raw = makeScalarRaw(v, "!!str")

	case bool:
		node.Raw = makeScalarRaw(strconv.FormatBool(v), "!!bool")

	case nil:
		node.Raw = makeScalarRaw("null", "!!null")

	default:
		node.Raw = makeScalarRaw(fmt.Sprintf("%v", v), "")
	}

	return node
}

func inferNumberTag(s string) string {
	for _, c := range s {
		if c == '.' || c == 'e' || c == 'E' {
			return "!!float"
		}
	}
	return "!!int"
}
