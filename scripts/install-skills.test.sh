#!/bin/sh
# Tests for scripts/install-skills.sh.
#
# Run directly (sh scripts/install-skills.test.sh) or from CI. Exits non-zero
# on the first failure. Every case works in a throwaway directory, so the real
# skills are never touched.
set -eu

repo_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
installer="$repo_dir/scripts/install-skills.sh"
source_dir="$repo_dir/.agents/skills"
skills="no-ai-slop caveman-review ponytail-review"

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# 1. Initial install links every skill.
"$installer" --target "$tmp/link" >/dev/null
for name in $skills; do
    [ -f "$tmp/link/$name/SKILL.md" ] || fail "initial install missed $name"
    [ -L "$tmp/link/$name" ] || fail "initial install should link, not copy, $name"
done

# 2. A repeat run is a no-op: everything is skipped, nothing is replaced.
output=$("$installer" --target "$tmp/link")
echo "$output" | grep -q "3 skipped" || fail "repeat run should skip all three: $output"
echo "$output" | grep -q "0 installed" || fail "repeat run should install none: $output"

# 3. Copy mode copies real directories.
"$installer" --target "$tmp/copy" --copy >/dev/null
for name in $skills; do
    [ -f "$tmp/copy/$name/SKILL.md" ] || fail "copy mode missed $name"
    if [ -L "$tmp/copy/$name" ]; then
        fail "copy mode must not link $name"
    fi
done

# 4. --force replaces an existing entry.
rm -rf "$tmp/copy/$skills"
mkdir -p "$tmp/copy/caveman-review"
echo stale >"$tmp/copy/caveman-review/stale"
"$installer" --target "$tmp/copy" --copy --force >/dev/null
[ -f "$tmp/copy/caveman-review/SKILL.md" ] || fail "--force did not replace the stale entry"
if [ -f "$tmp/copy/caveman-review/stale" ]; then
    fail "--force left the stale file behind"
fi

# 5. The source directory is refused as a target, even with --force.
if "$installer" --target "$source_dir" --force >/dev/null 2>&1; then
    fail "installing into the source directory must be refused"
fi
[ -f "$source_dir/no-ai-slop/SKILL.md" ] || fail "source skills were deleted"

# 6. A symlink alias to the source directory is refused too.
ln -s "$source_dir" "$tmp/alias"
if "$installer" --target "$tmp/alias" --force >/dev/null 2>&1; then
    fail "a symlink alias to the source directory must be refused"
fi
[ -f "$source_dir/no-ai-slop/SKILL.md" ] || fail "source skills were deleted through the alias"

echo "ok: install-skills"
