#!/bin/bash
# Autonomous backlog pickup: one labeled ticket, one agent, one PR.
#
# Trigger contract:
#   label ready-for-agent  ->  a run claims it (label moves to agent-wip
#   before any work starts, so a second tick cannot double-fire) -> a
#   headless opencode session implements it under the repo's rules -> a
#   PR is opened with `Closes #N`, qodo review capped, nothing merged.
#
# Retry cap: the agent gets two attempts per blocked step inside its
# session. If the session exits non-zero, the issue gets `agent-stuck`
# and waits for a human. A fresh `ready-for-agent` label is the only way
# a run starts, so a stuck ticket cannot loop forever.
#
# Never on this script's path: merge, force-push, direct ticket closure.
# Runs identically on the Mac and on the instabox box (bash, POSIX gh).

set -u

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
RUN_DIR="${REPO_DIR}/output/agent-runs"
LOCK_DIR="${RUN_DIR}/lock"

console_log() { echo "$(date -u '+%FT%TZ') $*"; }

cd "${REPO_DIR}" || exit 1

# --help must exit before ANY side effect: no board sync, no label
# claim, no lock, no agent. It exists so an installer probing the
# script cannot accidentally start a session (launchd's RunAtLoad is
# off; a probe must be just as inert).
for arg in "$@"; do
  if [ "${arg}" = "--help" ] || [ "${arg}" = "-h" ]; then
    echo "usage: agent-backlog-pickup.sh [--help]"
    echo "  no arguments: claim one ready-for-agent issue and run the"
    echo "  agent on it. --help: this text, and nothing else."
    exit 0
  fi
done

# The fully-qualified repo the guard scripts act on: gh autodetects it
# from the checkout, and the board sync compares items against it.
# Overridable for tests (a fake TARDIS_REPO_FULL with a PATH shim).
TARDIS_REPO_FULL="${TARDIS_REPO_FULL:-$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null)}"

# On the instabox box the managed environment (org Secrets like
# GH_TOKEN) loads in login shells only, via /etc/profile.d; a launcher
# is usually non-login, so load it here when present. On the Mac the
# file never exists and nothing changes.
[ -r /run/instabox/secrets/env ] && . /run/instabox/secrets/env

mkdir -p "${RUN_DIR}"

# --- board -> label sync (the second trigger condition) -------------------
# A GitHub Projects move does not touch labels, so the tick reads the
# board's Status column ("Ready for agent") and mirrors it into the
# ready-for-agent label. Direction is one-way (board -> label) and the
# run's own state labels win: an issue already carrying ready-for-agent,
# agent-wip or agent-stuck is never re-touched, so once work starts the
# claim state machine stays the single authority. The board itself is
# not written back; the human moves it after merge. The board and the
# ready queue are both read to exhaustion, and only this repo's items
# are touched, so a shared board or a long queue cannot hide a ticket.

# One page of the board query. The cursor is omitted on the first page
# because GitHub rejects an empty `after` value.
board_page() { # $1 query, $2 owner, $3 number, $4 cursor (empty first)
  if [ -n "$4" ]; then
    gh api graphql -f "query=$1" -f "owner=$2" -F "number=$3" -f "cursor=$4"
  else
    gh api graphql -f "query=$1" -f "owner=$2" -F "number=$3"
  fi
}

