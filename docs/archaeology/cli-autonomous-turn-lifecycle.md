# Code archaeology: CLI autonomous turn lifecycle

Autonomous loop and goal mode were hardened after stale done signals could
corrupt FIFO prompt draining. The task owns generation-bound completion,
progress/stall termination, and one presenter cleanup path for both local and
remote interruption. Workflow state must close on abort because no future
WorkflowFinished event will arrive.

The extracted helpers intentionally remain adjacent: splitting task stop policy
from its spawning/rebinding machinery would make queued corrections and stale
completion races harder to see. Characterization lives in CLI run/driver tests
for FIFO steering, interrupt/re-prompt behavior, model skip rebinding, goal
stalls/completion, and workflow presenter cleanup.
