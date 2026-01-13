package diff

// DiffSummary holds statistics about the differences between two YAML files
type DiffSummary struct {
	Added    int // Count of added nodes
	Removed  int // Count of removed nodes
	Modified int // Count of modified nodes
	Total    int // Total count of changes
}

// DiffResult represents the complete result of comparing two YAML files
type DiffResult struct {
	Root      *DiffNode   // Root of the diff tree
	Summary   DiffSummary // Summary statistics
	LeftFile  string      // Path to file1
	RightFile string      // Path to file2
}
