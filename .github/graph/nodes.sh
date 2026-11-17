#!/usr/bin/env bash
# Guard and finish steps of every node: `nodes.sh <node> <step>`.
#
#   guard   re-reads the ledger and labels; prints go=true|false (and more) to $GITHUB_OUTPUT.
#           Running a node twice on the same ledger state does nothing the second time.
#   finish  turns what the model left in the working tree into ledger changes (files, labels, PRs).
#
# Env: REPO, GH_TOKEN, N (issue or PR number) or REQ (catalog id), and what each guard outputs.
set -uo pipefail
GRAPH_DIR=.github/graph
. "$GRAPH_DIR/lib.sh"
node=$1 step=$2
out() { echo "$1" >> "${GITHUB_OUTPUT:-/dev/stdout}"; }
no() { echo "::notice::$*"; out go=false; exit 0; }
budget() { bash "$GRAPH_DIR/ledger.sh" check || no "budget reached"; }
git_bot() {
  git config user.name "facet-graph[bot]"
  git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
  mkdir -p .factory
  printf '.factory/\n*.log\n' >> .git/info/exclude
}
# Drop every working-tree change outside the given path prefixes (a node may only touch its part).
keep_only() {
  local p keep
  git status --porcelain -uall | cut -c4- | while read -r p; do
    keep=false
    for prefix in "$@"; do case "$p" in "$prefix"*) keep=true;; esac; done
    $keep && continue
    echo "::warning::reverting $p (outside $*)"
    if git ls-files --error-unmatch "$p" >/dev/null 2>&1; then git checkout -q -- "$p"; else rm -f "$p"; fi
  done
}
changed() { [ -n "$(git status --porcelain -uall)" ]; }
state_of() { gh issue view "$1" --repo "$REPO" --json state --jq .state; }
pr_of_branch() { gh pr list --repo "$REPO" --head "$1" --state open --json number --jq '.[0].number // empty'; }
report_comment() { # report_comment <n> <marker> <file>
  [ -s "$3" ] || return 0
  { echo "<!-- $2 -->"; cat "$3"; } > .factory/comment.md
  comment "$1" .factory/comment.md
}
# A branch left behind by a run that pushed but never opened its PR blocks a plain push: start it over.
fresh_push() { [ -z "$(pr_of_branch "$1")" ] && git push -q origin --delete "$1" 2>/dev/null; git push -q origin "$1"; }

merge_pr() { # merge_pr <pr> : squash-merge, retrying while GitHub computes mergeability
  local i
  for i in 1 2 3 4 5; do
    gh pr merge "$1" --repo "$REPO" --squash --delete-branch && return 0
    sleep 10
  done
  return 1
}

case "$node:$step" in

# ---------------------------------------------------------------- Upstream
upstream:guard)
  budget
  [ "$(state_of "$N")" = OPEN ] || no "#$N is not open"
  has_label "$N" requirement || no "#$N is not a requirement"
  has_label "$N" needs-decision && no "#$N waits for a decision"
  h=$(req_hash "$N"); f="specs/requirements/R-$(pad4 "$N").md"
  [ "$(sed -n 's/^source_hash: *//p' "$f" 2>/dev/null | head -1)" = "$h" ] && no "catalog is up to date"
  [ -n "$(pr_of_branch "graph/upstream-$N")" ] && no "a catalog PR is open"
  out go=true; out hash="$h"
  ;;
