package parser

import (
	"fmt"
	"strconv"
	"strings"
)

// ParsePath parses a path string like ".foo.bar" or ".foo[0].bar" into segments
func ParsePath(path string) ([]string, error) {
	if path == "" || path == "." {
		return nil, nil // root
	}

	// Must start with '.'
	if !strings.HasPrefix(path, ".") {
		return nil, fmt.Errorf("path must start with '.': %s", path)
	}

	path = path[1:] // remove leading '.'
	if path == "" {
		return nil, nil // root
	}

	var segments []string
	var current strings.Builder
	i := 0

	for i < len(path) {
		c := path[i]

		switch c {
		case '.':
			// End of current segment
			if current.Len() > 0 {
				segments = append(segments, current.String())
				current.Reset()
			}
			i++

		case '[':
			// Array index access
			if current.Len() > 0 {
				segments = append(segments, current.String())
				current.Reset()
			}
			// Find closing bracket
			end := strings.Index(path[i:], "]")
			if end == -1 {
				return nil, fmt.Errorf("unclosed bracket in path: %s", path)
			}
			indexStr := path[i+1 : i+end]
			// Validate it's a number
			if _, err := strconv.Atoi(indexStr); err != nil {
				return nil, fmt.Errorf("invalid array index: %s", indexStr)
			}
			segments = append(segments, indexStr)
			i += end + 1

		default:
			current.WriteByte(c)
			i++
		}
	}

	// Don't forget the last segment
	if current.Len() > 0 {
		segments = append(segments, current.String())
	}

	return segments, nil
}

// GetByPath retrieves a node by path from the root
func GetByPath(root *YamNode, path string) (*YamNode, error) {
	segments, err := ParsePath(path)
	if err != nil {
		return nil, err
	}

	if len(segments) == 0 {
		return root, nil
	}

	current := root

	// Skip document node if present
	if current.Kind() == KindDocument && len(current.Children) > 0 {
		current = current.Children[0]
	}

	for _, segment := range segments {
		found := false

		switch current.Kind() {
		case KindMapping:
			// Look for key match
			for _, child := range current.Children {
				if child.Key == segment {
					current = child
					found = true
					break
				}
			}

		case KindSequence:
			// Parse as array index
			idx, err := strconv.Atoi(segment)
			if err != nil {
				return nil, fmt.Errorf("expected array index, got: %s", segment)
			}
			if idx < 0 || idx >= len(current.Children) {
				return nil, fmt.Errorf("array index out of bounds: %d (length: %d)", idx, len(current.Children))
			}
			current = current.Children[idx]
			found = true

		default:
			return nil, fmt.Errorf("cannot traverse into scalar value at: %s", segment)
		}

		if !found {
			return nil, fmt.Errorf("path not found: %s", segment)
		}
	}

	return current, nil
}
