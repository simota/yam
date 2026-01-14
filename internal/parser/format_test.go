package parser

import (
	"strings"
	"testing"

	"gopkg.in/yaml.v3"
)

func parseYAML(t *testing.T, content string) *yaml.Node {
	t.Helper()
	var node yaml.Node
	if err := yaml.Unmarshal([]byte(content), &node); err != nil {
		t.Fatalf("failed to parse YAML: %v", err)
	}
	return &node
}

func TestFormatTo_BasicIndentation(t *testing.T) {
	input := `app:
  name: test
  nested:
    key: value`

	node := parseYAML(t, input)
	opts := FormatOptions{Indent: 2}

	result, err := FormatString(node, opts)
	if err != nil {
		t.Fatalf("FormatString failed: %v", err)
	}

	// Verify indentation
	if !strings.Contains(result, "  name: test") {
		t.Errorf("expected 2-space indentation, got:\n%s", result)
	}
}

func TestFormatTo_CustomIndentation(t *testing.T) {
	input := `app:
  name: test`

	node := parseYAML(t, input)
	opts := FormatOptions{Indent: 4}

	result, err := FormatString(node, opts)
	if err != nil {
		t.Fatalf("FormatString failed: %v", err)
	}

	if !strings.Contains(result, "    name: test") {
		t.Errorf("expected 4-space indentation, got:\n%s", result)
	}
}

func TestFormatTo_PreservesComments(t *testing.T) {
	input := `# Top comment
app:
  name: test # inline comment`

	node := parseYAML(t, input)
	opts := DefaultFormatOptions()

	result, err := FormatString(node, opts)
	if err != nil {
		t.Fatalf("FormatString failed: %v", err)
	}

	if !strings.Contains(result, "# Top comment") {
		t.Errorf("expected top comment to be preserved, got:\n%s", result)
	}
	if !strings.Contains(result, "# inline comment") {
		t.Errorf("expected inline comment to be preserved, got:\n%s", result)
	}
}

func TestSortMappingKeys(t *testing.T) {
	input := `zebra: 1
apple: 2
mango: 3`

	node := parseYAML(t, input)
	opts := FormatOptions{Indent: 2, SortKeys: true}

	result, err := FormatString(node, opts)
	if err != nil {
		t.Fatalf("FormatString failed: %v", err)
	}

	// Check that apple comes before mango and mango before zebra
	applePos := strings.Index(result, "apple")
	mangoPos := strings.Index(result, "mango")
	zebraPos := strings.Index(result, "zebra")

	if applePos >= mangoPos || mangoPos >= zebraPos {
		t.Errorf("expected keys to be sorted alphabetically, got:\n%s", result)
	}
}

func TestSortMappingKeys_Nested(t *testing.T) {
	input := `z:
  c: 1
  a: 2
a:
  z: 1
  a: 2`

	node := parseYAML(t, input)
	opts := FormatOptions{Indent: 2, SortKeys: true}

	result, err := FormatString(node, opts)
	if err != nil {
		t.Fatalf("FormatString failed: %v", err)
	}

	lines := strings.Split(result, "\n")
	// First key should be 'a:', not 'z:'
	if len(lines) > 0 && !strings.HasPrefix(lines[0], "a:") {
		t.Errorf("expected first key to be 'a:', got:\n%s", result)
	}
}

func TestCanBeUnquoted(t *testing.T) {
	tests := []struct {
		value    string
		tag      string
		expected bool
	}{
		{"simple", "", true},
		{"with space", "", false},
		{"with:colon", "", false},
		{"with#hash", "", false},
		{"", "", false},                   // empty string
		{"true", "!!str", false},          // boolean-like
		{"false", "!!str", false},         // boolean-like
		{"null", "!!str", false},          // null-like
		{"yes", "!!str", false},           // boolean-like
		{"no", "!!str", false},            // boolean-like
		{"hello world", "", false},        // contains space
		{"-dash", "", false},              // starts with dash
		{"normal_value", "", true},        // normal identifier
		{"123", "", true},                 // numbers are ok
		{"value\nwith\nnewlines", "", false},
	}

	for _, tt := range tests {
		result := canBeUnquoted(tt.value, tt.tag)
		if result != tt.expected {
			t.Errorf("canBeUnquoted(%q, %q) = %v, expected %v", tt.value, tt.tag, result, tt.expected)
		}
	}
}

func TestNormalizeComment(t *testing.T) {
	tests := []struct {
		input    string
		expected string
	}{
		{"", ""},
		{"no trailing", "no trailing"},
		{"with trailing   ", "with trailing"},
		{"line1  \nline2  ", "line1\nline2"},
	}

	for _, tt := range tests {
		result := normalizeComment(tt.input)
		if result != tt.expected {
			t.Errorf("normalizeComment(%q) = %q, expected %q", tt.input, result, tt.expected)
		}
	}
}

func TestFormatTo_Sequence(t *testing.T) {
	input := `items:
  - one
  - two
  - three`

	node := parseYAML(t, input)
	opts := DefaultFormatOptions()

	result, err := FormatString(node, opts)
	if err != nil {
		t.Fatalf("FormatString failed: %v", err)
	}

	if !strings.Contains(result, "- one") {
		t.Errorf("expected sequence items, got:\n%s", result)
	}
}

func TestDefaultFormatOptions(t *testing.T) {
	opts := DefaultFormatOptions()
	if opts.Indent != 2 {
		t.Errorf("expected default indent 2, got %d", opts.Indent)
	}
	if opts.SortKeys {
		t.Errorf("expected default SortKeys false")
	}
}
