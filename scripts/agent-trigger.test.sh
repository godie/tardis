#!/bin/bash
# Conformance harness for the agent trigger scripts, in the same spirit
# as scripts/install-skills.test.sh. Everything runs against a temp
# tree with PATH shims for `gh`, `opencode` and `launchctl`; the real
# `plutil` lints the generated XML. Covers the delegate-review fixes:
#   finding 2 — installer must make zero pickup calls
#   finding 3 — stale-lock recovery is atomic (parallel ticks)
#   finding 4 — board items from other repos are never touched
#   finding 6 — paths with spaces / `&` survive the installer

set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
PASS=0
FAIL=0

ok() { echo "✓ $1"; PASS=$((PASS + 1)); }
bad() { echo "✗ $1"; FAIL=$((FAIL + 1)); }

build_shims() { # $1 = fake repo root
  local root="$1"
  SHIM_DIR="${root}/.shims"
  MARKER="${SHIM_DIR}/opencode-markers.txt"
  SHIM_LOG="${SHIM_DIR}/invocations.txt"
  export SHIM_LOG MARKER
  mkdir -p "${SHIM_DIR}"
  : > "${SHIM_LOG}"; : > "${MARKER}"

  # Quoted heredocs; the log/marker paths arrive from the env so the
  # shim bodies never interpolate harness variables.
  cat > "${SHIM_DIR}/gh" <<'GH'
#!/bin/bash
echo "gh: $*" >> "${SHIM_LOG}"
case "$1 $2" in
  "repo view")
    echo '{"nameWithOwner":"godie/tardis"}'
    ;;
  "issue list")
    echo '10'
    ;;
  "issue view")
    echo '[]'
    ;;
  "api graphql")
    cat <<'RAW'
{"data":{"user":{"projectV2":{"items":{"nodes":[
 {"content":{"number":10,"state":"OPEN","repository":{"nameWithOwner":"godie/tardis"}},
  "fieldValues":{"nodes":[{"name":"Ready for agent","field":{"name":"Status"}}]}},
 {"content":{"number":9,"state":"OPEN","repository":{"nameWithOwner":"other/repo"}},
  "fieldValues":{"nodes":[{"name":"Ready for agent","field":{"name":"Status"}}]}}
]}}}}}
RAW
    ;;
esac
GH
  chmod +x "${SHIM_DIR}/gh"

  cat > "${SHIM_DIR}/opencode" <<'OC'
#!/bin/bash
echo "opencode: $*" >> "${SHIM_LOG}"
echo "$$" >> "${MARKER}"
OC
  chmod +x "${SHIM_DIR}/opencode"

  cat > "${SHIM_DIR}/launchctl" <<'LC'
#!/bin/bash
echo "launchctl: $*" >> "${SHIM_LOG}"
exit 0
LC
  chmod +x "${SHIM_DIR}/launchctl"
}

count_calls() {
  # grep -c already prints 0 on empty input; the || echo 0 fallback only
  # fires when the file itself is missing (|| fires on grep's exit 1
  # from a zero count, which would double-print and fail comparisons).
  if [ -f "${SHIM_LOG}" ]; then
    awk 'BEGIN { c = 0 } { if (index($0, prefix ":") == 1) c++ } END { print c }' prefix="$1" "${SHIM_LOG}"
  else
    echo 0
  fi
}

# --- test 1 (finding 2): pickup --help makes zero gh/opencode calls ---

T1_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/t1.XXXXXX")
mkdir -p "${T1_ROOT}/scripts"
cp "${HERE}/agent-backlog-pickup.sh" "${T1_ROOT}/scripts/"
build_shims "${T1_ROOT}"

if (cd "${T1_ROOT}" && env -i PATH="${SHIM_DIR}:/usr/bin:/bin" bash scripts/agent-backlog-pickup.sh --help); then
  if [ "$(count_calls gh)" = "0" ] && [ "$(count_calls opencode)" = "0" ]; then
    ok "--help exits before any gh/opencode call"
  else
    echo "(t1 shim log: $(cat "${SHIM_LOG}" 2>/dev/null))"
    bad "--help must not call gh/opencode"
  fi
else
  bad "--help exited nonzero"
fi

# --- test 2 (finding 4): board items from other repos are never touched ---

