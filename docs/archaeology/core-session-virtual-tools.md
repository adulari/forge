# Code archaeology: Core session virtual tools

Virtual tools mutate Session state rather than the workspace: a question blocks
only through the presenter, update_tasks persists the full merged list before
emitting it, and present_plan separates an unapproved proposal from active
work. The approval path captures/restores the prior permission mode so Plan
cannot accidentally leave a resumed session in a weaker mode.

Memory and skills are also stateful: durable facts are recorded through Store
with category validation; loaded skill guidance is persisted into the transcript
in order. These operations must remain beside their presenter and audit calls,
not be disguised as generic registry tools. Existing Core question/task/plan/
skill/memory tests characterize the behavior.
