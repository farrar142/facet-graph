#!/usr/bin/env bash
# Router of the autonomous graph (specs/README.md, "노드와 흐름").
#
# Reads only the ledger (`specs/` on main) and issue/PR labels, and dispatches every node that has
# work. It never decides what a node does, and dispatching twice is harmless: every node re-checks
# the ledger in its guard, and each node workflow keeps one run per subject (concurrency group).
# Runs on main's checkout. Needs REPO and GH_TOKEN.
set -uo pipefail
GRAPH_DIR=.github/graph
. "$GRAPH_DIR/lib.sh"
MAX_DEV=${GRAPH_MAX_DEV:-4}
STALE_MINUTES=${GRAPH_STALE_MINUTES:-15}

if ! bash "$GRAPH_DIR/ledger.sh" check; then exit 0; fi

ACTIVE=0 # nodes this pass found running or dispatched; the graph is not finished while any exist
dispatch() { # dispatch <workflow> <run title> <inputs...>
  local wf=$1 title=$2; shift 2
  ACTIVE=$((ACTIVE + 1))
  if running_titles "$wf" | grep -qxF "$title"; then echo "running: $title"; return; fi
  echo "dispatch: $title"
  gh workflow run "$wf" --repo "$REPO" "$@" >/dev/null
}

issues_json=$(gh issue list --repo "$REPO" --state all --limit 500 --json number,state,title,body,labels \
  --jq '[.[] | {number, state, title, body, labels: [.labels[].name]}]')
prs_json=$(gh pr list --repo "$REPO" --state open --limit 200 --json number,headRefName,headRefOid,labels \
  --jq '[.[] | {number, head: .headRefName, sha: .headRefOid, labels: [.labels[].name]}]')
open_with() { echo "$issues_json" | jq -r --arg l "$1" '.[] | select(.state == "OPEN" and (.labels | index($l)) and ((.labels | index("needs-decision")) | not)) | .number' | sort -n; }
issue_has() { echo "$issues_json" | jq -e --argjson n "$1" --arg l "$2" '.[] | select(.number == $n) | .labels | index($l)' >/dev/null; }
pr_open_for() { echo "$prs_json" | jq -e --arg h "$1" '.[] | select(.head == $h)' >/dev/null; }

# Stale claims: a label says a node holds the issue but no run of that node is alive.
release_stale() { # release_stale <label> <workflow> <title prefix> <back label>
  local label=$1 wf=$2 prefix=$3 back=$4 n since age
  for n in $(echo "$issues_json" | jq -r --arg l "$label" '.[] | select(.state == "OPEN" and (.labels | index($l))) | .number'); do
    running_titles "$wf" | grep -qxF "$prefix #$n" && continue
    since=$(gh api "repos/$REPO/issues/$n/events" --paginate --jq ".[] | select(.event == \"labeled\" and .label.name == \"$label\") | .created_at" | tail -1)
    [ -n "$since" ] || continue
    age=$(( ($(date +%s) - $(date -d "$since" +%s)) / 60 ))
    [ "$age" -ge "$STALE_MINUTES" ] || continue
    echo "stale $label on #$n ($age min); back to $back"
    gh issue edit "$n" --repo "$REPO" --remove-label "$label" --add-label "$back" >/dev/null
    gh issue comment "$n" --repo "$REPO" --body "<!-- stale-claim --> \`$label\`를 잡은 실행이 없어(${age}분) \`$back\`로 되돌렸다." >/dev/null
  done
}
release_stale in-progress graph-dev.yml dev ready-for-agent
release_stale verifying graph-verify.yml verify needs-verify

# 1. Upstream: a requirement document whose hash differs from its catalog entry.
for n in $(open_with requirement); do
  h=$(req_hash "$n")
  f="specs/requirements/R-$(pad4 "$n").md"
  cur=$(sed -n 's/^source_hash: *//p' "$f" 2>/dev/null | head -1)
  [ "$cur" = "$h" ] && continue
  pr_open_for "graph/upstream-$n" && continue
  [ "$(fail_count "$n" upstream "$h")" -ge "$FAIL_LIMIT" ] && continue
  dispatch graph-upstream.yml "upstream #$n" -f issue="$n"
done

