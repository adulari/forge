# Code archaeology: Core text policy

Recap and suggestion normalization is deliberately deterministic because an
auxiliary model previously inverted an explicit task-completion state. A recap
may summarize completed tasks only when tracked state changed and the final
answer independently reports completion; negative completion wording always
wins. Suggestions are one bounded visible line and must not echo the preceding
prompt. Memory scope derives from the explicit session root so multi-workspace
sessions never share project memories.

These are pure policy functions, safe to extract together but unsafe to
simplify without the task/long-session characterization. The module preserves
the existing function bodies and Session callers.
