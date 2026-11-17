#!/usr/bin/env bash
# Mechanical format check of the ledger (specs/README.md). Odin judges the logic; this only
# catches what a script can: missing front matter, sections, headers, and uncovered ACs.
# Prints every problem and exits 1 if there is any.
set -u
bad=0
problem() { echo "ledger: $*"; bad=1; }

for f in specs/requirements/R-*.md; do
  [ -f "$f" ] || continue
  for k in id source source_hash status; do grep -q "^$k:" "$f" || problem "$f: front matter '$k' missing"; done
  grep -q '^source_hash: *PENDING' "$f" && problem "$f: source_hash not filled"
  grep -q '^## 요구 항목' "$f" || problem "$f: '## 요구 항목' missing"
done

for spec in specs/features/*/spec.md; do
  [ -f "$spec" ] || continue
  dir=$(dirname "$spec")
  for k in feature requirement requirement_version; do grep -q "^$k:" "$spec" || problem "$spec: front matter '$k' missing"; done
  for s in "Problem Statement" "Solution" "User Stories" "Contract" "Acceptance Criteria" "E2E Tests" "Manual Checks" "Decided by the node" "Out of Scope"; do
    grep -q "^## $s" "$spec" || problem "$spec: section '## $s' missing"
  done
  req=$(sed -n 's/^requirement: *//p' "$spec" | head -1)
  [ -f "specs/requirements/$req.md" ] || problem "$spec: requirement $req not in the catalog"
  acs=$(grep -o '^- AC-[0-9]*' "$spec" | sed 's/^- //' | sort -u)
  [ -n "$acs" ] || problem "$spec: no '- AC-n:' lines"
  covered=""
  for t in "$dir"/acceptance/*.rs; do
    [ -f "$t" ] || continue
    l1=$(sed -n 1p "$t" | tr -d '\r'); l2=$(sed -n 2p "$t" | tr -d '\r')
    case "$l1" in "// target: crates/"*"/tests/"*.rs) ;; *) problem "$t: first line must be '// target: crates/<crate>/tests/<file>.rs'";; esac
    case "$l2" in "// AC: "*) covered="$covered $(echo "${l2#// AC: }" | tr ',' ' ')";; *) problem "$t: second line must be '// AC: ...'";; esac
    for a in $(echo "${l2#// AC: }" | tr ',' ' '); do echo "$acs" | grep -qx "$a" || problem "$t: $a is not an AC of the spec"; done
  done
  manual=$(sed -n '/^## Manual Checks/,/^## /p' "$spec" | grep -o 'AC-[0-9]*' | sort -u)
  for a in $acs; do
    echo " $covered " | grep -q " $a " && continue
    echo "$manual" | grep -qx "$a" && continue
    problem "$spec: $a is in no E2E test header and no manual check"
  done
done

exit $bad
