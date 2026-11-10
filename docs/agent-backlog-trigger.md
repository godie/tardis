# Autonomous backlog trigger

The trigger that lets the agent pick up a small backlog ticket without a
human opening a session. Built from tools the repo already has: GitHub
Issues as the queue, the `gh` CLI as the connection, `opencode run
--auto` as the headless agent, launchd as the schedule.

## The loop

```
gh issue label ready-for-agent
  -> launchd fires scripts/agent-backlog-pickup.sh every 1800 s
     -> claims the lowest-numbered labeled ticket (ready-for-agent -> agent-wip)
     -> opencode run --auto: ticket + AGENTS.md rules -> branch -> matrix -> PR
     -> qodo review capped at 2 fix rounds, dispositions documented
     -> exit non-zero: agent-wip -> agent-stuck, human comment on the ticket
```

Guarantees, in order of importance:

1. **Everything is a label.** `ready-for-agent` is the only way a run
   starts; `agent-wip` means claimed; `agent-stuck` means the retry cap
   hit. No cron black box: the ticket itself is the state machine.
2. **Nothing merges.** The agent stops at an open PR; merge is a human
   decision, always.
3. **One at a time.** A pid-checked lock in `output/agent-runs/` (a
   stale lock from a dead pid is reaped automatically) and the label
   claim that happens before any work starts.
4. **Retry cap.** Two attempts per blocked step inside the session;
   a non-zero exit re-labels the ticket and posts a path-specific
   blocker comment. The only way a stuck ticket runs again is a human
   re-applying `ready-for-agent`.

## Setup (once)

```bash
gh label create ready-for-agent --description "small ticket the agent may pick up autonomously" -C 0E8A16
gh label create agent-wip          --description "an agent session currently claimed this ticket" -C F9D0C4
gh label create agent-stuck        --description "agent run stoped at the retry cap; human input needed" -C FBCA04
scripts/install-agent-trigger.sh
```

The installer does not fire a run: `RunAtLoad` is off on purpose.

## Manual run an assert

```bash
gh issue edit 3 --add-label ready-for-agent   # or launchctl kickstart
launchctl kickstart gui/$(id -u)/dev.tardis.agent-pickup
tail -f output/agent-runs/trigger.log
```

Session transcripts land in `output/agent-runs/issue-<n>-*.log`
(gitignored, like everything under `/output`).

## What the agent promises on each run

Read the ticket and both rule files before touching code · branch off
`origin/master` (never commit / force-push to master) · implement only
what the ticket names · validation matrix with counts in the PR body ·
PR with `Closes #N` (never closes the ticket directly unless merged) ·
review loop on the PR's comments until the latest review shows zero
unresolved findings, capped at 3 rounds with the dispositions
documented, then stop with a specific blocker comment instead of
inventing scope · leave no mess behind.

## Removing the trigger

```bash
scripts/install-agent-trigger.sh remove
```