upstream:finish)
  git_bot
  has_label "$N" needs-decision && { echo "Open Question raised"; exit 0; }
  f="specs/requirements/R-$(pad4 "$N").md"
  keep_only specs/requirements/
  [ -f "$f" ] || { mark_fail "$N" upstream "$HASH" "카탈로그 항목($f)을 쓰지도 질문하지도 않았다"; exit 0; }
  sed -i "s/^source_hash:.*/source_hash: $HASH/" "$f"
  if ! bash "$GRAPH_DIR/ledger-check.sh" > .factory/check.txt; then
    mark_fail "$N" upstream "$HASH" "원장 형식 검사 실패: $(head -c 1500 .factory/check.txt)"; exit 0
  fi
  b="graph/upstream-$N"
  git switch -c "$b" && git add specs/requirements && git commit -qm "요구 카탈로그: R-$(pad4 "$N")를 #$N에 맞춘다" && fresh_push "$b"
  pr=$(gh pr create --repo "$REPO" --head "$b" --title "요구 카탈로그: R-$(pad4 "$N")" --body "Upstream 노드가 요구 문서 #$N(해시 \`$HASH\`)에서 고친 요구 카탈로그 항목이다. 카탈로그는 Odin을 거치지 않고 병합하고, 이 변경에서 Spec Drive가 스펙을 고친다.

🤖 Generated with [Claude Code](https://claude.com/claude-code)" | sed 's#.*/##')
  [ -n "$pr" ] || { mark_fail "$N" upstream "$HASH" "카탈로그 PR을 만들지 못했다"; exit 0; }
  merge_pr "$pr" || mark_fail "$N" upstream "$HASH" "카탈로그 PR #$pr을 병합하지 못했다"
  ;;

# ---------------------------------------------------------------- Spec Drive
spec-drive:guard)
  budget
  f="specs/requirements/$REQ.md"
  [ -f "$f" ] || no "$REQ is not in the catalog"
  grep -q '^status: *confirmed' "$f" || no "$REQ is not confirmed"
  src=$(sed -n 's/^source: *"\{0,1\}#\([0-9]*\).*/\1/p' "$f" | head -1)
  h=$(sed -n 's/^source_hash: *//p' "$f" | head -1)
  has_label "$src" needs-decision && no "#$src waits for a decision"
  spec=$(grep -lx "requirement: $REQ" specs/features/*/spec.md 2>/dev/null | head -1)
  [ -n "$spec" ] && [ "$(sed -n 's/^requirement_version: *//p' "$spec" | head -1)" = "$h" ] && no "spec is up to date"
  [ -n "$(pr_of_branch "graph/spec-$REQ")" ] && no "a spec PR is open"
  out go=true; out hash="$h"; out src="$src"
  ;;
spec-drive:finish)
  git_bot
  has_label "$SRC" needs-decision && { echo "Open Question raised"; exit 0; }
  keep_only specs/features/
  for s in $(grep -lx "requirement: $REQ" specs/features/*/spec.md 2>/dev/null); do
    sed -i "s/^requirement_version:.*/requirement_version: $HASH/" "$s"
  done
  changed || { mark_fail "$SRC" spec-drive "$HASH" "$REQ의 스펙을 쓰지도 질문하지도 않았다"; exit 0; }
  slug=$(git status --porcelain -uall | cut -c4- | sed -n 's#^specs/features/\([^/]*\)/.*#\1#p' | head -1)
  b="graph/spec-$REQ"
  git switch -c "$b" && git add specs/features && git commit -qm "스펙: $slug ($REQ)" && fresh_push "$b"
  gh pr create --repo "$REPO" --head "$b" --label spec-pr --title "스펙: $slug ($REQ)" --body "Spec Drive 노드가 \`$REQ\`(요구 문서 #$SRC, 해시 \`$HASH\`)에서 쓴 스펙이다. Odin 노드가 원장 전체의 논리 결함을 검증하고 형식 검사를 통과하면 병합한다.

$(git show --stat --format= HEAD | sed 's/^/    /')

🤖 Generated with [Claude Code](https://claude.com/claude-code)" >/dev/null || mark_fail "$SRC" spec-drive "$HASH" "스펙 PR을 만들지 못했다"
  ;;

# ---------------------------------------------------------------- Odin
odin:guard)
  budget
  if [ -n "${PR:-}" ]; then
    [ "$(gh pr view "$PR" --repo "$REPO" --json state --jq .state)" = OPEN ] || no "PR #$PR is not open"
    has_label "$PR" spec-pr || no "PR #$PR is not a spec PR"
    has_label "$PR" needs-decision && no "PR #$PR waits for a decision"
    gh pr checkout "$PR" --repo "$REPO" >/dev/null || no "cannot check out PR #$PR"
    out go=true; out mode=pr; out subject="$PR"; out key="$(git rev-parse HEAD)"
  else
    [ "$(state_of "$ISSUE")" = OPEN ] || no "#$ISSUE is not open"
    has_label "$ISSUE" spec-defect || no "#$ISSUE has no spec-defect label"
    has_label "$ISSUE" needs-decision && no "#$ISSUE waits for a decision"
    out go=true; out mode=issue; out subject="$ISSUE"; out key=defect
  fi
  ;;
odin:finish)
  git_bot
  verdict=$(tr -d ' \r\n' < .factory/odin-verdict 2>/dev/null || true)
  report_comment "$SUBJECT" odin-report .factory/odin-report.md
  keep_only specs/
  case "$MODE:$verdict" in
    *:question)
      [ "$MODE" = issue ] && remove_label "$SUBJECT" spec-defect
      has_label "$SUBJECT" needs-decision || add_label "$SUBJECT" needs-decision
      ;;
    pr:pass|pr:fixed)
      if changed; then git add specs && git commit -qm "Odin: 원장의 결함을 고친다" && git push -q; fi
      if ! bash "$GRAPH_DIR/ledger-check.sh" > .factory/check.txt; then
        mark_fail "$SUBJECT" odin "$KEY" "원장 형식 검사 실패: $(head -c 1500 .factory/check.txt)"; exit 0
      fi
      merge_pr "$SUBJECT" || mark_fail "$SUBJECT" odin "$KEY" "스펙 PR을 병합하지 못했다"
      ;;
    issue:fixed)
      changed || { mark_fail "$SUBJECT" odin defect "fixed라고 했지만 원장을 바꾸지 않았다"; exit 0; }
      if ! bash "$GRAPH_DIR/ledger-check.sh" > .factory/check.txt; then
        mark_fail "$SUBJECT" odin defect "원장 형식 검사 실패: $(head -c 1500 .factory/check.txt)"; exit 0
      fi
      b="graph/odin-fix-$SUBJECT-${GITHUB_RUN_ID:-0}"
      git switch -c "$b" && git add specs && git commit -qm "Odin: #$SUBJECT가 알린 스펙 결함을 고친다" && git push -q origin "$b"
      pr=$(gh pr create --repo "$REPO" --head "$b" --title "Odin: #$SUBJECT의 스펙 결함 고침" --body "Dev 노드가 #$SUBJECT에서 알린 스펙 결함을 Odin 노드가 고쳤다. 판단은 #$SUBJECT의 Odin 보고에 있다.

🤖 Generated with [Claude Code](https://claude.com/claude-code)" | sed 's#.*/##')
      merge_pr "$pr" || { mark_fail "$SUBJECT" odin defect "고친 스펙 PR #$pr을 병합하지 못했다"; exit 0; }
      gh issue edit "$SUBJECT" --repo "$REPO" --remove-label spec-defect --add-label ready-for-agent >/dev/null
      ;;
    issue:pass)
      gh issue edit "$SUBJECT" --repo "$REPO" --remove-label spec-defect --add-label ready-for-agent >/dev/null
      ;;
    *)
      mark_fail "$SUBJECT" odin "$KEY" "판정(.factory/odin-verdict)을 남기지 않았다: '$verdict'"
      ;;
  esac
  ;;

