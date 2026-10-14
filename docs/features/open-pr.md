# `open_pr`: deterministic pull requests

`open_pr` turns the committed work on the current branch into a GitHub pull request. The model
supplies a title and a short what/why body. Everything else (the checks, the push, the evidence
and provenance sections, the `gh pr create` call) is code, so the model cannot skip or misformat
it, and cannot invent proof.

`/pr [title]` is a one-line prompt over it: commit the work on a feature branch, then call
`open_pr`.

## Arguments

| arg | meaning |
|---|---|
| `title` | required, one line |
| `body` | required: what changed and why, in Markdown. Evidence and provenance are appended for you |
| `base` | base branch; default is the repository's default branch |
| `draft` | open as a draft |
| `allow_untracked` | go ahead even though untracked files exist (they are not part of the PR) |
| `publish_images` | push captured screenshots to `forge-evidence` (default `true`) |

## What it does, in order

1. **Refuses a dirty tree.** Any staged or unstaged change to a tracked file stops the call, with
   the paths listed. Untracked files stop it too unless `allow_untracked: true`. Paths under
   `.forge/` (evidence, jobs, checkpoints) are Forge's own state and are ignored. The tool never
   stages or commits anything itself.
2. **Refuses a detached HEAD, and the base branch itself.**
3. **Resolves the push remote**: `branch.<b>.pushRemote`, then `remote.pushDefault`, then
   `branch.<b>.remote`, then `origin` or the only remote. The GitHub repository comes from the
   remote's configured URL (`pushurl`, else `url`), before `insteadOf` rewriting. A shorthand
   whose "host" has no dot (`gh:owner/repo`) is resolved with `git remote get-url --push` instead.
4. **Checks write access** with `gh repo view <owner/repo> --json viewerPermission,defaultBranchRef,isArchived`.
   Only `ADMIN`, `MAINTAIN` and `WRITE` may push; anything else (`TRIAGE`, `READ`, none) is
   refused before any push, and so is an archived repository.
5. **Refuses a branch with no commits ahead** of `<remote>/<base>` (when that remote-tracking ref
   exists locally).
6. **Pushes** with `git push -u <remote> HEAD:refs/heads/<branch>`. No force push is ever
   attempted; a rejected push is reported as is. Hooks run as usual.
7. If the branch **already has an open PR** from this repository, the push updates it and the tool
   returns that PR's URL without touching its title or body. A fork's PR from a same-named branch
   (`isCrossRepository`) does not count, since the push never reaches it.
8. **Publishes screenshots** (see below), then runs
   `gh pr create --repo <slug> --base <base> --head <branch> --title … --body-file -`.

The result names the PR URL, the branch and base, the commit count, and what evidence went in.

## The body

```markdown
<the model's body>

---
### Evidence (captured, not reviewed)
Checks that passed during the session:
- `cargo test -p shop` — exit 0 at … (`.forge/evidence/<id>/0002-shell.txt`, sha256 `…`)
Screenshots:
- `.forge/evidence/<id>/0001-browser.png` (browser, http://localhost:3000/cart, sha256 `…`) — empty cart
  ![empty cart](https://github.com/acme/shop/blob/<commit>/<id>/0001-browser.png?raw=true)
Preview URLs (temporary public tunnels; they close with the dev server or after their TTL):
- https://x.trycloudflare.com (job 4242 port 5173, opened …)

### Provenance
- Session: `<id>` — inspect with `forge replay <short>`
- Models: …
- Spend: …
- Diff: 3 files changed, 40 insertions(+), 2 deletions(-)

🔨 Created with Forge
```

The evidence section comes from the [evidence ledger](evidence-ledger.md). **Preview URLs** are
recorded in the ledger by every `shell_job` expose ([shell-tool.md](shell-tool.md)); `open_pr`
lists only the ones whose tunnel is still up and counts the closed ones, since a dead link reads
like a broken deploy. The diff line is `git diff --shortstat <remote>/<base>...HEAD`, computed by
the tool. The title and the whole body go through the same secret redaction as `forge replay`
([redaction.md](redaction.md)) before they leave the machine. A bare call outside any session gets
the model's body and the footer only.

## Evidence images: the `forge-evidence` branch

GitHub has no API for attaching images to a PR. Screenshots are therefore committed to an orphan
branch, `forge-evidence`, under `<session>/<file>`, with git plumbing only: `hash-object -w`, a
private index file under the git dir (`read-tree` of the current tip, `update-index --cacheinfo`),
`write-tree`, `commit-tree --no-gpg-sign` as `Forge <forge@adulari.dev>`, and
`git push <remote> <commit>:refs/heads/forge-evidence`. The worktree, the real index and HEAD are
never touched. Each publish builds on the remote tip, so other sessions' evidence stays; an
identical publish makes no new commit, and one retry covers a concurrent publish. Links point at
the commit, not the branch, so they never change; they render for anyone who can read the repo,
private repos included.

Only intact captures are published. The manifest is a plain file in the workspace, so each
screenshot entry must name a plain image file (`png`, `jpg`, `jpeg`, `webp`, `gif`) inside the
session's evidence directory, be a regular file rather than a symlink, and still match the sha256
the manifest recorded. Anything else is left out and named in the result.

Publishing is skipped (and the PR names images by local path) when `publish_images: false`, when
`[pr] publish_evidence_images = false` is set in the user config or in the workspace's or
repository's `.forge/config.toml`, or when the push fails. `false` in any of those places wins;
the repository files are read on every call, so a daemon serving many repositories honours each
one's own setting. The same write-permission check covers publishing: nothing is pushed to a
repository the user cannot write to.

```toml
[pr]
publish_evidence_images = true   # false: keep screenshots local
```

Screenshots can show secrets (logged-in pages, an attached real browser profile). The permission
prompt for `open_pr` is the moment to say no, or pass `publish_images: false`.

## Permissions

`open_pr` is a registry tool whose side effect is `External`: it asks in the default,
accept-edits and auto modes, is denied in plan mode, and runs unprompted only in bypass. A
permission rule on the shell (`git push`, `gh pr create`) does not reach it, because it runs
neither through the shell tool; a rule naming the tool does:

```toml
[[permissions.rules]]
tool = "open_pr"
deny = "*"
``` It is never part
of a concurrent read-only batch. Because it lives in the core `ToolRegistry`, it is available on
the direct path and through a CLI bridge's `forge mcp-serve`, where it sits in the deferred set
behind `tool_search`/`tool_call`. The direct path's lean surface defers it the same way: it is
listed in `tool_search`'s catalog, and `tool_call{"name":"open_pr"}` (or a `run_code` script) runs
it through the normal permission gate.

`git` and `gh` run with no TTY: `GIT_TERMINAL_PROMPT=0`, `GH_PROMPT_DISABLED=1`, ssh in batch mode
(unless `GIT_SSH_COMMAND` is set), and a 180 s timeout per command.

## Code

- `crates/forge-tools/src/open_pr.rs`: the tool and the flow.
- `crates/forge-tools/src/open_pr/evidence_branch.rs`: the orphan-branch publish.
- `crates/forge-tools/src/open_pr/screenshots.rs`: which ledger screenshots may be published.
- `crates/forge-tools/src/open_pr/github.rs`: remote URL parsing and `gh` output.
- `crates/forge-tools/src/evidence/pr_block.rs`: the evidence section.
- `crates/forge-tools/src/shell/preview_links.rs`: preview URLs as evidence, and their liveness.
- `crates/forge-cli/src/open_pr_hooks.rs`: provenance and redaction, installed at startup.
