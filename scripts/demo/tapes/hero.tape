# README hero GIF — chat, tools, the workflow view, mesh routing, the autopilot digest.
# Recorded fully offline via --mock (see scripts/demo/setup.sh); render with scripts/demo/record.sh.
Output docs/assets/forge-demo.gif

Set Shell "bash"
Set FontFamily "JetBrainsMono Nerd Font"
Set FontSize 14
Set Theme "Catppuccin Mocha"
Set Width 1270
Set Height 780
Set Padding 20
Set Margin 40
Set MarginFill "#181825"
Set BorderRadius 10
Set WindowBar Colorful
Set WindowBarSize 34
Set Framerate 24
Set TypingSpeed 45ms

Hide
Type "source scripts/demo/.stage/env.sh && rm -f mock-note.txt && clear"
Enter
Sleep 1s
Show

# Open the full-screen TUI (offline deterministic provider — no keys needed).
Type "forge chat --mock"
Sleep 500ms
Enter
Sleep 3s

# A read round-trip: mesh classification, streaming, a tool call.
Type "read the project and summarize it"
Sleep 600ms
Enter
Wait+Screen@20s /healthy/
Sleep 1500ms

# A write round-trip: diff preview + the autofix lint pass.
Type "create a file with the project notes"
Sleep 600ms
Enter
Wait+Screen@20s /written/
Sleep 2s

# Fan out a saved multi-agent workflow — the dedicated animated view opens.
Type "/workflow run audit"
Sleep 700ms
Enter
Wait+Screen@60s /finished/
Sleep 3s

# Background the view, then quit the chat.
Escape
Sleep 1200ms
Escape
Sleep 1s

# How the mesh routes, straight from the CLI.
Type "forge mesh"
Sleep 400ms
Enter
# The catalog/quota probe takes a few seconds — skip the wait frames.
Hide
Wait+Screen@60s /tip:/
Show
Sleep 5s

# The overnight autopilot's morning digest.
Type "clear"
Enter
Type "forge queue report"
Sleep 400ms
Enter
Wait+Screen@30s /digest/
Sleep 4500ms