T3_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/t3.XXXXXX")
mkdir -p "${T3_ROOT}/scripts"
cp "${HERE}/agent-backlog-pickup.sh" "${T3_ROOT}/scripts/"
build_shims "${T3_ROOT}"
export TARDIS_REPO_FULL="godie/tardis"

run_out=$( (cd "${T3_ROOT}" && env PATH="${SHIM_DIR}:/usr/bin:/bin" TARDIS_REPO_FULL="godie/tardis" bash scripts/agent-backlog-pickup.sh) 2>&1 )
if grep -q "issue #10 earned ready-for-agent" <<<"${run_out}" && ! grep -q "issue #9 earned" <<<"${run_out}"; then
  ok "board sync touches only godie/tardis items (#10 claimed, other/repo#9 skipped)"
else
  bad "board sync repo scoping failed: ${run_out}"
fi

# --- test 3 (finding 3): parallel ticks on a stale lock run one agent ---

T2_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/t2.XXXXXX")
mkdir -p "${T2_ROOT}/scripts" "${T2_ROOT}/output/agent-runs"
cp "${HERE}/agent-backlog-pickup.sh" "${T2_ROOT}/scripts/"
build_shims "${T2_ROOT}"
export TARDIS_REPO_FULL="godie/tardis"
# A stale lock: its pid points at a process that is definitely dead.
echo "9999999" > "${T2_ROOT}/output/agent-runs/lock"

(cd "${T2_ROOT}" && PATH="${SHIM_DIR}:$PATH" TARDIS_REPO_FULL="godie/tardis" bash scripts/agent-backlog-pickup.sh >/dev/null 2>&1) &
p1=$!
(cd "${T2_ROOT}" && PATH="${SHIM_DIR}:$PATH" TARDIS_REPO_FULL="godie/tardis" bash scripts/agent-backlog-pickup.sh >/dev/null 2>&1) &
p2=$!
wait "${p1}" "${p2}"

agents=$(grep -c . "${MARKER}" 2>/dev/null || echo 0)
if [ "${agents}" = "1" ]; then
  ok "parallel ticks on a stale lock: exactly one agent ran"
else
  bad "expected exactly one agent run on a stale lock, got ${agents}"
fi
if [ ! -e "${T2_ROOT}/output/agent-runs/lock" ]; then
  ok "lock released after the run"
else
  bad "lock left behind after the run"
fi
if [ ! -d "${T2_ROOT}/output/agent-runs/lock.reaper" ]; then
  ok "reaper released after stale recovery"
else
  bad "reaper left behind after stale recovery"
fi

# A missing / empty pid also counts as stale (the pid=0 bug).
T2B_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/t2b.XXXXXX")
mkdir -p "${T2B_ROOT}/scripts" "${T2B_ROOT}/output/agent-runs"
cp "${HERE}/agent-backlog-pickup.sh" "${T2B_ROOT}/scripts/"
build_shims "${T2B_ROOT}"
export TARDIS_REPO_FULL="godie/tardis"
: > "${T2B_ROOT}/output/agent-runs/lock"

run_out=$( (cd "${T2B_ROOT}" && PATH="${SHIM_DIR}:$PATH" bash scripts/agent-backlog-pickup.sh) 2>&1 )
if grep -q "trigger: claiming issue #10" <<<"${run_out}"; then
  ok "empty pid file is treated as stale and reaped"
else
  bad "empty pid file blocked recovery: ${run_out}"
fi

# A live pid in an existing lock means another session owns the run, and
# the tick must not reap it.
T2C_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/t2c.XXXXXX")
mkdir -p "${T2C_ROOT}/scripts" "${T2C_ROOT}/output/agent-runs"
cp "${HERE}/agent-backlog-pickup.sh" "${T2C_ROOT}/scripts/"
build_shims "${T2C_ROOT}"
export TARDIS_REPO_FULL="godie/tardis"
echo "$$" > "${T2C_ROOT}/output/agent-runs/lock"

run_out=$( (cd "${T2C_ROOT}" && PATH="${SHIM_DIR}:$PATH" bash scripts/agent-backlog-pickup.sh) 2>&1 )
if grep -q "a previous run is active" <<<"${run_out}" \
  && ! grep -q "trigger: claiming issue" <<<"${run_out}" \
  && [ -e "${T2C_ROOT}/output/agent-runs/lock" ]; then
  ok "a live pid blocks the tick and its lock is left alone"
else
  bad "live lock was reaped or claimed: ${run_out}"
fi

