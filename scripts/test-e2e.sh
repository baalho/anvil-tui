#!/bin/sh
# E2E smoke tests for Anvil.
# Verifies basic CLI functionality without requiring an LLM backend.
set -e

# Resolve to absolute path so cd to temp dir doesn't break relative paths
if [ -n "$1" ]; then
    ANVIL="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
else
    ANVIL="$(cd "$(dirname "target/debug/anvil")" && pwd)/anvil"
fi
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

pass=0
fail=0

check() {
    desc="$1"
    shift
    if eval "$@" >/dev/null 2>&1; then
        echo "  ✓ $desc"
        pass=$((pass + 1))
    else
        echo "  ✗ $desc"
        fail=$((fail + 1))
    fi
}

check_output() {
    desc="$1"
    pattern="$2"
    shift 2
    if eval "$@" 2>&1 | grep -q "$pattern"; then
        echo "  ✓ $desc"
        pass=$((pass + 1))
    else
        echo "  ✗ $desc"
        fail=$((fail + 1))
    fi
}

echo "anvil e2e smoke tests"
echo "====================="

# Version
check_output "anvil --version prints version" "anvil" "$ANVIL --version"

# Help
check_output "anvil --help shows usage" "coding agent" "$ANVIL --help"

# Init
echo ""
echo "init tests (in $TMPDIR):"
check "anvil init creates .anvil/" "cd $TMPDIR && $ANVIL init"
check ".anvil/config.toml exists" "test -f $TMPDIR/.anvil/config.toml"
check ".anvil/context.md exists" "test -f $TMPDIR/.anvil/context.md"
check ".anvil/skills/ directory exists" "test -d $TMPDIR/.anvil/skills"

# Skills count
skill_count=$(find "$TMPDIR/.anvil/skills" -name "*.md" 2>/dev/null | wc -l | tr -d ' ')
if [ "$skill_count" -eq 21 ]; then
    echo "  ✓ 21 bundled skills installed"
    pass=$((pass + 1))
else
    echo "  ✗ expected 21 skills, found $skill_count"
    fail=$((fail + 1))
fi

# Model profiles
check ".anvil/models/ directory exists" "test -d $TMPDIR/.anvil/models"
profile_count=$(find "$TMPDIR/.anvil/models" -name "*.toml" 2>/dev/null | wc -l | tr -d ' ')
if [ "$profile_count" -ge 5 ]; then
    echo "  ✓ $profile_count model profiles installed"
    pass=$((pass + 1))
else
    echo "  ✗ expected ≥5 profiles, found $profile_count"
    fail=$((fail + 1))
fi

# Idempotent init
check "re-init is idempotent" "cd $TMPDIR && $ANVIL init"

echo ""
echo "results: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
