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

# Render yam tree output
Type "./yam testdata/sample.yaml"
Enter
Sleep 1.5s

Type "clear"
Enter
Sleep 300ms
Type "./yam --raw .metadata.name testdata/sample.yaml"
Enter
Sleep 800ms

Type "./yam diff --summary testdata/config-dev.yaml testdata/config-prod.yaml"
Enter
Sleep 1s