# ---------------------------------------------------------------- Dev: plan
dev-plan:guard)
  budget
  [ "$(state_of "$N")" = OPEN ] || no "#$N is not open"
  has_label "$N" spec || no "#$N is not a tracking issue"
  has_label "$N" needs-decision && no "#$N waits for a decision"
  spec=$(gh issue view "$N" --repo "$REPO" --json body --jq .body | sed -n 's/^스펙: `\(.*\)`/\1/p' | head -1)
  [ -f "$spec" ] || no "spec $spec not on main"
  ver=$(sed -n 's/^requirement_version: *//p' "$spec" | head -1)
  gh api "repos/$REPO/issues/$N/comments" --paginate --jq '.[].body' | grep -qF "<!-- dev-plan version=$ver -->" && no "already planned for $ver"
  out go=true; out ver="$ver"
  ;;
dev-plan:finish)
  has_label "$N" needs-decision && { echo "Open Question raised"; exit 0; }
  subs=$(gh api "repos/$REPO/issues/$N/sub_issues" --paginate --jq '.[] | select(.state == "open") | .number')
  all=$(gh api "repos/$REPO/issues/$N/sub_issues" --paginate --jq '.[] | .number')
  [ -n "$all" ] || { mark_fail "$N" dev-plan "$VER" "개발 티켓(하위 이슈)을 만들지 않았다"; exit 0; }
  for s in $subs; do
    l=$(labels "$s")
    case "$l" in *,model:opus,*|*,model:sonnet,*) ;; *) gh issue edit "$s" --repo "$REPO" --add-label model:sonnet >/dev/null;; esac
    case "$l" in *,ready-for-agent,*|*,in-progress,*|*,needs-verify,*|*,verifying,*|*,spec-defect,*|*,needs-decision,*) ;;
      *) gh issue edit "$s" --repo "$REPO" --add-label ready-for-agent >/dev/null;; esac
  done
  gh issue comment "$N" --repo "$REPO" --body "<!-- dev-plan version=$VER --> 스펙 버전 \`$VER\`의 개발 티켓: $(echo $all | sed 's/\([0-9]*\)/#\1/g')" >/dev/null
  ;;

