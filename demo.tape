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

Sleep 500ms
Type "box"
Sleep 300ms
Enter
Sleep 1s

# ── 2. Create a new session ────────────────────────────────────

# "New box..." is already selected, press Enter
Enter
Sleep 500ms

# Type session name
Type "my-feature"
Sleep 300ms
Enter
Sleep 500ms

# Accept default image (alpine:latest)
Enter

# Wait for workspace clone + container start
Sleep 500ms

# ── 3. Work inside the container ───────────────────────────────

Type "ls"
Sleep 200ms
Enter
Sleep 1s

Type "git log --oneline -5"
Sleep 200ms
Enter
Sleep 1s

Type "git remote -v"
Sleep 200ms
Enter
Sleep 1s

# ── 4. Exit the container ─────────────────────────────────────

Type "exit"
Sleep 200ms
Enter
Sleep 1s

# ── 5. Resume the session ─────────────────────────────────────

Type "box my-feature"
Sleep 300ms
Enter
Sleep 2s

Type "echo 'Welcome back!'"
Sleep 200ms
Enter
Sleep 1s

Type "exit"
Sleep 200ms
Enter
Sleep 1s

# ── 6. Show session in TUI ────────────────────────────────────

Type "box"
Sleep 300ms
Enter
Sleep 2s

# Navigate down to show the session
Down
Sleep 1s

# Quit
Type "q"
Sleep 5s
