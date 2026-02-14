# VHS demo tape for box
# Run: vhs demo.tape
# Prerequisites: Docker running, inside a git repo

Output demo.gif

Set Shell "bash"
Set FontSize 14
Set Width 1100
Set Height 700
Set Padding 20
Set TypingSpeed 60ms

# ── 1. Launch TUI session manager ──────────────────────────────

Sleep 1000ms
Type "box"
Sleep 500ms
Enter

Sleep 1500ms

# ── 2. Create a new session via TUI ─────────────────────────────

Down
Sleep 500ms
Up
Sleep 500ms

# "New box..." is already selected, press Enter
Enter
Sleep 1500ms

# Type session name
Type "my-feature"
Sleep 500ms
Enter
Sleep 1500ms
 
# Accept default image (alpine:latest)
Enter
Sleep 1500ms

# Accept default command (empty)
Type "zsh"
Sleep 1000ms
Backspace 3
Sleep 500ms
Enter
Sleep 1500ms

# Wait for workspace clone + container start
Wait+Screen@5s /\#/
Sleep 3000ms
 
Enter
Sleep 500ms
Type "you are in a box session!"
Sleep 500ms
Backspace 25
Sleep 500ms
Enter
Sleep 500ms
Enter
Sleep 1500ms

# ── 3. Work inside the container ───────────────────────────────

Type "ls"
Sleep 200ms
Enter
Wait+Screen@5s /\#/
Sleep 500ms

Enter
Sleep 500ms
Type "exit"
Sleep 500ms
Enter
Sleep 500ms
Enter
Wait+Screen@5s /\>/
 
Sleep 5s