# A lock directory from the older script is recovered too: its pid lives
# in `lock/pid`, and `ln` must not mistake the directory for a claim.
T2F_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/t2f.XXXXXX")
mkdir -p "${T2F_ROOT}/scripts" "${T2F_ROOT}/output/agent-runs/lock"
cp "${HERE}/agent-backlog-pickup.sh" "${T2F_ROOT}/scripts/"
build_shims "${T2F_ROOT}"
export TARDIS_REPO_FULL="godie/tardis"
echo "9999999" > "${T2F_ROOT}/output/agent-runs/lock/pid"

run_out=$( (cd "${T2F_ROOT}" && PATH="${SHIM_DIR}:$PATH" bash scripts/agent-backlog-pickup.sh) 2>&1 )
if grep -q "trigger: claiming issue #10" <<<"${run_out}" \
  && [ ! -e "${T2F_ROOT}/output/agent-runs/lock" ] \
  && [ "$(grep -c . "${MARKER}" 2>/dev/null || echo 0)" = "1" ]; then
  ok "a legacy lock directory with a dead pid is recovered"
else
  bad "legacy lock directory blocked or doubled the run: ${run_out}"
fi

# The same directory with a live pid still blocks the tick.
T2G_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/t2g.XXXXXX")
mkdir -p "${T2G_ROOT}/scripts" "${T2G_ROOT}/output/agent-runs/lock"
cp "${HERE}/agent-backlog-pickup.sh" "${T2G_ROOT}/scripts/"
build_shims "${T2G_ROOT}"
export TARDIS_REPO_FULL="godie/tardis"
echo "$$" > "${T2G_ROOT}/output/agent-runs/lock/pid"

run_out=$( (cd "${T2G_ROOT}" && PATH="${SHIM_DIR}:$PATH" bash scripts/agent-backlog-pickup.sh) 2>&1 )
if grep -q "a previous run is active" <<<"${run_out}" \
  && [ -d "${T2G_ROOT}/output/agent-runs/lock" ]; then
  ok "a legacy lock directory with a live pid is left alone"
else
  bad "legacy live lock was reaped or claimed: ${run_out}"
fi

# An orphaned reaper, left when a tick is killed during stale recovery,
# must not stop an ordinary start: with no lock the fast path never looks
# at it.
T2D_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/t2d.XXXXXX")
mkdir -p "${T2D_ROOT}/scripts" "${T2D_ROOT}/output/agent-runs/lock.reaper"
cp "${HERE}/agent-backlog-pickup.sh" "${T2D_ROOT}/scripts/"
build_shims "${T2D_ROOT}"
export TARDIS_REPO_FULL="godie/tardis"

run_out=$( (cd "${T2D_ROOT}" && PATH="${SHIM_DIR}:$PATH" bash scripts/agent-backlog-pickup.sh) 2>&1 )
if grep -q "trigger: claiming issue #10" <<<"${run_out}" \
  && [ "$(grep -c . "${MARKER}" 2>/dev/null || echo 0)" = "1" ] \
  && [ -d "${T2D_ROOT}/output/agent-runs/lock.reaper" ]; then
  ok "an orphaned reaper does not block an ordinary pickup and is left alone"
else
  bad "orphaned reaper blocked a pickup or was removed: ${run_out}"
fi

# --- test 3b (finding 3): a published lock always carries its pid ---

# Hold the winning tick inside `ln` after the lock is published, start a
# second tick against that published lock, and release the first only
# once the second has finished. The pid is already in the file, so the
# second tick must find a live owner; a lock that appears before its pid
# is written lets the second tick reap it and run beside the first.
T2E_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/t2e.XXXXXX")
mkdir -p "${T2E_ROOT}/scripts" "${T2E_ROOT}/output/agent-runs"
cp "${HERE}/agent-backlog-pickup.sh" "${T2E_ROOT}/scripts/"
build_shims "${T2E_ROOT}"
export TARDIS_REPO_FULL="godie/tardis"
SIGNAL="${T2E_ROOT}/published"
RELEASE="${T2E_ROOT}/release"
cat > "${SHIM_DIR}/ln" <<'LN'
#!/bin/bash
/bin/ln "$@"
status=$?
case "$*" in
  */agent-runs/lock)
    if [ "${status}" = "0" ] && [ -n "${PICKUP_TEST_LN_SIGNAL:-}" ]; then
      : > "${PICKUP_TEST_LN_SIGNAL}"
      i=0
      while [ ! -e "${PICKUP_TEST_LN_RELEASE}" ] && [ "${i}" -lt 300 ]; do
        sleep 0.01
        i=$((i + 1))
      done
    fi
    ;;
