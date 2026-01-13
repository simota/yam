package cmd

import (
	"fmt"
	"io"
	"os"

	"github.com/simota/yam/internal/parser"
	"github.com/simota/yam/internal/renderer"
	"github.com/simota/yam/internal/ui"
	"github.com/spf13/cobra"
)

var (
	interactive bool
	treeStyle   string
	version     = "0.1.0"
)

var rootCmd = &cobra.Command{
	Use:   "yam [file]",
	Short: "A beautiful YAML renderer for the terminal",
	Long: `yam renders YAML files with syntax highlighting and tree visualization.

Examples:
  yam config.yaml          # Render a file
  cat config.yaml | yam    # Render from stdin
  yam -i config.yaml       # Interactive TUI mode`,
	Version: version,
	Args:    cobra.MaximumNArgs(1),
	RunE:    run,
}

func Execute() error {
	return rootCmd.Execute()
}

func init() {
	rootCmd.Flags().BoolVarP(&interactive, "interactive", "i", false, "Interactive TUI mode")
	rootCmd.Flags().StringVarP(&treeStyle, "style", "s", "unicode", "Tree style: unicode, ascii, indent")
}

func run(cmd *cobra.Command, args []string) error {
	var input io.Reader
	var filename string

	if len(args) > 0 {
		filename = args[0]
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
		filename = "stdin"
	}

	// Parse YAML
	p := parser.New()
	root, err := p.Parse(input)
	if err != nil {
		return err
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
		return ui.Run(root, filename, style)
	}

	// CLI mode: render and print
	opts := renderer.DefaultOptions()
	opts.TreeStyle = style
	r := renderer.New(nil, opts)
	output := r.Render(root)
	fmt.Print(output)

	return nil
}
