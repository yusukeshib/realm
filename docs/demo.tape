Output docs/demo.gif
Set Shell bash
Set FontSize 15
Set Width 900
Set Height 450
Set Padding 16
Set Theme "Catppuccin Mocha"
Set TypingSpeed 40ms

# Create a session with custom image and command
Type "realm my-feature --image ubuntu:latest -- bash"
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

# Resume with zero flags — session persists
Type "realm my-feature"
Sleep 500ms
Enter
Sleep 3s

# Verify work is still there
Type "git log --oneline -1"
Enter
Sleep 1.5s

Type "exit"
Enter
Sleep 2s

# Create session with extra Docker flags
Type "realm bugfix --docker-args \"-e DEBUG=1\" --image ubuntu:latest -- bash"
Sleep 500ms
Enter
Sleep 3s

Type "echo $DEBUG"
Enter
Sleep 1.5s

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
