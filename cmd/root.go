package cmd

import (
	"fmt"
	"io"
	"os"
	"strings"

	"github.com/simota/yam/internal/parser"
	"github.com/simota/yam/internal/renderer"
	"github.com/simota/yam/internal/ui"
	"github.com/spf13/cobra"
)

var (
	interactive bool
	treeStyle   string
	showTypes   bool
	version     = "0.1.0"
)

var rootCmd = &cobra.Command{
	Use:   "yam [path] [file]",
	Short: "A beautiful YAML renderer for the terminal",
	Long: `yam renders YAML files with syntax highlighting and tree visualization.

Examples:
  yam config.yaml              # Render a file
  cat config.yaml | yam        # Render from stdin
  yam -i config.yaml           # Interactive TUI mode
  yam '.data.host' config.yaml # Extract value at path
  yam '.items[0]' config.yaml  # Extract array element`,
	Version: version,
	Args:    cobra.MaximumNArgs(2),
	RunE:    run,
}

func Execute() error {
	return rootCmd.Execute()
}

func init() {
	rootCmd.Flags().BoolVarP(&interactive, "interactive", "i", false, "Interactive TUI mode")
	rootCmd.Flags().StringVarP(&treeStyle, "style", "s", "unicode", "Tree style: unicode, ascii, indent")
	rootCmd.Flags().BoolVarP(&showTypes, "types", "t", false, "Show type annotations")
}

func run(cmd *cobra.Command, args []string) error {
	var input io.Reader
	var filename string
	var pathQuery string

	// Parse arguments: [path] [file] or [file] or nothing
	// Path starts with '.'
	switch len(args) {
	case 2:
		// yam '.path' file.yaml
		pathQuery = args[0]
		filename = args[1]
	case 1:
		// Could be path or file
		if strings.HasPrefix(args[0], ".") {
			// yam '.path' (with stdin)
			pathQuery = args[0]
		} else {
			// yam file.yaml
			filename = args[0]
		}
	}

	// Set up input source
	if filename != "" {
		f, err := os.Open(filename)
		if err != nil {
			return fmt.Errorf("failed to open file: %w", err)
		}
		defer f.Close()
		input = f
	} else {
		// Check if stdin has data
		stat, _ := os.Stdin.Stat()
		if (stat.Mode() & os.ModeCharDevice) != 0 {
			return fmt.Errorf("no input: provide a file or pipe YAML content")
		}
		input = os.Stdin
		if filename == "" {
			filename = "stdin"
		}
	}

	// Parse YAML
	p := parser.New()
	root, err := p.Parse(input)
	if err != nil {
		return err
	}

	// Apply path query if specified
	if pathQuery != "" {
		root, err = parser.GetByPath(root, pathQuery)
		if err != nil {
			return fmt.Errorf("path query failed: %w", err)
		}
	}

	// Determine tree style
	style := renderer.TreeStyleUnicode
	switch treeStyle {
	case "ascii":
		style = renderer.TreeStyleASCII
	case "indent":
		style = renderer.TreeStyleIndent
	}

	if interactive {
		// Run TUI
		return ui.Run(root, filename, style, showTypes)
	}

	// CLI mode: render and print
	opts := renderer.DefaultOptions()
	opts.TreeStyle = style
	opts.ShowTypes = showTypes
	r := renderer.New(nil, opts)
	output := r.Render(root)
	fmt.Print(output)

	return nil
}
