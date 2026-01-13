# VHS Demo for yam
# Run: vhs demo.tape

Output demo.gif

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 700
Set Padding 20
Set Theme "Catppuccin Mocha"
Set TypingSpeed 50ms

# Start yam in interactive mode
Type "./yam -i testdata/sample.yaml"
Enter
Sleep 1.5s

# Navigate down through the tree
Sleep 500ms
Type "jjjjj"
Sleep 800ms

# Fold metadata section
Type "kkk"
Sleep 500ms
Enter
Sleep 1s

# Unfold
Enter
Sleep 1s

# Navigate to data section and fold it
Type "jjjjjj"
Sleep 500ms
Enter
Sleep 1s

# Unfold data
Enter
Sleep 800ms

# Collapse all nodes
Type "C"
Sleep 1.2s

# Expand all nodes
Type "O"
Sleep 1.2s

# Search for "host"
Type "/"
Sleep 400ms
Type "host"
Sleep 600ms
Enter
Sleep 800ms

# Next match
Type "n"
Sleep 600ms

# Go to top
Type "g"
Sleep 800ms

# Navigate to name field for editing
Type "jjjjj"
Sleep 600ms

# Edit value
Type "e"
Sleep 600ms

# Clear and type new value
Ctrl+U
Type "my-awesome-config"
Sleep 800ms

# Confirm edit
Enter
Sleep 1s

# Quit (twice because of unsaved changes warning)
Type "q"
Sleep 500ms
Type "q"
Sleep 1s
