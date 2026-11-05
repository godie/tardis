#!/bin/sh
# Install this repo's agent skills into a skills directory.
#
# The copies in .agents/skills/ are read directly by agents that support
# project-level skills (OpenCode, and anything following the .agents
# convention). Agents that only read a global directory need this script.
#
# Usage:
#   scripts/install-skills.sh                        # link into ~/.agents/skills
#   scripts/install-skills.sh --target ~/.claude/skills
#   scripts/install-skills.sh --target ~/.config/opencode/skills
#   scripts/install-skills.sh --copy                 # copy instead of link
#   scripts/install-skills.sh --force                # replace an existing entry
#
# Linking (the default) keeps one copy: editing the skill in the repo changes
# what your agent loads. Copying is for machines where the repo will move.
set -eu

repo_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
source_dir="$repo_dir/.agents/skills"
target_dir="${AGENT_SKILLS_DIR:-$HOME/.agents/skills}"
mode=link
force=0

usage() {
    sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'
}

while [ $# -gt 0 ]; do
    case "$1" in
        --target)
            [ $# -ge 2 ] || { echo "--target needs a directory" >&2; exit 2; }
            target_dir=$2
            shift 2
            ;;
        --target=*)
            target_dir=${1#--target=}
            shift
            ;;
        --copy)
            mode=copy
            shift
            ;;
        --force)
            force=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

[ -d "$source_dir" ] || { echo "no skills at $source_dir" >&2; exit 1; }

# Physical path of a directory, symlinks resolved.
resolve_dir() {
    (CDPATH= cd -- "$1" && pwd -P)
}

mkdir -p "$target_dir"
source_path=$(resolve_dir "$source_dir")
target_path=$(resolve_dir "$target_dir")

# A forced install into the source directory would delete each skill just
# before copying or linking it. Refuse early, before any rm. Comparing
# resolved paths catches a symlink alias too.
if [ "$target_path" = "$source_path" ]; then
    echo "refusing to install into the source directory: $target_path" >&2
    echo "pass a different target, for example --target \"$HOME/.agents/skills\"" >&2
    exit 1
fi

installed=0
skipped=0
for skill in "$source_dir"/*/; do
    [ -f "$skill/SKILL.md" ] || continue
    name=$(basename "$skill")
    dest="$target_dir/$name"

    if [ -e "$dest" ] || [ -L "$dest" ]; then
        if [ "$force" -eq 0 ]; then
            echo "skip $name: $dest already exists (use --force to replace)"
            skipped=$((skipped + 1))
            continue
        fi
        rm -rf "$dest"
    fi

    if [ "$mode" = copy ]; then
        cp -R "$skill" "$dest"
        echo "copied $name -> $dest"
    else
        ln -s "$skill" "$dest"
        echo "linked $name -> $dest"
    fi
    installed=$((installed + 1))
done

echo "$installed installed, $skipped skipped (target: $target_dir)"
