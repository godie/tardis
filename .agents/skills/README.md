# Vendored skills

Agent skills the PR gate in [`AGENTS.md`](../../AGENTS.md#before-opening-a-pr)
calls. They live here so a clone is enough to run the gate, in the
agent-agnostic `.agents/skills/` layout: OpenCode, and any other agent that
follows that convention, reads `<repo>/.agents/skills/<name>/SKILL.md`
directly. Agents that only read a global directory get them through
[`scripts/install-skills.sh`](../../scripts/install-skills.sh).

| Skill | Source | Licence |
| --- | --- | --- |
| `no-ai-slop` | `~/.agents/skills/no-ai-slop` (skill author's local install) | MIT |
| `caveman-review` | `~/.agents/skills/caveman-review` (skill author's local install) | MIT |
| `ponytail-review` | `@dietrichgebert/ponytail`, `skills/ponytail-review` | MIT, Copyright (c) 2026 DietrichGebert |

Updating: copy the new `SKILL.md` (and `eval.md` for `no-ai-slop`) over the
vendored file, and note the change in the commit. Do not edit the copies by
hand: a local edit is lost on the next update.

Not vendored on purpose:

- The rest of `@dietrichgebert/ponytail` (`audit`, `debt`, `help`, `gain`).
  Install the plugin if you want the full set.
- The `qodo-*` skills. They depend on the Qodo CLI and its account, so they
  are installed per machine with `qodo skills install <name>`.
