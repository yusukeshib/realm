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
Wait+Screen@5s /New box/

# ── 2. Create a new session via TUI ─────────────────────────────

# "New box..." is already selected, press Enter
Enter
Wait+Screen@5s /Session name/

# Type session name
Type "my-feature"
Sleep 300ms
Enter
Wait+Screen@5s /Image/

# Accept default image (alpine:latest)
Enter
Wait+Screen@5s /Command/

# Accept default command (empty)
Enter

# Wait for workspace clone + container start
Wait+Screen@30s /\$/
Sleep 500ms

# ── 3. Work inside the container ───────────────────────────────

Type "ls"
Sleep 200ms
Enter
Wait+Screen@5s /\$/
Sleep 500ms

Type "git log --oneline -5"
Sleep 200ms
Enter
Wait+Screen@5s /\$/
Sleep 500ms

Type "git remote -v"
Sleep 200ms
Enter
Wait+Screen@5s /\$/
Sleep 500ms

# ── 4. Exit the container ─────────────────────────────────────

Type "exit"
Sleep 200ms
Enter
Wait+Screen@5s /\$/
Sleep 500ms

# ── 5. Resume the session ─────────────────────────────────────

Type "box resume my-feature"
Sleep 300ms
Enter
Wait+Screen@15s /\$/
Sleep 500ms

Type "echo 'Welcome back!'"
Sleep 200ms
Enter
Wait+Screen@5s /Welcome back/
Sleep 500ms

Type "exit"
Sleep 200ms
Enter
Wait+Screen@5s /\$/
Sleep 500ms

# ── 6. Show session in TUI ────────────────────────────────────

Type "box"
Sleep 300ms
Enter
Wait+Screen@5s /my-feature/

# Navigate down to show the session
Down
Sleep 1s

# Quit
Type "q"
Sleep 5s
