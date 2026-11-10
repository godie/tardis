#!/bin/zsh
# Mac-side dispatcher for the autonomous backlog pickup.
#
# Keeps the local launchd tick cheap: instead of running the agent on
# the Mac, it wakes the instabox box (a paused one resumes on the next
# exec) and launches the in-box copy of scripts/agent-backlog-pickup.sh
# fire-and-forget. All state lives on the box: its labels, its lock, its
# logs — the Mac tick neither claims tickets nor holds a lock.
#
# Dispatch failures are logged to the same trigger.log the in-box runs
# use, so "what happened while I was away" stays in one file.

set -u

BOX="${AGENT_LOW_BACKLOG_BOX:-agent-pickup}"
REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
LOG_DIR="${REPO_DIR}/output/agent-runs"
cd "${REPO_DIR}" || exit 1
mkdir -p "${LOG_DIR}"
LOG="${LOG_DIR}/trigger.log"

echo "$(date -u '+%FT%TZ') dispatcher: handing a tick to box ${BOX}"
if ! result="$(instabox ssh "${BOX}" -- 'sh -c "nohup /data/home/tardis/scripts/agent-backlog-pickup.sh >> /data/home/tardis/output/agent-runs/trigger.log 2>&1 & echo dispatch pid \$!"' 2>&1)"; then
  echo "$(date -u '+%FT%TZ') dispatcher: ssh to ${BOX} failed: ${result}" | tee -a "${LOG}"
  exit 1
fi
echo "${result}"