# ---------------------------------------------------------------- Dev: implement
dev:guard)
  budget
  [ "$(state_of "$N")" = OPEN ] || no "#$N is not open"
  l=$(labels "$N")
  case "$l" in *,ready-for-agent,*) ;; *) no "#$N is not ready-for-agent";; esac
  case "$l" in *,needs-decision,*|*,in-progress,*|*,spec-defect,*) no "#$N is held ($l)";; esac
  [ "$(open_blockers "$N")" -eq 0 ] || no "#$N has open blockers"
  gh issue edit "$N" --repo "$REPO" --remove-label ready-for-agent --add-label in-progress >/dev/null
  model=claude-sonnet-5-5
  case "$l" in *,model:opus,*) model=claude-opus-5-5;; esac
  out go=true; out model="$model"
  ;;
dev:prepare)
  git_bot
  b="graph/dev-$N"; conflict=false
  if git ls-remote --exit-code --heads origin "$b" >/dev/null; then
    git fetch -q origin "$b" && git switch -q -c "$b" "origin/$b"
    if ! git merge -q --no-edit origin/main; then conflict=true; fi
  else
    git switch -q -c "$b"
    git commit -q --allow-empty -m "#$N 작업을 시작한다"
    git push -q -u origin "$b"
  fi
  if [ -z "$(pr_of_branch "$b")" ]; then
    title=$(gh issue view "$N" --repo "$REPO" --json title --jq .title)
    gh pr create --repo "$REPO" --draft --head "$b" --title "$title" --body "Closes #$N

(Dev 노드가 작업 중이다. 100턴 안에 끝나지 않으면 다음 Dev 실행이 이 브랜치에서 이어 간다.)" >/dev/null
  fi
  out conflict="$conflict"
  ;;
