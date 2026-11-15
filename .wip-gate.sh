#!/bin/bash
# The pre-push gate's steps 1-5 (everything except the slice's #[ignore]d data tests) on a GitHub runner.
# Same checks as scratch/pre-push-gate.sh; injected into gate/<tag> commits on the fork by scratch/split-gate.ps1.
# Usage (in the checkout, injected files already removed): bash gh-gate.sh <base-sha> <known-upstream-file>
set -u
BASE=$1; KNOWN=$2
SUM=gate-summary.txt; : > "$SUM"; FAILS=0
mkdir -p gate-logs
say() { echo "$1" | tee -a "$SUM"; }
step() { if [ "$3" = 0 ]; then say "PASS     $1"; else say "FAIL     $1  (log: $2)"; FAILS=$((FAILS+1)); fi; }
say "github gate vs $(git rev-parse --short "$BASE") start $(date -u +%H:%M)Z"
CHANGED=$( { git diff --name-only "$BASE"; git ls-files --others --exclude-standard; } | sort -u)
UNTRACKED=$(git ls-files --others --exclude-standard)

L=gate-logs/diffcheck.log
{ git diff --check "$BASE"; for f in $UNTRACKED; do git diff --no-index --check /dev/null "$f"; done; } > "$L" 2>&1
[ -s "$L" ] && step "git diff --check" "$L" 1 || step "git diff --check" "$L" 0

L=gate-logs/fmt2024.log
FMT=$(echo "$CHANGED" | grep -E '^crates/(horizon-client/src/game|horizon-mod-api/src|horizon-mod-vocab/src)/.*\.rs$|^crates/horizon-mod-api-gen/dispatch-tests/src/fixtures/[^/]*\.rs$' || true)
EX=0
for f in $FMT; do [ -f "$f" ] && { rustup run stable rustfmt --edition 2024 --config skip_children=true --check "$f" >> "$L" 2>&1 || EX=1; }; done
[ -n "$FMT" ] && step "rustfmt --edition 2024 ($(echo "$FMT" | grep -c .) files)" "$L" $EX || say "SKIP     rustfmt --edition 2024 (no file in its scope)"

L=gate-logs/boundary.log; python3 tools/ci/check_api_boundary.py --source-only > "$L" 2>&1; step "check_api_boundary --source-only" "$L" $?
L=gate-logs/vocabstate.log; python3 tools/check_mod_vocabulary_state.py > "$L" 2>&1; step "check_mod_vocabulary_state" "$L" $?

L=gate-logs/prready.log; MD=gate-logs/prready.md
export HORIZON_HOME="$(mktemp -d)" HORIZON_LAUNCHER_HOME="$(mktemp -d)" XDG_DATA_HOME="$(mktemp -d)" XDG_CONFIG_HOME="$(mktemp -d)" HORIZON_LAUNCHER_NO_SELF_UPDATE=1 HORIZON_NO_AUDIO=1
echo "start $(date -u +%H:%M)" > "$L"
python3 tools/pr_ready.py --quick --verbose --base "$BASE" > "$MD" 2>> "$L"
echo "exit=$? end $(date -u +%H:%M)" >> "$L"
grep -E '^  [^ ].*: (pass|fail|skip|list|warn)( |$)' "$L" | while IFS= read -r line; do
  name=$(echo "$line" | sed -E 's/^  (.*): (pass|fail|skip|list|warn).*/\1/')
  st=$(echo "$line" | sed -E 's/^  .*: (pass|fail|skip|list|warn).*/\1/')
  case $st in
    pass) echo "PASS     pr_ready: $name" ;;
    skip|list) echo "SKIP     pr_ready: $name" ;;
    warn) echo "PASS     pr_ready: $name (warning, see log)" ;;
    fail)
      sect=$(awk -v n="  $name..." -v e="$line" '$0==n{on=1;next} $0==e{on=0} on' "$L")
      if [ "$name" = "Python tests" ]; then pat='^ *(FAIL|ERROR): [A-Za-z0-9_]+'
      else pat='^ *(FAIL|ERROR): [A-Za-z0-9_]+|^ *test [A-Za-z0-9_:]+ \.\.\. FAILED'; fi
      names=$(echo "$sect" | grep -oE "$pat" | sed -E 's/^ *(FAIL|ERROR): //; s/^ *test //; s/ \.\.\. FAILED//' | sort -u)
      unknown=""
      if [ -n "$names" ] && [ -f "$KNOWN" ]; then
        for t in $names; do grep -qxF "$t" <(grep -v '^#' "$KNOWN" | cut -d' ' -f1) || unknown="$unknown $t"; done
      else unknown="(no test names found)"; fi
      if [ -z "$unknown" ]; then echo "UPSTREAM pr_ready: $name (only known upstream failures:$(echo $names | tr '\n' ' '))"
      else echo "FAIL     pr_ready: $name  (new:$unknown)"; echo "$sect" | grep -E "$pat|error|panicked" | head -n 40 | sed 's/^/           /'; fi ;;
  esac
done | tee -a "$SUM"
FAILS=$((FAILS + $(grep -c '^FAIL     pr_ready' "$SUM")))
grep -q '^exit=' "$L" || { say "FAIL     pr_ready did not finish"; FAILS=$((FAILS+1)); }
grep -qE '^  [^ ].*: (pass|fail|skip|list|warn)( |$)' "$L" || { say "FAIL     pr_ready printed no steps"; tail -n 30 "$L" | tee -a "$SUM"; FAILS=$((FAILS+1)); }

# check_api_boundary (generated) runs in the aidan-51 half (pre-push-gate.sh, GATE_LOCAL_ONLY=1).

if [ "$FAILS" = 0 ]; then say "GITHUB PART PASS"; else say "GITHUB PART FAIL ($FAILS failing steps)"; fi
[ "$FAILS" = 0 ]