sync_board_labels() {
  local board_query cursor page_response page_lines page_info
  local has_next next_cursor line repo issue state labels
  local owner number
  owner="${AGENT_BOARD_OWNER:-godie}"
  number="${AGENT_BOARD_NUMBER:-3}"
  cursor=""
  # `gh repo view` resolves the name from the checkout and can fail,
  # leaving it empty. Every item would then pass the
  # `repo != TARDIS_REPO_FULL` check against "", including a board item
  # that carries no repository at all (a pull request or a draft), so
  # the sync would label a stranger's issue. Label-only pickup still
  # runs.
  if [ -z "${TARDIS_REPO_FULL}" ]; then
    console_log "board sync: repository unknown, label-only pickup still runs"
    return 0
  fi
  board_query='query($owner: String!, $number: Int!, $cursor: String) {
    user(login: $owner) {
      projectV2(number: $number) {
        items(first: 100, after: $cursor) {
          pageInfo { hasNextPage endCursor }
          nodes {
            content { ... on Issue { number state repository { nameWithOwner } } }
            fieldValues(first: 10) {
              nodes { ... on ProjectV2ItemFieldSingleSelectValue { name } }
            }
          }
        }
      }
    }
  }'

  # Walk every page until `pageInfo` says the board is exhausted. A cap
  # here would drop a ready item past the last page, and the docs promise
  # the lowest ready ticket from the whole queue.
  while :; do
    if ! page_response="$(board_page "${board_query}" "${owner}" "${number}" "${cursor}" 2>&1)"; then
      console_log "board sync: gh graphql query failed, label-only pickup still runs"
      break
    fi
    page_lines="$(printf '%s\n' "${page_response}" | jq -r '
      .data.user.projectV2.items.nodes[]?
      | select([.fieldValues.nodes[].name] | index("Ready for agent") != null)
      | .content.repository.nameWithOwner + "#" + (.content.number|tostring)
        + " / " + (.content.state // "?")
    ' 2>/dev/null)"

    printf '%s\n' "${page_lines}" | while IFS= read -r line; do
      [ -n "${line}" ] || continue
      repo="${line%%#*}"
      # A shared board carries items from every repo; only this repo's
      # issues may be touched (delegate finding 4: `other-repo#5` must
      # not be checked against, or labeled as, `tardis#5`).
      if [ "${repo}" != "${TARDIS_REPO_FULL}" ]; then
        continue
      fi
      issue="${line#*#}"
      issue="${issue%% *}"
      state="${line##* }"
      [ "${state}" = "OPEN" ] || continue
      labels="$(gh issue view "${issue}" --json labels -q '[.labels[].name]|join(",")' 2>/dev/null)"
      case ",${labels}," in
        *,ready-for-agent,*|*,agent-wip,*|*,agent-stuck,*) continue ;;
      esac
      if gh issue edit "${issue}" --add-label ready-for-agent >/dev/null 2>&1; then
        console_log "board sync: issue #${issue} earned ready-for-agent from the [Ready for agent] column"
      fi
    done

    page_info="$(printf '%s\n' "${page_response}" \
      | jq -r '.data.user.projectV2.items.pageInfo | "\(.hasNextPage // false) \(.endCursor // "")"' 2>/dev/null)"
    has_next="${page_info%% *}"
    next_cursor="${page_info#* }"
    if [ "${has_next}" != "true" ] || [ -z "${next_cursor}" ]; then
      break
    fi
    cursor="${next_cursor}"
  done
}
sync_board_labels

# One run at a time. Recovery is atomic under a dedicated reaper lock:
# only the tick that holds `lock/reaper` may rm+recreate `lock`, so two
# concurrent ticks can never both read a dead PID, delete each other's
# fresh lock and run in parallel (delegate finding 3). A missing,
# empty or unparsable pid file counts as stale: `kill -0 0` targets the
# process group and always succeeds, which would have stranded the
# lock forever.
if ! mkdir "${LOCK_DIR}" 2>/dev/null; then
  if ! mkdir "${LOCK_DIR}.reaper" 2>/dev/null; then
    console_log "trigger: a previous run is active and another tick is already reaping; skipping tick"
    exit 0
  fi
  trap 'rm -rf "${LOCK_DIR}.reaper"' EXIT
  old_pid="$(cat "${LOCK_DIR}/pid" 2>/dev/null | tr -cd '0-9')"
  if [ -z "${old_pid}" ] || ! kill -0 "${old_pid}" 2>/dev/null; then
    rm -rf "${LOCK_DIR}"
  fi
  if ! mkdir "${LOCK_DIR}" 2>/dev/null; then
    console_log "trigger: a previous run reclaimed the lock mid-reap; skipping tick"
    exit 0
  fi
fi
echo $$ > "${LOCK_DIR}/pid"
trap 'rm -rf "${LOCK_DIR}" "${LOCK_DIR}.reaper"' EXIT