dev:finish)
  b="graph/dev-$N"
  if [ -f .git/MERGE_HEAD ] && git diff --name-only --diff-filter=U | grep -q .; then
    echo "::warning::merge conflicts left unresolved; aborting the merge"; git merge --abort
  fi
  if changed; then
    if [ "$(git rev-list --count origin/main..HEAD)" -gt 0 ] || [ -f .git/MERGE_HEAD ]; then
      git add -A && git commit -qm "Dev: 커밋하지 않고 남긴 변경을 커밋한다"
    else
      git reset -q --hard && git clean -fdq
    fi
  fi
  git push -q origin "HEAD:$b" || echo "::warning::push failed"
  if [ "${CANCELLED:-false}" = true ]; then
    gh issue edit "$N" --repo "$REPO" --remove-label in-progress --add-label ready-for-agent >/dev/null
    gh issue comment "$N" --repo "$REPO" --body "<!-- dev-cancelled --> Dev 실행이 취소·시간 초과로 끝나 \`ready-for-agent\`로 되돌렸다. 커밋한 데까지는 \`$b\`에 남았다." >/dev/null
    exit 0
  fi
  l=$(labels "$N")
  case "$l" in
    *,needs-decision,*|*,spec-defect,*)
      gh issue edit "$N" --repo "$REPO" --remove-label in-progress >/dev/null ;;
    *)
      if [ -f .factory/done ]; then
        { echo "Closes #$N"; echo; cat .factory/pr-body.md 2>/dev/null; echo; echo "🤖 Generated with [Claude Code](https://claude.com/claude-code)"; } > .factory/body.md
        gh pr edit "$(pr_of_branch "$b")" --repo "$REPO" --body-file .factory/body.md >/dev/null
        gh issue edit "$N" --repo "$REPO" --remove-label in-progress --add-label needs-verify >/dev/null
      else
        gh issue edit "$N" --repo "$REPO" --remove-label in-progress --add-label ready-for-agent >/dev/null
        mark_fail "$N" dev unfinished "끝냈다는 신호(.factory/done) 없이 끝났다(턴 한도 등). 다음 Dev 실행이 \`$b\`에서 이어 간다"
      fi ;;
  esac
  ;;

# ---------------------------------------------------------------- Verify
verify:guard)
  budget
  [ "$(state_of "$N")" = OPEN ] || no "#$N is not open"
  l=$(labels "$N")
  case "$l" in *,needs-verify,*) ;; *) no "#$N is not needs-verify";; esac
  case "$l" in *,needs-decision,*|*,verifying,*) no "#$N is held ($l)";; esac
  pr=$(pr_of_branch "graph/dev-$N")
  [ -n "$pr" ] || { gh issue edit "$N" --repo "$REPO" --remove-label needs-verify --add-label ready-for-agent >/dev/null; no "no PR for #$N"; }
  gh issue edit "$N" --repo "$REPO" --remove-label needs-verify --add-label verifying >/dev/null
  out go=true; out pr="$pr"
  ;;
verify:prepare)
  # Mechanical checks before the expensive build. Problems go to .factory/verify-problems.md.
  git_bot
  b="graph/dev-$N"; : > .factory/verify-problems.md
  git fetch -q origin "$b" && git switch -q -c "$b" "origin/$b"
  if ! git merge -q --no-edit origin/main; then
    git merge --abort
    echo "- main을 합치면 충돌한다. Dev가 \`git merge origin/main\`으로 충돌을 풀어야 한다." >> .factory/verify-problems.md
  fi
  git diff --name-only origin/main...HEAD | grep -E '^(specs|\.github)/' | sed 's/^/- 원장·워크플로를 바꿨다(Dev는 바꾸지 않는다): /' >> .factory/verify-problems.md
  body=$(gh issue view "$N" --repo "$REPO" --json body --jq .body | tr -d '\r')
  for t in $(echo "$body" | sed -n '/^## Acceptance tests/,/^## /p' | sed -n 's/^- *`\{0,1\}\(specs\/[^` ]*\.rs\).*/\1/p'); do
    if [ ! -f "$t" ]; then echo "- 인수 시험 원본이 원장에 없다: $t" >> .factory/verify-problems.md; continue; fi
    target=$(sed -n 1p "$t" | tr -d '\r' | sed 's#^// target: *##')
    if [ ! -f "$target" ]; then echo "- 인수 시험 $t를 $target에 옮기지 않았다" >> .factory/verify-problems.md
    elif ! cmp -s <(tr -d '\r' < "$t") <(tr -d '\r' < "$target"); then echo "- $target이 원장의 $t와 글자 그대로 같지 않다" >> .factory/verify-problems.md; fi
  done
  if [ -s .factory/verify-problems.md ]; then out mech=fail; else out mech=pass; fi
  ;;
