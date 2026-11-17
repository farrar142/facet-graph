# 원장 (SSOT)

자율 그래프 시범의 단 하나의 원장이다. 요구 사항, 계약(API), 인수 조건, E2E 시험, 진행 상태를 이 폴더에 둔다. 노드(run)는 앞 run을 기억하지 않는다. 일을 시작할 때마다 이 폴더와 이슈의 라벨만 보고 컨텍스트를 되찾고, 몇 번 다시 돌아도 결과가 같다.

## 배치

```
specs/
  README.md                      이 문서(원장의 약속)
  requirements/R-0001.md         요구 사항 카탈로그 항목 하나 (Upstream 노드가 씀)
  features/<slug>/spec.md        스펙: 계약·인수 조건·E2E 시험 목록 (Spec Drive 노드가 씀, Odin이 검증)
  features/<slug>/acceptance/*.rs  E2E(인수) 시험 원본. 컴파일되지 않는 자리다. Dev가 머리말의 target 자리에 글자 그대로 옮긴다
  features/<slug>/status.md      인수 조건별 진행 상태 (Verify 노드만 씀)
```

### 요구 사항 카탈로그 `requirements/R-NNNN.md`

머리말(YAML):

```
---
id: R-0001
source: "#1"              # 요구 문서 이슈
source_hash: <sha256>     # 요구 문서 본문의 해시. Upstream 워크플로가 채운다
status: confirmed         # confirmed | question
---
```

본문: 목표, 요구 항목(`R-0001.1`, `R-0001.2` …, 한 줄에 하나, 검증할 수 있는 문장), 지킬 제약(ADR·성능 기준), 노드가 정한 기본값, 범위 밖.

### 스펙 `features/<slug>/spec.md`

머리말:

```
---
feature: <slug>
requirement: R-0001
requirement_version: <요구 항목의 source_hash>
---
```

본문 절(이 차례, 이 제목): `## Problem Statement`, `## Solution`, `## User Stories`, `## Contract`(코어 공개 인터페이스의 이름·시그니처·의미, 세션 필드, 명령 ID 등 다른 노드가 기대는 약속), `## Acceptance Criteria`(`- AC-1: …` 한 줄에 하나, 요구 항목 번호를 괄호로), `## E2E Tests`(시험 파일마다 다루는 AC), `## Manual Checks`(화면처럼 자동 시험할 수 없는 AC와 사람 확인 목록), `## Decided by the node`, `## Out of Scope`.

모든 AC는 E2E 시험 파일 하나 이상이나 `## Manual Checks`에 나온다.

### E2E 시험 `features/<slug>/acceptance/<name>.rs`

첫 두 줄이 머리말이다.

```
// target: crates/facet-core/tests/spec_<slug>_<name>.rs
// AC: AC-1, AC-3
```

`## Contract`의 인터페이스만 쓴다. Dev는 이 파일을 target 자리에 **글자 그대로** 옮기고 통과시킨다. 고치지 않는다. 시험이 틀렸다고 보면 Dev는 `spec-defect`로 Odin에게 돌린다. Verify가 글자 그대로인지 확인한다.

### 진행 상태 `features/<slug>/status.md`

Verify 노드가 티켓을 병합할 때 한 줄씩 더한다: `| AC | 티켓 | PR | 검증 시각 |`.

## 이슈 라벨 (원장의 움직이는 부분)

진행 중인 일의 상태는 파일이 아니라 이슈 라벨에 둔다. 병렬 노드가 같은 상태 파일을 고치면 서로 부딪히기 때문이다.

- `requirement`: 요구 문서(사람이 씀). 본문이 바뀌면 Upstream이 카탈로그를 고친다.
- `spec`: 기능 하나의 추적 이슈(라우터가 만듦). 개발 티켓은 그 하위 이슈다.
- `spec-pr`: 원장(`specs/`)을 바꾸는 PR. Odin이 검증해야 병합된다.
- `ready-for-agent`: 개발 티켓, Dev가 구현할 차례. `in-progress`: Dev가 잡고 있음.
- `needs-verify`: Dev가 끝냈다고 함, Verify 차례. `verifying`: Verify가 잡고 있음.
- `spec-defect`: Dev가 스펙 결함을 봤음, Odin 차례.
- `needs-decision`: Open Question. 그 갈래만 멈춘다. 사람이 댓글로 답하면(또는 라벨을 떼면) 다시 돈다.
- `model:opus`·`model:sonnet`: Dev가 쓸 모델.

## 노드와 흐름

노드끼리 서로 부르지 않는다. 라우터(`graph-router`)가 원장과 라벨을 읽고 할 일이 있는 노드를 띄운다. 노드는 끝날 때 원장을 바꾸고 라우터를 깨운다(같은 원장을 다시 읽으라는 신호일 뿐 다음 노드를 정하지 않는다). 라우터는 10분마다도 돈다.

1. Upstream: 요구 문서 → 카탈로그 PR(자동 병합)
2. Spec Drive: 확정된 카탈로그 항목 → 스펙 PR(`spec-pr`)
3. Odin: 스펙 PR과 원장 전체의 논리 결함 검증 → 통과면 병합, 고칠 수 있으면 고쳐 병합, 사람 판단이면 Open Question. `spec-defect` 티켓도 Odin이 본다
4. Dev(계획): 병합된 스펙 → 개발 티켓(하위 이슈, 막힘 관계)
5. Dev(구현): 막힘이 풀린 티켓마다 병렬, 100턴 한도, 드래프트 PR에 주기적으로 커밋, 끝나면 `needs-verify`
6. Verify: 자동 검사(`cargo xtask ci`)와 E2E 시험 글자 확인 → 병합과 status.md, 실패면 까닭을 남기고 Dev에게 돌림