esac
exit "${status}"
LN
chmod +x "${SHIM_DIR}/ln"

( cd "${T2E_ROOT}" && PATH="${SHIM_DIR}:$PATH" TARDIS_REPO_FULL="godie/tardis" \
    PICKUP_TEST_LN_SIGNAL="${SIGNAL}" PICKUP_TEST_LN_RELEASE="${RELEASE}" \
    bash scripts/agent-backlog-pickup.sh >/dev/null 2>&1 ) &
p1=$!
i=0
while [ ! -e "${SIGNAL}" ] && [ "${i}" -lt 200 ]; do
  sleep 0.01
  i=$((i + 1))
done
if [ -e "${SIGNAL}" ]; then
  published_pid="$(tr -cd '0-9' < "${T2E_ROOT}/output/agent-runs/lock" 2>/dev/null)"
  if [ -n "${published_pid}" ]; then
    ok "the published lock already carries its pid"
  else
    bad "the lock was published without a pid"
  fi
  ( cd "${T2E_ROOT}" && PATH="${SHIM_DIR}:$PATH" TARDIS_REPO_FULL="godie/tardis" \
      bash scripts/agent-backlog-pickup.sh >"${T2E_ROOT}/tick2.log" 2>&1 ) &
  p2=$!
  # Let the second tick reach its decision, then release the first so
  # both processes end instead of one waiting out the shim cap.
  i=0
  while kill -0 "${p2}" 2>/dev/null && [ "${i}" -lt 300 ]; do
    sleep 0.01
    i=$((i + 1))
  done
  : > "${RELEASE}"
  wait "${p1}" "${p2}" 2>/dev/null
  agents=$(grep -c . "${MARKER}" 2>/dev/null || echo 0)
  if [ "${agents}" = "1" ] && grep -q "a previous run is active" "${T2E_ROOT}/tick2.log"; then
    ok "ticks racing the initial lock creation: exactly one agent ran"
  else
    bad "expected one agent against a published lock, got ${agents}: $(cat "${T2E_ROOT}/tick2.log" 2>/dev/null)"
  fi
else
  : > "${RELEASE}"
  wait "${p1}" 2>/dev/null
  bad "the first tick never published its lock"
fi

# --- test 4 (finding 6): installer survives spaces & ampersands ---

T4_HOME=$(mktemp -d "${TMPDIR:-/tmp}/t4home.XXXXXX")
T4_ROOT="${T4_HOME}/Work & <Projects>/tardis"
mkdir -p "${T4_ROOT}/scripts" "${T4_ROOT}/output/agent-runs" \
         "${T4_HOME}/Library/LaunchAgents"
cp "${HERE}/install-agent-trigger.sh" "${HERE}/agent-pickup-remote.sh" "${HERE}/agent-backlog-pickup.sh" "${T4_ROOT}/scripts/"
build_shims "${T4_ROOT}"

if out=$(
  cd "${T4_ROOT}" && env -i \
    HOME="${T4_HOME}" \
    PATH="${SHIM_DIR}:/usr/bin:/bin" \
    BOX_NAME=agent-pickup \
    AGENT_TRIGGER_SKIP_REACHABILITY=1 \
    bash "${T4_ROOT}/scripts/install-agent-trigger.sh" 2>&1
); then
  plist="${T4_HOME}/Library/LaunchAgents/dev.tardis.agent-pickup.plist"
  if [ -f "${plist}" ] && /usr/bin/plutil -lint "${plist}" >/dev/null 2>&1; then
    ok "installer with 'Work & <Projects>' produced a valid plist"
  else
    bad "plist missing or invalid for a spaced path: ${out}"
  fi
  # The installer must never have launched the pickup (finding 2).
  if grep -q "^gh:" "${SHIM_LOG}" || grep -q "^opencode:" "${SHIM_LOG}"; then
    bad "installer made pickup calls (gh/opencode shims tapped)"
  else
    ok "installer run made zero pickup calls"
  fi
else
  bad "installer failed on a spaced path: ${out}"
fi

echo
echo "passed: ${PASS}, failed: ${FAIL}"
[ "${FAIL}" = "0" ]