# 2. Spec Drive: a confirmed catalog entry whose spec is missing or built from an older version.
for f in specs/requirements/R-*.md; do
  [ -f "$f" ] || continue
  id=$(sed -n 's/^id: *//p' "$f" | head -1)
  src=$(sed -n 's/^source: *"\{0,1\}#\([0-9]*\).*/\1/p' "$f" | head -1)
  h=$(sed -n 's/^source_hash: *//p' "$f" | head -1)
  grep -q '^status: *confirmed' "$f" || continue
  [ -n "$src" ] && issue_has "$src" needs-decision && continue
  spec=$(grep -lx "requirement: $id" specs/features/*/spec.md 2>/dev/null | head -1)
  ver=$([ -n "$spec" ] && sed -n 's/^requirement_version: *//p' "$spec" | head -1)
  [ "$ver" = "$h" ] && continue
  pr_open_for "graph/spec-$id" && continue
  [ -n "$src" ] && [ "$(fail_count "$src" spec-drive "$h")" -ge "$FAIL_LIMIT" ] && continue
  dispatch graph-spec-drive.yml "spec-drive $id" -f req="$id"
done

# 3. Odin, PR mode: every open PR that changes the ledger.
for n in $(echo "$prs_json" | jq -r '.[] | select((.labels | index("spec-pr")) and ((.labels | index("needs-decision")) | not)) | .number'); do
  sha=$(echo "$prs_json" | jq -r --argjson n "$n" '.[] | select(.number == $n) | .sha')
  [ "$(fail_count "$n" odin "$sha")" -ge "$FAIL_LIMIT" ] && continue
  dispatch graph-odin.yml "odin pr #$n" -f pr="$n"
done

# 3b. Odin, issue mode: a ticket whose Dev node reported a spec defect.
for n in $(open_with spec-defect); do
  [ "$(fail_count "$n" odin defect)" -ge "$FAIL_LIMIT" ] && continue
  dispatch graph-odin.yml "odin issue #$n" -f issue="$n"
done

# 4. Tracking issue per merged spec (no model needed), then Dev planning per spec version.
for spec in specs/features/*/spec.md; do
  [ -f "$spec" ] || continue
  slug=$(basename "$(dirname "$spec")")
  ver=$(sed -n 's/^requirement_version: *//p' "$spec" | head -1)
  req=$(sed -n 's/^requirement: *//p' "$spec" | head -1)
  t=$(echo "$issues_json" | jq -r --arg m "<!-- graph-spec: $slug -->" '.[] | select((.labels | index("spec")) and (.body | startswith($m))) | .number' | head -1)
  if [ -z "$t" ]; then
    reqf="specs/requirements/$req.md"
    src=$(sed -n 's/^source: *"\{0,1\}\(#[0-9]*\).*/\1/p' "$reqf" 2>/dev/null | head -1)
    title="스펙: $slug"
    t=$(gh issue create --repo "$REPO" --label spec --title "$title" --body "<!-- graph-spec: $slug -->
스펙: \`$spec\`
요구 사항: \`$req\` ($src)

개발 티켓은 이 이슈의 하위 이슈다. Dev(계획) 노드가 만든다." | sed 's#.*/##')
    echo "tracking issue #$t for $slug"
  fi
  issue_has "$t" needs-decision && continue
  [ "$(echo "$issues_json" | jq -r --argjson n "$t" '.[] | select(.number == $n) | .state')" = "CLOSED" ] && continue
  planned=$(gh api "repos/$REPO/issues/$t/comments" --paginate --jq ".[].body | select(startswith(\"<!-- dev-plan version=$ver -->\"))" | head -1)
  if [ -z "$planned" ]; then
    [ "$(fail_count "$t" dev-plan "$ver")" -ge "$FAIL_LIMIT" ] && continue
    dispatch graph-dev-plan.yml "dev-plan #$t" -f issue="$t"
    continue
  fi
  # Every ticket closed: the feature is done.
  subs=$(gh api "repos/$REPO/issues/$t/sub_issues" --paginate --jq '.[] | .state')
  if [ -n "$subs" ] && ! echo "$subs" | grep -q '^open$'; then
    gh issue close "$t" --repo "$REPO" --comment "<!-- graph-spec-done --> 모든 개발 티켓이 Verify를 거쳐 병합됐다. 진행 상태: \`specs/features/$slug/status.md\`" >/dev/null
    echo "tracking issue #$t closed"
  fi
done

# 5. Dev: unblocked tickets, in parallel up to MAX_DEV.
busy=$(echo "$issues_json" | jq '[.[] | select(.state == "OPEN" and (.labels | index("in-progress")))] | length')
for n in $(open_with ready-for-agent); do
  [ "$busy" -lt "$MAX_DEV" ] || { echo "Dev limit $MAX_DEV reached"; break; }
  issue_has "$n" in-progress && continue
  issue_has "$n" spec-defect && continue
  [ "$(open_blockers "$n")" -eq 0 ] || continue
  [ "$(fail_count "$n" dev unfinished)" -ge "$FAIL_LIMIT" ] && continue
  [ "$(fail_count "$n" verify verify)" -ge "$FAIL_LIMIT" ] && continue
  dispatch graph-dev.yml "dev #$n" -f issue="$n"
  busy=$((busy + 1))
done

# 6. Verify: one ticket at a time (merges are serial; see graph-verify.yml).
if [ -z "$(running_titles graph-verify.yml)" ]; then
  n=$(open_with needs-verify | head -1)
  if [ -n "$n" ] && ! issue_has "$n" verifying; then dispatch graph-verify.yml "verify #$n" -f issue="$n"; fi
fi

# 7. The whole graph is done: every requirement has a spec, every spec's tracking issue is closed,
#    and nothing is open in between. Mark the ledger once (the coordinator's watcher reports it).
open_work=$(echo "$issues_json" | jq '[.[] | select(.state == "OPEN" and ((.labels | index("spec")) or (.labels | index("ready-for-agent")) or (.labels | index("in-progress")) or (.labels | index("needs-verify")) or (.labels | index("verifying")) or (.labels | index("spec-defect")) or (.labels | index("needs-decision"))))] | length')
specs=$(ls specs/features/*/spec.md 2>/dev/null | wc -l)
node_runs=$(for s in queued in_progress waiting pending; do gh run list --repo "$REPO" --status $s --limit 50 --json workflowName --jq '.[].workflowName'; done | grep '^graph-' | grep -vc '^graph-router$' || true)
if [ "$ACTIVE" -eq 0 ] && [ "$node_runs" -eq 0 ] && [ "$open_work" -eq 0 ] && [ "$specs" -gt 0 ] && [ "$(echo "$prs_json" | jq length)" -eq 0 ]; then
  ln=$(gh issue list --repo "$REPO" --label budget-ledger --state all --limit 1 --json number --jq '.[0].number')
  if [ -n "$ln" ] && ! gh api "repos/$REPO/issues/$ln/comments" --paginate --jq '.[].body' | grep -q '<!-- graph-finished -->'; then
    gh issue comment "$ln" --repo "$REPO" --body "<!-- graph-finished --> 열린 일이 없다. 총비용 \$$(bash "$GRAPH_DIR/ledger.sh" total)." >/dev/null
    echo "graph finished"
  fi
fi
