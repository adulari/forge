# Provenance: blame a line back to its model/session/prompt, replay the session, fork a counterfactual.
Output docs/assets/demo-provenance.gif

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
Sleep 1s
Show

# Which model wrote each line of this file? (No git involved — Forge's own tool-call records.)
Type "forge blame mock-note.txt"
Sleep 400ms
Enter
Sleep 3500ms

# The full provenance card for one line: model, session, the prompt that caused it.
Type "forge blame mock-note.txt --line 1"
Sleep 400ms
Enter
Sleep 4500ms

# Replay the recorded session, turn by turn.
Type "forge replay @SID@"
Sleep 400ms
Enter
Sleep 5s

# The counterfactual: re-ask the same turn in a fork and diff it against the original.
Type "clear"
Enter
Type "forge fork @SID@ --rerun --mock"
Sleep 400ms
Enter
Wait+Screen@30s /turn diff/
Sleep 3500ms

Type "forge tree"
Sleep 400ms
Enter
Sleep 4s
