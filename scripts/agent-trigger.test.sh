#!/bin/bash
# Conformance harness for the agent trigger scripts, in the same spirit
# as scripts/install-skills.test.sh. Everything runs against a temp
# tree with PATH shims for `gh`, `opencode` and `launchctl`; the real
# `plutil` lints the generated XML. Covers the delegate-review fixes:
#   finding 2 — installer must make zero pickup calls
#   finding 3 — stale-lock recovery is atomic (parallel ticks)
#   finding 4 — board items from other repos are never touched
#   finding 11 — the board and the label queue are read to exhaustion,
#                and the lowest ready number wins
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
  "issue view")
    echo '[]'
    ;;
  "api --paginate")
    # What gh prints after applying the script's --jq to one page.
    echo '10'
    ;;
  "api graphql")
    cat <<'RAW'
{"data":{"user":{"projectV2":{"items":{"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[
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

# --- test 2b (finding 11): the board sync reads every page ---

# Two pages, each holding a ready TARDIS item; the second page only
# appears through the first page's `pageInfo.endCursor`, so an item past
# the old 50-item window earns its label only if the cursor is followed.
T3B_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/t3b.XXXXXX")
mkdir -p "${T3B_ROOT}/scripts"
cp "${HERE}/agent-backlog-pickup.sh" "${T3B_ROOT}/scripts/"
build_shims "${T3B_ROOT}"
export TARDIS_REPO_FULL="godie/tardis"
cat > "${SHIM_DIR}/gh" <<'GH'
#!/bin/bash
echo "gh: $*" >> "${SHIM_LOG}"
case "$1 $2" in
  "repo view") echo '{"nameWithOwner":"godie/tardis"}' ;;
  "issue view") echo '[]' ;;
  "api --paginate") echo '[]' ;;
  "api graphql")
    case "$*" in
      *cursor=CURSOR1*)
        cat <<'RAW'
{"data":{"user":{"projectV2":{"items":{"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[
 {"content":{"number":12,"state":"OPEN","repository":{"nameWithOwner":"godie/tardis"}},
  "fieldValues":{"nodes":[{"name":"Ready for agent"}]}}
]}}}}}
RAW
        ;;
      *)
        cat <<'RAW'
{"data":{"user":{"projectV2":{"items":{"pageInfo":{"hasNextPage":true,"endCursor":"CURSOR1"},"nodes":[
 {"content":{"number":10,"state":"OPEN","repository":{"nameWithOwner":"godie/tardis"}},
  "fieldValues":{"nodes":[{"name":"Ready for agent"}]}},
 {"content":{"number":9,"state":"OPEN","repository":{"nameWithOwner":"other/repo"}},
  "fieldValues":{"nodes":[{"name":"Ready for agent"}]}}
]}}}}}
RAW
        ;;
    esac
    ;;
esac
GH
chmod +x "${SHIM_DIR}/gh"

run_out=$( (cd "${T3B_ROOT}" && PATH="${SHIM_DIR}:$PATH" bash scripts/agent-backlog-pickup.sh) 2>&1 )
if grep -q "issue #10 earned ready-for-agent" <<<"${run_out}" \
  && grep -q "issue #12 earned ready-for-agent" <<<"${run_out}" \
  && ! grep -q "issue #9 earned" <<<"${run_out}"; then
  ok "board sync follows the cursor (#10 and #12 claimed, other/repo#9 skipped)"
else
  bad "board sync missed the second page: ${run_out}"
fi

# --- test 2c (finding 11): the label queue is read to exhaustion ---

# One REST page stands in for the whole queue: `--paginate` follows the
# Link header itself, so the shim is called once. The numbers arrive
# unsorted and include a pull request, which is not a ticket. The tick
# must claim the lowest issue number in the queue, not the first one
# returned, and must not fall back to the abandoned ten-result query.
T3C_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/t3c.XXXXXX")
mkdir -p "${T3C_ROOT}/scripts"
cp "${HERE}/agent-backlog-pickup.sh" "${T3C_ROOT}/scripts/"
build_shims "${T3C_ROOT}"
export TARDIS_REPO_FULL="godie/tardis"
cat > "${SHIM_DIR}/gh" <<'GH'
#!/bin/bash
echo "gh: $*" >> "${SHIM_LOG}"
case "$1 $2" in
  "repo view") echo '{"nameWithOwner":"godie/tardis"}' ;;
  "api graphql") echo '{"data":{"user":{"projectV2":{"items":{"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[]}}}}}' ;;
  "api --paginate")
    # gh applies --jq to each page itself; the shim stands in for that
    # layer, so the script's filter still runs end to end.
    expr=""
    prev=""
    for arg in "$@"; do
      if [ "${prev}" = "-q" ] || [ "${prev}" = "--jq" ]; then expr="${arg}"; fi
      prev="${arg}"
    done
    echo '[{"number":50},{"number":30},{"number":3,"pull_request":{"url":"https://example.invalid/pr/3"}},{"number":12},{"number":7}]' \
      | jq -r "${expr}"
    ;;
esac
GH
chmod +x "${SHIM_DIR}/gh"

run_out=$( (cd "${T3C_ROOT}" && PATH="${SHIM_DIR}:$PATH" bash scripts/agent-backlog-pickup.sh) 2>&1 )
if grep -q "trigger: claiming issue #7" <<<"${run_out}"; then
  ok "the tick claims the lowest issue number across the queue"
else
  bad "the tick picked the wrong ticket: ${run_out}"
fi
if grep -Fq "api --paginate repos/{owner}/{repo}/issues?labels=ready-for-agent&state=open&per_page=100" "${SHIM_LOG}"; then
  ok "the queue is fetched with --paginate and a full page size"
else
  bad "the queue query is not paginated: $(grep -F 'api ' "${SHIM_LOG}" 2>/dev/null)"
fi

# --- test 3 (finding 3): parallel ticks on a stale lock run one agent ---

T2_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/t2.XXXXXX")
mkdir -p "${T2_ROOT}/scripts" "${T2_ROOT}/output/agent-runs"
cp "${HERE}/agent-backlog-pickup.sh" "${T2_ROOT}/scripts/"
build_shims "${T2_ROOT}"
export TARDIS_REPO_FULL="godie/tardis"
# A stale lock: its pid points at a process that is definitely dead.
mkdir -p "${T2_ROOT}/output/agent-runs/lock"
echo "9999999" > "${T2_ROOT}/output/agent-runs/lock/pid"

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
if [ ! -d "${T2_ROOT}/output/agent-runs/lock" ]; then
  ok "lock released after the run"
else
  bad "lock dir left behind after the run"
fi

# A missing / empty pid file also counts as stale (the pid=0 bug).
T2B_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/t2b.XXXXXX")
mkdir -p "${T2B_ROOT}/scripts" "${T2B_ROOT}/output/agent-runs/lock"
cp "${HERE}/agent-backlog-pickup.sh" "${T2B_ROOT}/scripts/"
build_shims "${T2B_ROOT}"
export TARDIS_REPO_FULL="godie/tardis"
: > "${T2B_ROOT}/output/agent-runs/lock/pid"

run_out=$( (cd "${T2B_ROOT}" && PATH="${SHIM_DIR}:$PATH" bash scripts/agent-backlog-pickup.sh) 2>&1 )
if grep -q "trigger: claiming issue #10" <<<"${run_out}"; then
  ok "empty pid file is treated as stale and reaped"
else
  bad "empty pid file blocked recovery: ${run_out}"
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