verify:finish)
  b="graph/dev-$N"
  verdict=$(tr -d ' \r\n' < .factory/verify-verdict 2>/dev/null || true)
  if [ "${CANCELLED:-false}" = true ]; then
    gh issue edit "$N" --repo "$REPO" --remove-label verifying --add-label needs-verify >/dev/null; exit 0
  fi
  if [ "$MECH" = pass ] && [ "$CI_OUTCOME" != success ]; then
    { echo "- \`cargo xtask ci\`가 실패했다. 끝부분:"; echo '```'; tail -n 60 .factory/ci.log 2>/dev/null; echo '```'; } >> .factory/verify-problems.md
  fi
  if [ ! -s .factory/verify-problems.md ] && [ "$verdict" = pass ]; then
    git reset -q --hard
    body=$(gh issue view "$N" --repo "$REPO" --json body --jq .body | tr -d '\r')
    spec=$(echo "$body" | sed -n '/^## Spec/,/^## /p' | grep -o 'specs/features/[^` ]*/spec.md' | head -1)
    st="$(dirname "$spec")/status.md"
    [ -f "$st" ] || printf '# 진행 상태\n\nVerify 노드만 쓴다.\n\n| AC | 티켓 | PR | 검증 시각 |\n|---|---|---|---|\n' > "$st"
    for a in $(echo "$body" | sed -n '/^## Acceptance criteria/,/^## /p' | grep -o 'AC-[0-9]*' | sort -u); do
      echo "| $a | #$N | #$PR | $(date -u +%FT%TZ) |" >> "$st"
    done
    git add "$st" && git commit -qm "Verify: #$N의 인수 조건을 진행 상태에 적는다"
    git push -q origin "HEAD:$b" || { mark_fail "$N" verify verify "브랜치에 밀어 넣지 못했다"; gh issue edit "$N" --repo "$REPO" --remove-label verifying --add-label ready-for-agent >/dev/null; exit 0; }
    gh pr ready "$PR" --repo "$REPO" >/dev/null || true
    report_comment "$N" verify-report .factory/verify-report.md
    if merge_pr "$PR"; then
      [ "$(state_of "$N")" = OPEN ] && gh issue close "$N" --repo "$REPO" >/dev/null
      gh issue edit "$N" --repo "$REPO" --remove-label verifying >/dev/null
    else
      gh issue edit "$N" --repo "$REPO" --remove-label verifying --add-label needs-verify >/dev/null
      mark_fail "$N" verify verify "검증은 통과했지만 PR #$PR을 병합하지 못했다"
    fi
  else
    [ -s .factory/verify-report.md ] && { echo; echo "### 인수 조건 검토"; cat .factory/verify-report.md; } >> .factory/verify-problems.md
    [ -n "$verdict" ] || [ "$CI_OUTCOME" != success ] || [ "$MECH" != pass ] || echo "- 검토 판정이 없다" >> .factory/verify-problems.md
    gh issue edit "$N" --repo "$REPO" --remove-label verifying --add-label ready-for-agent >/dev/null
    mark_fail "$N" verify verify "검증을 통과하지 못해 Dev에게 돌려보낸다.
$(head -c 6000 .factory/verify-problems.md)"
  fi
  ;;

*) echo "usage: nodes.sh <upstream|spec-drive|odin|dev-plan|dev|verify> <guard|prepare|finish>" >&2; exit 2 ;;
esac
