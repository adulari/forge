# Mesh routing, straight from the CLI: per-tier picks + quota, then one prompt explained.
# Note: this tape reflects the recording machine's live provider catalog (everything else is --mock).
Output docs/assets/demo-mesh.gif

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

# The overview: subscription quota pressure + the best model per tier.
Type "forge mesh"
Sleep 400ms
Enter
# The catalog/quota probe takes a few seconds — skip the wait frames.
Hide
Wait+Screen@60s /tip:/
Show
Sleep 6s

# Explain how one concrete prompt routes (fallback chain elided — it lists the full catalog).
Type "clear"
Enter
Sleep 500ms
Type "forge mesh 'refactor the auth middleware' | grep -v fallbacks"
Sleep 500ms
Enter
Hide
Wait+Screen@60s /why:/
Show
Sleep 8s
