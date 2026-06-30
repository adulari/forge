# Debiasing Techniques

Techniques to avoid anchoring on existing code and think creatively.

## 1. Fresh Eyes Mode

**Purpose**: See the problem without being contaminated by existing implementation.

**How to use**:
- Before reading any code, write what you would build
- Then read the code and compare
- Ask: "What would I have done differently?"

**Questions to ask**:
- If I had never seen this code, what would I assume it does?
- What would a newcomer think is happening here?
- What would I Google if I saw this for the first time?

## 2. Ignore Mode

**Purpose**: Deliberately disregard existing implementation when generating alternatives.

**How to use**:
- List what the current code does
- Cross out each item and ask: "Is this necessary?"
- Generate solutions that don't use any existing patterns

**Questions to ask**:
- What if this entire module didn't exist?
- What if I had to solve this with half the code?
- What if I could only use standard library?

## 3. Rebuild Mode

**Purpose**: Solve the problem from scratch without constraints.

**How to use**:
- Define the inputs and outputs only
- Design the ideal solution
- Then compare to existing and note gaps

**Questions to ask**:
- If I had unlimited time, how would I solve this?
- What would the perfect solution look like?
- What's the simplest thing that could work?

## 4. Challenge Mode

**Purpose**: Question every assumption.

**How to use**:
- List all assumptions in the current approach
- For each, ask "What if the opposite were true?"
- Look for hidden constraints that aren't actually required

**Questions to ask**:
- What assumptions is this design based on?
- What would break if we changed X?
- Is this constraint real or self-imposed?
- What would a competitor do differently?

## 5. Inversion Mode

**Purpose**: Flip the problem to see it from opposite angle.

**How to use**:
- Instead of "How do we achieve X?", ask "How would we prevent X?"
- Instead of "How to optimize for Y?", ask "How to maximize failure of Y?"

**Questions to ask**:
- How would you make this fail?
- What would make this approach completely wrong?
- What's the worst possible implementation?

## 6. First Principles Mode

**Purpose**: Break down to fundamental truths.

**How to use**:
- Identify what you're really trying to accomplish
- Break this into atomic truths
- Rebuild from there

**Questions to ask**:
- What is this actually trying to do?
- What are the fundamental constraints?
- Is there a simpler way to satisfy these constraints?

## 7. Analogical Mode

**Purpose**: Use patterns from unrelated domains.

**How to use**:
- Find analogous systems in nature/business/other fields
- Extract the pattern
- Apply to current problem

**Questions to ask**:
- How does nature solve this?
- What does this remind me of from a completely different domain?
- What patterns from other fields could apply?

## 8. Time Travel Mode

**Purpose**: View from past or future perspective.

**How to use**:
- Imagine this was built 10 years ago (what would you change?)
- Imagine this needs to scale 100x (what breaks?)
- Imagine you're looking back from 5 years in the future

**Questions to ask**:
- What would you have done differently if building this in 2014?
- What will definitely need to change in 5 years?
- What decisions will you regret?

## Applying Debiasing in Council

For each perspective, apply at least 2 debiasing techniques:

| Perspective | Primary Technique | Secondary Technique |
|-------------|-------------------|---------------------|
| Architect | Fresh Eyes | First Principles |
| Challenger | Challenge Mode | Inversion |
| Opportunist | Ignore Mode | Rebuild |
| Security | Inversion | Time Travel |
| Economist | First Principles | Time Travel |
| User | Fresh Eyes | Analogical |
| Archaeologist | Time Travel | Challenge |
| Futurist | Time Travel | Rebuild |

## Warning Signs of Bias

Watch for these in your analysis:

- "The current implementation..."
- "Since they chose..."
- "The problem with this approach is..."
- "Obviously..."
- "Everyone knows..."
- "The standard way is..."

If you catch yourself using these, apply a debiasing technique.