# Read the whole ready queue before choosing: `--paginate` follows the
# Link header to exhaustion, so the lowest number comes from every
# labeled ticket instead of the first ten results. The issues endpoint
# also returns pull requests; `select(.pull_request == null)` drops
# them, matching what `gh issue list` returned.
ticket="$(gh api --paginate \
  "repos/{owner}/{repo}/issues?labels=ready-for-agent&state=open&per_page=100" \
  -q '.[] | select(.pull_request == null) | .number' 2>/dev/null | sort -n | head -1)"
if [ -z "${ticket}" ]; then
  echo "$(date -u '+%FT%TZ') trigger: no issues labeled ready-for-agent, idle exit"
  exit 0
fi
stamp="$(date -u '+%Y%m%dT%H%M%SZ')"
run_log="${RUN_DIR}/issue-${ticket}-${stamp}.log"

# Claim first, launch second: after this line a crash can never lead to
# a second run on the same ticket, because the label is gone. The
# failure path below re-labels, so the ticket is the source of truth.
gh issue edit "${ticket}" --remove-label ready-for-agent --add-label agent-wip || exit 1

echo "$(date -u '+%FT%TZ') trigger: claiming issue #${ticket}, transcript ${run_log}"

session_output="$(opencode run --auto --title "tar-${ticket}-agent-run" \
  "You are the autonomous backlog pickup for this repo. Ticket: GitHub issue ${ticket}. Work autonomously end to end under the repo's rules; the human only decides whether to merge.

Contract, in order:
1. Read the whole ticket first: gh issue view ${ticket}. Then read AGENTS.md and DEVELOPMENT_RULES.md and follow both exactly (validation matrix, file caps, commit style, review gates).
2. git fetch origin. Branch feat/agent-${ticket} off origin/master. Never commit to master; never force-push.
3. Implement ONLY what the ticket names. It is a deliberately small ticket: stop as soon as the acceptance criteria pass and every touched helper is covered by the coverage-table rows that now apply.
4. Run the full validation matrix for the areas you touched (fmt, clippy -D warnings, tests on both crates when Rust files changed, npm run test:e2e when ui changed, plus the manual rows the ticket names that do not need a human). Put the resulting counts in the PR body next to each command, per AGENTS.md.
5. Open the PR with gh pr create, title conventional-commit, body with the validation matrix, deviations argued (code wins), and \`Closes #${ticket}\`. Never merge: the human decides. Never close the issue directly.
6. Qodo review loop on the open PR, until clean: after opening it, poll the PR's review comments (\`gh pr view HEAD --json comments\`; reviewer bot account: qodo-code-review) and treat a still-unresolved finding as work to do. For each one: triage against the code — apply the confirmed ones as new commits (fmt, clippy, tests before every push), and dismiss the false ones with the reason in the PR body. Pushing triggers a fresh automated review; wait ~90 seconds after each push before re-reading comments. The loop's goal is zero unresolved findings, not a single pass.
7. Cap: 3 review rounds maximum. A round that only repeats already-fixed findings does not count as new work; document the dispositions in the PR body. If a confirmed finding is still unresolved at the cap: comment on the ticket with the finding and where it stops, add the agent-stuck label, and exit nonzero.
8. If the ticket turns out stale (criteria already shipped, PRs already filed, docker container unavailable, tooling missing), say so explicitly in a ticket comment and exit nonzero instead of inventing scope.
9. Leave the working tree clean (no untracked files left behind).")"
status=$?
# Transcript lands in its named file; the stdout this script already
# prints to is the shared trigger.log, so mirror it for two files, one
# look.
printf '%s\n' "${session_output}" > "${run_log}"
printf '%s\n' "${session_output}"

echo "$(date -u '+%FT%TZ') trigger: session for #${ticket} exited ${status}"

if [ "${status}" -ne 0 ]; then
  gh issue edit "${ticket}" --remove-label agent-wip --add-label agent-stuck >/dev/null 2>&1
  gh issue comment "${ticket}" \
    --body "Agent run stopped (exit ${status}, transcript: \`output/agent-runs/issue-${ticket}-${stamp}.log\`). The ticket waits for a human: diagnose with the transcript, then re-apply \`ready-for-agent\` to retry, or finish the work yourself." >/dev/null 2>&1
  echo "$(date -u '+%FT%TZ') trigger: #${ticket} marked agent-stuck, human needed"
fi

exit 0
