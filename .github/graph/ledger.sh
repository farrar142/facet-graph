#!/usr/bin/env bash
# Cost ledger of the autonomous graph trial (budget: $GRAPH_BUDGET_USD, default 80, API list price).
# The ledger is the open-or-closed issue labelled `budget-ledger`; every Claude run adds one comment.
#
#   ledger.sh record <node> <issue>  read the action's execution file, comment cost and tokens,
#                                    and move the file to $RUNNER_TEMP/graph-runs for the artifact
#   ledger.sh total                  print the sum in USD
#   ledger.sh check                  exit 1 (and mark the ledger once) when the sum reached the budget
#
# Needs REPO and GH_TOKEN. Works in the Windows runner's Git Bash and on Ubuntu.
set -u
BUDGET=${GRAPH_BUDGET_USD:-80}
EXEC="${RUNNER_TEMP:-/tmp}/claude-execution-output.json"

ledger_issue() {
  gh issue list --repo "$REPO" --label budget-ledger --state all --limit 1 --json number --jq '.[0].number // empty'
}

total() {
  local n; n=$(ledger_issue)
  [ -n "$n" ] || { echo 0; return; }
  gh api "repos/$REPO/issues/$n/comments" --paginate \
    --jq '.[].body | select(startswith("<!-- ledger -->")) | capture("cost=(?<c>[0-9.]+)").c' \
    | awk '{s += $1} END {printf "%.4f\n", s + 0}'
}

case "${1:-}" in
  record)
    node=$2; issue=$3
    if [ ! -f "$EXEC" ]; then echo "no execution file for $node"; exit 0; fi
    line=$(jq -r '[(if type == "array" then .[] else . end) | select(.type == "result")] | last
      | if . == null then "cost=0 note=no-result" else
        "cost=\(.total_cost_usd // 0) turns=\(.num_turns // 0) input=\(.usage.input_tokens // 0) output=\(.usage.output_tokens // 0) cache_read=\(.usage.cache_read_input_tokens // 0) cache_write=\(.usage.cache_creation_input_tokens // 0)" end' "$EXEC")
    mkdir -p "${RUNNER_TEMP:-/tmp}/graph-runs"
    mv "$EXEC" "${RUNNER_TEMP:-/tmp}/graph-runs/${node}-${issue}-$(date +%s).json"
    n=$(ledger_issue)
    if [ -z "$n" ]; then echo "::warning::no budget-ledger issue; $node #$issue $line"; exit 0; fi
    gh issue comment "$n" --repo "$REPO" --body "<!-- ledger --> node=$node issue=#$issue run=${GITHUB_RUN_ID:-local} $line" >/dev/null
    echo "ledger: $node #$issue $line (total \$$(total))"
    ;;
  total)
    total
    ;;
  check)
    t=$(total)
    if awk -v t="$t" -v b="$BUDGET" 'BEGIN {exit !(t >= b)}'; then
      echo "::warning::budget reached: \$$t of \$$BUDGET; no further dispatch"
      n=$(ledger_issue)
      if [ -n "$n" ] && ! gh api "repos/$REPO/issues/$n/comments" --paginate --jq '.[].body' | grep -q '<!-- budget-exhausted -->'; then
        gh issue comment "$n" --repo "$REPO" --body "<!-- budget-exhausted --> Budget reached: \$$t of \$$BUDGET. Dispatching stopped." >/dev/null
      fi
      exit 1
    fi
    echo "budget: \$$t of \$$BUDGET"
    ;;
  *)
    echo "usage: ledger.sh record <node> <issue> | total | check" >&2; exit 2
    ;;
esac
