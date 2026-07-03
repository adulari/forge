# The dedicated full-screen workflow view: phases, live agents, narration, transcript zoom.
Output docs/assets/demo-workflow.gif

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
Type "source scripts/demo/.stage/env.sh && clear"
Enter
Type "forge chat --mock"
Enter
Sleep 3s
Show

# Run the saved .forge/workflows/audit.js — mesh-routed agents fan out per phase.
Type "/workflow run audit"
Sleep 700ms
Enter
Wait+Screen@60s /finished/
Sleep 2500ms

# Zoom into one agent's transcript.
Down
Sleep 700ms
Down
Sleep 700ms
Enter
Sleep 3s
Escape
Sleep 1200ms

# Esc backgrounds the view — a status band stays above the input.
Escape
Sleep 2500ms
