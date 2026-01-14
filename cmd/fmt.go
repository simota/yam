package cmd

import (
	"fmt"
	"io"
	"os"
	"path/filepath"

	"github.com/simota/yam/internal/parser"
	"github.com/spf13/cobra"
)

var (
	fmtWriteInPlace bool
	fmtIndent       int
	fmtSortKeys     bool
)

var fmtCmd = &cobra.Command{
	Use:   "fmt [file]",
	Short: "Format YAML files",
	Long: `Format YAML files with consistent styling.

Reads YAML from a file or stdin and outputs formatted YAML.
By default, output goes to stdout. Use -w to overwrite the input file.

Formatting includes:
  - Consistent indentation (default: 2 spaces)
  - Trailing whitespace removal
  - Normalized quoting (unquoted when safe)
  - Final newline ensured
  - Optionally: alphabetically sorted keys (--sort-keys)

Exit codes:
  0  Success
  1  Error occurred

Examples:
  yam fmt config.yaml              # Format and print to stdout
  yam fmt -w config.yaml           # Format in-place
  cat config.yaml | yam fmt        # Format from stdin
  yam fmt --indent 4 config.yaml   # Use 4-space indentation
  yam fmt --sort-keys config.yaml  # Sort keys alphabetically`,
	Args: cobra.MaximumNArgs(1),
	RunE: runFmt,
}

func init() {
	rootCmd.AddCommand(fmtCmd)
	fmtCmd.Flags().BoolVarP(&fmtWriteInPlace, "write", "w", false, "Write result to source file instead of stdout")
	fmtCmd.Flags().IntVarP(&fmtIndent, "indent", "i", 2, "Indentation width in spaces")
	fmtCmd.Flags().BoolVarP(&fmtSortKeys, "sort-keys", "s", false, "Sort keys alphabetically")
}

func runFmt(cmd *cobra.Command, args []string) error {
	var input io.Reader
	var filename string
	var isStdin bool

	// Determine input source
	if len(args) == 1 {
		filename = args[0]
		f, err := os.Open(filename)
		if err != nil {
			return fmt.Errorf("failed to open file: %w", err)
		}
		defer f.Close()
		input = f
	} else {
		// stdin
		stat, _ := os.Stdin.Stat()
		if (stat.Mode() & os.ModeCharDevice) != 0 {
			return fmt.Errorf("no input: provide a file or pipe YAML content")
		}
		input = os.Stdin
		isStdin = true
	}

	// -w flag requires a file (not stdin)
	if fmtWriteInPlace && isStdin {
		return fmt.Errorf("cannot use -w with stdin input")
	}

	// Parse YAML
	p := parser.New()
	yamNode, err := p.Parse(input)
	if err != nil {
		return err
	}

	// Format options
	opts := parser.FormatOptions{
		Indent:   fmtIndent,
		SortKeys: fmtSortKeys,
	}

	// Get the raw yaml.Node for formatting
	rawNode := yamNode.Raw

	// Determine output destination
	if fmtWriteInPlace {
		// Write to temp file then rename (atomic)
		dir := filepath.Dir(filename)
		tmpFile, err := os.CreateTemp(dir, ".yam-fmt-*.yaml")
		if err != nil {
			return fmt.Errorf("failed to create temp file: %w", err)
		}
		tmpPath := tmpFile.Name()
		defer os.Remove(tmpPath) // cleanup on error

		if err := parser.FormatTo(rawNode, tmpFile, opts); err != nil {
			tmpFile.Close()
			return fmt.Errorf("failed to format: %w", err)
		}
		tmpFile.Close()

		// Rename temp file to original
		if err := os.Rename(tmpPath, filename); err != nil {
			return fmt.Errorf("failed to write file: %w", err)
		}
	} else {
		if err := parser.FormatTo(rawNode, os.Stdout, opts); err != nil {
			return fmt.Errorf("failed to format: %w", err)
		}
	}

	return nil
}
