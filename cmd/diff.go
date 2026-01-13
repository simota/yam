package cmd

import (
	"fmt"
	"os"

	"github.com/simota/yam/internal/diff"
	"github.com/simota/yam/internal/parser"
	diffui "github.com/simota/yam/internal/ui/diff"
	"github.com/spf13/cobra"
)

var summaryOnly bool
var diffInteractive bool

var diffCmd = &cobra.Command{
	Use:   "diff <file1> <file2>",
	Short: "Compare two YAML/JSON files",
	Long: `Compare two YAML or JSON files and show structural differences.

The diff command parses both files and performs a structural comparison,
showing added, removed, and modified values. File format (YAML or JSON)
is automatically detected based on file extension.

Exit codes:
  0  No differences found
  1  Differences found
  2  Error occurred

Examples:
  yam diff config-dev.yaml config-prod.yaml
  yam diff --summary config-dev.yaml config-prod.yaml
  yam diff -i config-dev.yaml config-prod.yaml  # Interactive TUI mode
  yam diff config.yaml config.json  # Cross-format comparison`,
	Args: cobra.ExactArgs(2),
	RunE: runDiff,
}

func init() {
	rootCmd.AddCommand(diffCmd)
	diffCmd.Flags().BoolVarP(&summaryOnly, "summary", "s", false, "Show only summary (no detailed diff)")
	diffCmd.Flags().BoolVarP(&diffInteractive, "interactive", "i", false, "Interactive TUI mode with split view")
}

func runDiff(cmd *cobra.Command, args []string) error {
	file1 := args[0]
	file2 := args[1]

	// Parse both files
	left, err := parseFile(file1)
	if err != nil {
		return fmt.Errorf("failed to parse %s: %w", file1, err)
	}

	right, err := parseFile(file2)
	if err != nil {
		return fmt.Errorf("failed to parse %s: %w", file2, err)
	}

	// Compare the two parsed trees
	result := diff.Compare(left, right)
	result.LeftFile = file1
	result.RightFile = file2

	// Interactive TUI mode
	if diffInteractive {
		return diffui.Run(result, left, right)
	}

	// Render output
	if summaryOnly {
		fmt.Println(diff.RenderSummary(result.Summary))
	} else {
		if result.Summary.Total == 0 {
			fmt.Println("No differences found.")
		} else {
			output := diff.Render(result)
			fmt.Print(output)
		}
	}

	// Exit with code 1 if there are differences
	if result.Summary.Total > 0 {
		os.Exit(1)
	}

	return nil
}

// parseFile opens and parses a file, detecting format from extension
func parseFile(filename string) (*parser.YamNode, error) {
	f, err := os.Open(filename)
	if err != nil {
		return nil, err
	}
	defer f.Close()

	p := parser.New()

	if isJSONFile(filename) {
		return p.ParseJSON(f)
	}
	return p.Parse(f)
}
