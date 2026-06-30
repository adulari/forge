---
name: council
description: >-
  Get objective, unbiased analysis of your codebase, project, or documentation. Use when
  you need fresh eyes on a problem, want to challenge existing assumptions, need multiple
  perspectives on architecture or approach, or feel stuck in the current implementation.
  This skill deliberately avoids "existing code bias" by using debiasing techniques,
  multiple viewpoints, and structured synthesis. Perfect for: evaluating if your current
  approach is still optimal, exploring alternatives, identifying hidden assumptions,
  getting objective feedback before major refactors, or when AI tools keep echoing back
  your existing patterns. Trigger when user mentions: "is this still the best approach",
  "evaluate my architecture", "challenge my assumptions", "look at this objectively",
  "what am I missing", "are there better alternatives", "help me think outside the box",
  or similar requests for unbiased analysis. Also use when user wants multiple perspectives
  on a decision, needs to validate ideas against different mental models, or wants a
  "council" of viewpoints.
tier: standard
---

# Council — Multi-Perspective Analysis

A multi-perspective analysis framework that provides objective, debiased evaluation of
codebases, projects, and documentation. Uses multiple viewpoints and debiasing techniques
to overcome the tendency to anchor on existing code.

## The Problem This Solves

AI tools (and humans) tend to anchor on existing code. They see what IS and struggle to
imagine what COULD BE. They validate rather than explore. Council uses:

1. **Debiasing techniques** — Methods that force fresh thinking
2. **Multiple perspectives** — Different mental models that catch different things
3. **Structured synthesis** — Combining insights while tracking disagreements

## When to Use

- Evaluating if current approach is still optimal
- Exploring alternatives to existing implementation
- Identifying hidden assumptions in your project
- Getting objective feedback before major decisions
- When stuck in local optimum
- When you suspect there might be a better way but can't see it
- When AI tools keep echoing back your existing patterns

## Input

User provides:
- **Target**: Repository path, documentation, or specific problem description
- **Goal** (optional): What success looks like (e.g., "maximize profitability")
- **Constraints** (optional): What must be preserved (e.g., "must use existing data model")

## Execution Flow

### Phase 1: Scout

Read and understand the target thoroughly. Document:
- What the project does
- How it's currently structured
- What constraints exist
- What the stated goals are

**Do NOT form opinions yet.** Just gather facts.

### Phase 2: Generate Perspectives

Select 3-5 relevant perspectives based on the target:

| Perspective | Focus | Best For |
|-------------|-------|----------|
| **Architect** | Structure, patterns, coupling, cohesion | System design, refactoring |
| **Challenger** | Assumptions, risks, alternatives | Decision validation, risk mitigation |
| **Opportunist** | New tech, shortcuts, simplification | Optimization, debt reduction |
| **Security** | Trust boundaries, failure modes | Safety, reliability |
| **Economist** | Cost, complexity, ROI | Resource allocation, prioritization |
| **User** | Usability, value, friction | UX, product decisions |
| **Archaeologist** | Legacy, debt, history | Understanding, migration |
| **Futurist** | Trends, scaling, evolution | Long-term planning |

**For complex targets**, use `spawn_agents` to run perspectives in parallel — each agent
works independently to avoid groupthink, then results combine in Phase 3.

**For simpler targets**, run perspectives sequentially in a single agent.

Each perspective applies these debiasing techniques:

1. **Fresh Eyes Mode**: Deliberately ignore the existing implementation. Ask: "What would I build if starting from zero, knowing only the goal?"
2. **Challenge Mode**: Question every assumption. Why does this component exist? What if we removed it? What if the opposite were true?
3. **Rebuild Mode**: "If I had to solve this from scratch with today's knowledge, what would I do differently?"

See `references/debiasing.md` for detailed debiasing techniques and prompts.

### Phase 3: Synthesize

Combine perspectives into structured output:

1. **Agreements**: What multiple perspectives agree on (high confidence)
2. **Disagreements**: Where perspectives conflict (dig deeper — these reveal tradeoffs)
3. **Novel Insights**: Things only one perspective caught
4. **Prioritized Recommendations**: Ranked by impact and confidence

See `agents/synthesizer.md` for detailed synthesis instructions when using subagents.

### Phase 4: Validate

- Check that recommendations don't contradict each other
- Flag any assumptions that weren't validated
- Identify what information would change conclusions

## Output Format

```markdown
# Council Report: [Target]

## Executive Summary
[2-3 sentence objective assessment — not a summary of what was done, but a verdict]

## Key Findings

### [Finding Name]
- **What**: [Clear description of the finding]
- **Perspectives that agree**: [List]
- **Perspectives that disagree**: [List + brief reason]
- **Confidence**: [High/Medium/Low]
- **Evidence**: [Specific file:line, metric, or observation]

## Recommendations

### [HIGH IMPACT] [Recommendation Title]
- **Problem addressed**: [The finding this fixes]
- **Solution**: [Concrete action]
- **Expected impact**: [What improves and by how much]
- **Effort**: [Low/Medium/High]
- **Risk**: [Low/Medium/High]
- **Perspectives supporting**: [List]

### [MEDIUM IMPACT] ...

### [LOW IMPACT] ...

## Alternative Approaches Considered

| Approach | Pros | Cons | Verdict |
|----------|------|------|---------|
| ...      | ...  | ...  | Keep/Replace/Explore further |

## What Would Change This Assessment
- If [X] is true, then [recommendation Y changes]
- [Unvalidated assumption that matters]
```

## Bundled Resources

- `references/debiasing.md` — Detailed debiasing techniques and prompts per perspective type
- `agents/perspective.md` — Instructions for subagents running individual perspective analyses
- `agents/synthesizer.md` — Instructions for a synthesis subagent combining multiple perspective outputs