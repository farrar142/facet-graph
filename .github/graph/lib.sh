# Shared helpers of the autonomous graph. Source it; needs REPO and GH_TOKEN.
#
# Logic lives in these scripts, not in workflow files: GITHUB_TOKEN may not push a branch whose
# tree changes `.github/workflows/`, so workflow files must stay unchanged while node branches exist.

GRAPH_DIR=${GRAPH_DIR:-.github/graph}
FAIL_LIMIT=${GRAPH_FAIL_LIMIT:-2}

# Comma-separated labels of an issue or PR, wrapped in commas: ",a,b,".
# Through the issues API, so the same helpers work on issue and PR numbers alike.
labels() {
  echo ",$(gh api "repos/$REPO/issues/$1" --jq '[.labels[].name] | join(",")'),"
}
has_label() { case "$(labels "$1")" in *",$2,"*) return 0;; esac; return 1; }
add_label() { gh api "repos/$REPO/issues/$1/labels" -f "labels[]=$2" >/dev/null; }
remove_label() { gh api -X DELETE "repos/$REPO/issues/$1/labels/$2" >/dev/null 2>&1 || true; }
comment() { gh api "repos/$REPO/issues/$1/comments" -F "body=@$2" >/dev/null; } # comment <n> <file>
comment_text() { local f; f=$(mktemp); printf '%s' "$2" > "$f"; comment "$1" "$f"; rm -f "$f"; }

# Hash of a requirement document: its body plus every human comment (answers to Open Questions
# count as part of the requirement). Bot comments and marker comments do not count.
req_hash() {
  gh issue view "$1" --repo "$REPO" --json body,comments --jq '
    [.body] + [.comments[] | select(.author.login | test("^github-actions") | not)
               | select(.body | startswith("<!--") | not) | .body] | join("\n----\n")' \
    | sha256sum | cut -c1-16
}

pad4() { printf '%04d' "$1"; }

# Number of open blockers of an issue.
open_blockers() { gh api "repos/$REPO/issues/$1" --jq '.issue_dependencies_summary.blocked_by // 0'; }

# Comment bodies of an issue or PR after the last answered Open Question.
comments_since_answer() {
  gh api "repos/$REPO/issues/$1/comments" --paginate --jq '.[].body | @json' | jq -rs '
    ([to_entries[] | select(.value | startswith("<!-- open-question-answered -->")) | .key] | last) as $i
    | (if $i == null then . else .[($i + 1):] end) | .[]'
}

# Count failures of <node> with <key> on issue/PR <n> since the last answered Open Question.
fail_count() {
  comments_since_answer "$1" | grep -c "<!-- graph-failed node=$2 key=$3 -->" || true
}

# Record a failure: mark_fail <n> <node> <key> <message>. At FAIL_LIMIT failures the subject becomes
# an Open Question (only this branch stops; the owner is notified through the boss's mailbox).
mark_fail() {
  local n=$1 node=$2 key=$3 msg=$4 url="${GITHUB_SERVER_URL:-https://github.com}/$REPO/actions/runs/${GITHUB_RUN_ID:-0}"
  comment_text "$n" "<!-- graph-failed node=$node key=$key -->
노드 \`$node\` 실패: $msg
실행: $url"
  local c; c=$(fail_count "$n" "$node" "$key")
  if [ "$c" -ge "$FAIL_LIMIT" ]; then escalate "$n" "$node" "$key" "$c"; fi
}

escalate() {
  local n=$1 node=$2 key=$3 c=$4
  comment_text "$n" "<!-- open-question -->
노드 \`$node\`가 같은 상태(\`$key\`)에서 $c번 실패해 이 갈래만 멈췄다. 위의 실패 댓글과 실행 기록을 보고 답해 달라.
- 다시 시도: 이 댓글에 지시를 적어 답한다(답한 뒤부터 실패 수를 다시 센다).
- 범위를 줄이거나 바꾼다: 어떻게 바꿀지 답한다.
- 그만둔다: 이슈를 닫는다.
추천: 실패 까닭이 일시적이면 다시 시도."
  add_label "$n" needs-decision
}

# Titles of runs of a workflow that have not completed.
running_titles() {
  gh run list --repo "$REPO" --workflow "$1" --limit 100 --json status,displayTitle \
    --jq '.[] | select(.status != "completed") | .displayTitle'
}

wake_router() { gh workflow run graph-router.yml --repo "$REPO" >/dev/null || true; }
