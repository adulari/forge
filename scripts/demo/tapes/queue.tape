# The overnight autopilot: queue tasks, drain them into isolated worktrees, read the digest.
Output docs/assets/demo-queue.gif

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

# Queue work for later — with a hard budget cap.
Type "forge queue add 'create a file with the API changelog' --budget 2.50"
Sleep 400ms
Enter
Sleep 2s

Type "forge queue add 'read the project and summarize it'"
Sleep 400ms
Enter
Sleep 2s

Type "forge queue"
Sleep 400ms
Enter
Sleep 3s

# Drain: each task runs headless in its own git worktree and leaves a review-ready branch.
Type "clear"
Enter
Type "forge queue run --mock"
Sleep 400ms
Enter
Wait+Screen@90s /for the digest/
Sleep 2s

# The morning digest + the result branches.
Type "forge queue report"
Sleep 400ms
Enter
Sleep 4s

Type "git branch --list 'autopilot/*'"
Sleep 400ms
Enter
Sleep 3500ms
