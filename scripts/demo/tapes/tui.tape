# Full-screen TUI chat: streaming turns, tool calls, diff preview, autofix, temper switching.
Output docs/assets/demo-tui.gif

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

Type "forge chat --mock"
Sleep 500ms
Enter
Sleep 3s

# Read round-trip: classification in the statusline, streaming text, a tool call rendering.
Type "read the project and summarize it"
Sleep 600ms
Enter
Wait+Screen@20s /healthy/
Sleep 2s

# Write round-trip: diff preview plus the automatic lint pass after the edit.
Type "create a file with the project notes"
Sleep 600ms
Enter
Wait+Screen@20s /written/
Sleep 2s

# Shift+Tab cycles the temper (permission mode) — watch the statusline.
Shift+Tab
Sleep 1200ms
Shift+Tab
Sleep 1200ms
Shift+Tab
Sleep 2500ms
