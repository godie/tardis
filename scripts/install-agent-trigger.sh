#!/bin/zsh
# Install (or remove) the launchd trigger that hands a tick to the
# instabox agent box every 30 minutes while this Mac is awake.
#
#   scripts/install-agent-trigger.sh                  install (instabox box)
#   scripts/install-agent-trigger.sh remove           remove
#
# The box-side body is scripts/agent-backlog-pickup.sh; on the Mac only
# scripts/agent-pickup-remote.sh runs (an ssh wake + fire-and-forget).
# RunAtLoad is deliberately off so installing never fires a surprise
# run: add a ready-for-agent label, or `launchctl kickstart`, when you
# want one.

set -eu

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
LABEL="dev.tardis.agent-pickup"
PLIST="${HOME}/Library/LaunchAgents/${LABEL}.plist"
RUNNER="${REPO_DIR}/scripts/agent-pickup-remote.sh"

if [ "${1:-install}" = "remove" ]; then
  launchctl bootout "gui/$(id -u)/${LABEL}" 2>/dev/null || true
  rm -f "${PLIST}"
  echo "removed ${PLIST}"
  exit 0
fi

[ -x "${RUNNER}" ] || { echo "missing runner: ${RUNNER}" >&2; exit 1; }
if [ "${AGENT_TRIGGER_SKIP_REACHABILITY:-0}" != "1" ]; then
  instabox get "${BOX_NAME:-agent-pickup}" >/dev/null 2>&1 || {
    echo "instabox box '${BOX_NAME:-agent-pickup}' not reachable — check instabox whoami / name" >&2
    exit 1
  }
fi

mkdir -p "${REPO_DIR}/output/agent-runs"

launchctl bootout "gui/$(id -u)/${LABEL}" 2>/dev/null || true

# Paths go into XML <string> values and launchd argv items directly:
# no shell string is built, so spaces survive, and every placeholder is
# XML-escaped so a path with `&` cannot corrupt the plist (delegate
# finding 6).
xml_escape() { sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g'; }
ESCAPED_RUNNER="$(printf '%s' "${RUNNER}" | xml_escape)"
ESCAPED_LOG="$(printf '%s' "${REPO_DIR}/output/agent-runs/trigger.log" | xml_escape)"
ESCAPED_REPO="$(printf '%s' "${REPO_DIR}" | xml_escape)"

cat > "${PLIST}" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>${LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>$(printf '%s' "${RUNNER}" | xml_escape)</string>
  </array>
  <!-- the runner's shebang selects bash; launchd exec()s it directly -->
  <key>StandardOutPath</key>
  <string>${ESCAPED_LOG}</string>
  <key>StandardErrorPath</key>
  <string>${ESCAPED_LOG}</string>
  <key>StartInterval</key>
  <integer>1800</integer>
  <key>RunAtLoad</key>
  <false/>
  <key>EnvironmentVariables</key>
  <dict>
    <key>REPO_DIR</key>
    <string>${ESCAPED_REPO}</string>
  </dict>
</dict>
</plist>
EOF

if command -v plutil >/dev/null 2>&1; then
  plutil -lint "${PLIST}" >/dev/null || { echo "generated plist is not valid XML — refusing to install" >&2; exit 1; }
fi

launchctl bootstrap "gui/$(id -u)" "${PLIST}"
launchctl print "gui/$(id -u)/${LABEL}" >/dev/null && echo "installed: ${PLIST} (remote executor ${BOX_NAME:-agent-pickup}, every 1800 s)"
echo "fire a run now:  launchctl kickstart gui/$(id -u)/${LABEL}"
echo "temporary off:   launchctl bootout gui/$(id -u)/${LABEL}"
echo "resume:          launchctl bootstrap gui/$(id -u) ${PLIST}"
