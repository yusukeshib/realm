Output docs/demo.gif
Set Shell bash
Set FontSize 15
Set Width 900
Set Height 450
Set Padding 16
Set Theme "Catppuccin Mocha"
Set TypingSpeed 40ms

# Create first session
Type "realm my-feature -c --image alpine/git"
Sleep 500ms
Enter
Sleep 3s

# Work inside the container
Type "git checkout -b experiment"
Enter
Sleep 1s

Type "echo 'hello' > test.txt && git add . && git commit -m 'experiment'"
Enter
Sleep 2s

# Exit container
Type "exit"
Enter
Sleep 2s

# Create second session
Type "realm bugfix -c --image alpine/git"
Sleep 500ms
Enter
Sleep 3s

Type "exit"
Enter
Sleep 2s

# List sessions via TUI
Type "realm"
Sleep 500ms
Enter
Sleep 2s

# Navigate and quit
Down
Sleep 500ms
Type "q"
Sleep 1.5s

# Delete a session
Type "realm bugfix -d"
Sleep 500ms
Enter
Sleep 1.5s

# Host is untouched
Type "git status"
Enter
Sleep 2s
