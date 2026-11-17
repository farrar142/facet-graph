---
feature: batch-rename
requirement: R-0013
requirement_version: 0e8b3e4bd0701480
---

## Problem Statement

지금 facet은 이름을 한 번에 하나씩만 바꾼다(F2 그 자리 이름바꾸기, `App::rename`). 사진 수백 장에 같은 앞말을 붙이거나 「 (1)」 같은 꼬리말을 지우거나 번호를 매기려면 항목마다 F2를 눌러야 한다. 손으로 하다 보면 이름이 겹치거나 쓸 수 없는 이름을 만들고, 중간에 실패하면 일부만 바뀐 채 남으며, 되돌리려면 Ctrl+Z를 항목 수만큼 눌러야 한다(이름바꾸기 기록이 항목마다 하나).

## Solution

1. **명령 하나** `CommandId::BatchRename`(「일괄 이름 바꾸기」, 기본 단축키 없음)을 더한다. 위치가 실제 폴더이거나 검색 결과 위치이고 두 항목 이상 선택했을 때 켜진다. 명령 팔레트와 항목 오른쪽 메뉴(「이름 바꾸기」 바로 아래 한 줄, 16줄)에서 닿는다. 실행하면 `Execution::BatchRename`으로 화면에 일괄 이름 바꾸기 창을 맡긴다.
2. **규칙과 미리 보기는 순수 함수**(새 모듈 `facet_core::batch_rename`, 화면·파일 시스템·시계를 부르지 않음, ADR-0007). 규칙 `RenameRule`은 찾아 바꾸기(글자 일치, 대소문자 무시, 바꿀 글자가 비면 지우기) → 앞·뒤에 붙이기 → 번호 매기기를 한 번에 쓴다. 파일은 확장자 앞(줄기)만 바꾸고 확장자는 그대로다. `preview`가 항목마다 원래 이름·바뀔 이름·상태(바꿈·그대로·문제와 까닭)를 내고, 겹침(선택 안에서 서로, 같은 폴더의 다른 항목과)과 쓸 수 없는 이름(`validate_name`)을 찾는다. 이름을 맞바꾸거나 다른 항목이 비울 이름을 쓰는 것(번호 밀기)은 겹침이 아니다.
3. **실행은 작업 대기열의 작업**(`JobKind::BatchRename`, 대표 답 2A). 작업 영역에 진행 표시와 「취소」가 생긴다. 실행 전 디스크에서 원본과 새 이름 자리를 확인하고, 같은 볼륨 이름 바꾸기(`file_ops::rename`, ADR-0005)로 바꾸며, 맞바꿈·사슬은 임시 이름을 거쳐 겹침 없이 바꾼다. **전부 아니면 전무**: 실패하거나 취소하면 이미 바꾼 항목을 모두 원래 이름으로 되돌리고 끝난다.
4. **작업 기록 한 건**(`OperationKind::BatchRename`, 항목마다 `Step::Renamed`). 실행취소 한 번이 모두 되돌리고 다시 실행 한 번이 모두 다시 한다. 실행취소·다시 실행도 전부 아니면 전무다(하나라도 밖에서 바뀌었으면 아무것도 바꾸지 않고 까닭을 보인다).
5. **검색 결과 위치**에서도 항목마다 제 폴더에서 바뀌고, 끝나면 모든 결과 탭에 새 이름이 반영된다(기존 `PathChange::of_step`).
6. **앱**: 일괄 이름 바꾸기 창(겹쳐 그림, 상시 화면 요소 없음)에 규칙 입력란, 항목마다 「원래 이름 → 바뀔 이름」과 문제의 까닭, 「바꿈 n · 건너뜀 m · 문제 k」를 보이고 문제가 있거나 바꿀 것이 없으면 「바꾸기」가 꺼진다. 화면 글자는 `src/text.rs`와 `@tr()`에만 둔다.
7. **속도**: 선택 1만 개 미리 보기 계산을 새 벤치마크 `facet-bench batch-rename`이 지킨다(로컬 16ms, CI 160ms).

## User Stories

- 사진 정리하는 사람으로서, 고른 사진 수백 장의 이름 앞에 「2026-11 제주 」를 한 번에 붙이고 싶다.
- 내려받은 파일이 많은 사람으로서, 「 (1)」 꼬리말을 한꺼번에 지우고 싶다.
- 자료를 묶는 사람으로서, 고른 파일에 목록 순서대로 001, 002, … 번호를 매기고 싶다.
- 조심스러운 사용자로서, 실행 전에 항목마다 원래 이름과 바뀔 이름을 보고, 겹치거나 쓸 수 없는 이름이 어디인지 까닭과 함께 알고 싶다.
- 일괄 이름 바꾸기를 잘못했을 때, Ctrl+Z 한 번으로 모든 이름을 되돌리고 싶다.
- 중간에 파일 하나가 잠겨 있어도 일부만 바뀐 채 남지 않았으면 좋겠다.
- 검색 결과에서 여러 폴더에 흩어진 파일을 골라 한꺼번에 이름을 바꾸고 싶다.
- 마우스만 쓰는 사람으로서, 여러 항목을 고르고 오른쪽 클릭해 「일괄 이름 바꾸기」를 누르고 싶다.

## Contract

### 새 모듈 `facet_core::batch_rename` (순수 함수, `lib.rs`에 `pub mod batch_rename;`)

```rust
/// 이름 규칙. 쓰는 차례: 찾아 바꾸기 → 붙이기·번호. 모든 필드가 비면(기본값) 아무 이름도 바꾸지 않는다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RenameRule {
    /// 찾을 글자. 비면 찾아 바꾸기를 하지 않는다.
    pub find: String,
    /// 바꿀 글자. 비면 찾은 글자를 지운다.
    pub replace: String,
    /// 앞에 붙일 글자.
    pub prefix: String,
    /// 뒤에(확장자 앞에) 붙일 글자.
    pub suffix: String,
    /// 번호 매기기. `None`이면 번호를 붙이지 않는다.
    pub numbering: Option<Numbering>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Numbering {
    /// 첫 항목의 번호. 항목 목록의 i번째(0부터) 항목은 `start + i`다.
    pub start: u64,
    /// 최소 자릿수. 모자라면 앞을 0으로 채운다(0·1이면 채우지 않음, 넘치면 그대로 길어짐).
    pub digits: usize,
    /// 번호를 줄기의 앞에 둘지 뒤에 둘지.
    pub position: NumberPosition,
    /// 번호와 줄기 사이에 넣을 글자(비면 붙여 씀).
    pub separator: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberPosition { Start, End }

/// 바꿀 항목 하나: 든 폴더, 원래 이름, 폴더인가.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchItem {
    pub folder: PathBuf,
    pub name: OsString,
    pub is_folder: bool,
}

/// 겹침을 볼 때 쓰는 폴더마다의 이름들(바꿀 항목이 아닌 다른 항목 포함, 숨김 항목 포함).
#[derive(Debug, Clone, Default)]
pub struct ExistingNames { /* … */ }
impl ExistingNames {
    pub fn new() -> Self;
    /// `folder`에 `name` 항목이 있다고 적는다. 같은 이름(대소문자 무시)을 여러 번 넣어도 하나다.
    pub fn insert(&mut self, folder: &Path, name: &OsStr);
}

/// 규칙을 항목들에 써 본 결과(아무것도 바꾸지 않는다).
pub fn preview(items: &[BatchItem], rule: &RenameRule, existing: &ExistingNames) -> Preview;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview { /* … */ }
impl Preview {
    /// 항목마다(넘긴 차례 그대로) 결과.
    pub fn entries(&self) -> &[PreviewEntry];
    /// 상태가 `EntryStatus::Rename`인 항목 수.
    pub fn rename_count(&self) -> usize;
    /// 상태가 `EntryStatus::Unchanged`인 항목 수(건너뛸 항목).
    pub fn unchanged_count(&self) -> usize;
    /// 상태가 `EntryStatus::Problem`인 항목 수.
    pub fn problem_count(&self) -> usize;
    /// 실행할 수 있는가: 문제가 하나도 없고 바꿀 항목이 하나 이상.
    pub fn can_execute(&self) -> bool;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewEntry {
    pub folder: PathBuf,
    pub original: OsString,
    /// 바뀔 이름(그대로면 원래 이름과 같다).
    pub new_name: String,
    pub status: EntryStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryStatus {
    /// 이름이 바뀐다.
    Rename,
    /// 규칙이 이름을 바꾸지 않는다(바뀔 이름이 원래 이름과 글자 그대로 같음). 건너뛴다.
    Unchanged,
    /// 바꿀 수 없다. 하나라도 있으면 실행할 수 없다.
    Problem(BatchProblem),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchProblem {
    /// 쓸 수 없는 이름(`name_input::validate_name`의 까닭 그대로).
    Invalid(NameError),
    /// 같은 폴더의 다른 바뀌는 항목과 바뀔 이름이 같다(대소문자 무시). 겹치는 항목 모두에 붙는다.
    Duplicate,
    /// 바뀔 이름이 같은 폴더에 남아 있을 다른 항목의 이름과 같다(대소문자 무시).
    NameTaken,
    /// 원래 이름이 유니코드로 읽을 수 없는 이름이라 규칙을 쓸 수 없다.
    NotUnicode,
}
```

규칙을 쓰는 법(`preview`):

- **줄기와 확장자**: 폴더는 이름 전체가 줄기다. 파일은 마지막 점 뒤가 확장자이고(점 포함), 점으로 시작해 다른 점이 없는 이름(`.gitignore`)과 점으로 끝나는 이름은 확장자가 없다(`name_input::extension_changed`와 같은 규칙). 규칙은 줄기에만 쓰고 확장자는 그대로 붙인다.
- **찾아 바꾸기**: 줄기 안에서 `find`를 왼쪽부터 겹치지 않게 모두 찾아 `replace`로 바꾼다. 견주기는 글자마다 대소문자 무시(유니코드 단순 접기: `name_match`와 같은 규칙)이고 바꿀 글자는 쓴 그대로 넣는다.
- **붙이기·번호**: 바꾼 줄기를 `s`, 번호 글자를 `n`(0 채움), 구분 글자를 `sep`라 하면 새 줄기는 `prefix + n + sep + s + suffix`(`Start`), `prefix + s + sep + n + suffix`(`End`), 번호가 없으면 `prefix + s + suffix`다. 붙인 글자에는 찾아 바꾸기를 쓰지 않는다.
- **상태**: 원래 이름이 유니코드가 아니면 `Problem(NotUnicode)`(`new_name`은 손실 변환한 원래 이름). 바뀔 이름이 원래 이름과 글자 그대로 같으면 `Unchanged`. `validate_name`이 막으면 `Problem(Invalid(까닭))`. 그 밖은 바뀌는 항목이다. 바뀌는 항목끼리 같은 폴더에서 바뀔 이름이 같으면(대소문자 무시) 모두 `Problem(Duplicate)`. 바뀔 이름이 「바뀐 뒤 그 폴더에 남는 이름」(=`existing`의 그 폴더 이름과 넘긴 항목들의 원래 이름에서 바뀌는 항목(문제가 있는 항목 포함)의 원래 이름을 뺀 것)과 같으면(대소문자 무시) `Problem(NameTaken)`. 그래서 대소문자만 바꾸는 이름, 다른 바뀌는 항목이 비우는 이름(번호 밀기·사슬·맞바꿈)은 겹침이 아니다. 폴더는 경로로 가르고 Windows에서는 대소문자를 무시한다. 다른 폴더끼리는 겹치지 않는다.
- 시간: 항목·이름 수에 거의 비례(해시 집합). 선택 1만 개에 로컬 16ms 안.

### 명령 `CommandId::BatchRename`

- `CommandId::ALL`에 `CommandId::Rename` 바로 뒤에 넣는다(배열 길이 56). `as_str()`은 `"batch-rename"`, `from_id("batch-rename")`가 되돌린다. `shortcuts()`는 비어 있다(기본 단축키 없음).
- `CommandId::is_enabled(tab)`: 탭 위치가 실제 폴더이거나 검색 결과 위치이고(`has_real_items`) `tab.selected_count() >= 2`일 때만 참. 커서 항목은 세지 않는다.
- `App::execute(CommandId::BatchRename)`는 켜져 있으면 `Execution::BatchRename`(새 변형)을 돌려주고 아무것도 바꾸지 않는다. 꺼져 있으면 `Execution::Disabled`.
- 앱 `text::command_name`의 이름은 「일괄 이름 바꾸기」.

### 오른쪽 메뉴 (`menu::build`, `MenuKind::Item`)

항목 메뉴의 고정 순서가 16줄이 된다(검색 결과 위치면 그 앞에 기존대로 「폴더에서 보기」와 구분선):

`Open, OpenInNewTab, |, Cut, Copy, Paste, CopyPath, |, CopyToOtherPanel, MoveToOtherPanel, CopyTo, MoveTo, |, Rename, BatchRename, Delete`

`BatchRename` 줄의 `enabled`는 `(location_is_folder || location_is_results) && selected_count >= 2`, `mark`는 `Mark::None`. `MenuContext`에 새 필드는 없다. 빈 곳 메뉴는 바뀌지 않는다.

**기존 원장과의 관계**: 대표 답 1A에 따라 `mouse-reachable-commands`의 AC-6과 그 인수 시험(15줄 항목 메뉴를 못박던 `item_menu_is_unchanged`)은 Odin이 이 16줄 순서로 고쳤다(시험 이름은 `item_menu_is_unchanged_by_the_new_fields`). Dev는 고친 `specs/features/mouse-reachable-commands/acceptance/layout.rs`를 그 target `crates/facet-core/tests/spec_mouse_reachable_commands_layout.rs`에 글자 그대로 다시 옮기고, `crates/facet-core/tests/menu.rs`의 항목 메뉴 순서 단언과 `crates/facet-app/src/context_menu.rs` 단위 테스트를 새 순서에 맞춘다.

### 앱 상태 (`facet_core::window::App`)

```rust
impl App {
    /// 일괄 이름 바꾸기할 항목: 활성 탭의 선택(목록 순서, 커서 항목은 더하지 않음), 항목마다 든 폴더
    /// (`Tab::folder_of`, 검색 결과면 제 폴더)·이름·폴더 여부. 명령이 꺼져 있으면 `None`.
    pub fn batch_rename_items(&self) -> Option<Vec<BatchItem>>;
    /// 그 항목들에 규칙을 쓴 미리 보기. `existing`은 실제 폴더 위치면 탭이 읽은 그 폴더의 모든 이름(숨김 항목 포함),
    /// 검색 결과 위치면 결과 목록에 있는 그 폴더 항목들의 이름이다(목록을 다시 읽지 않음). 명령이 꺼져 있으면 `None`.
    pub fn batch_rename_preview(&self, rule: &RenameRule) -> Option<Preview>;
    /// 지금 상태로 미리 보기를 다시 만들어 실행할 수 있으면 일괄 이름 바꾸기 작업을 넣고 곧바로 시작한다.
    pub fn start_batch_rename(&mut self, rule: &RenameRule) -> Result<JobId, BatchRenameError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchRenameError {
    /// 명령이 꺼져 있다(선택 둘 미만, 실제 폴더·검색 결과 위치가 아님, 창 없음).
    Unavailable,
    /// 미리 보기에 문제가 있다(`Preview::problem_count() > 0`).
    Problems,
    /// 바꿀 항목이 없다(모두 그대로).
    NothingToRename,
}
```

`start_batch_rename`이 `Err`면 작업을 만들지 않고 작업 영역·디스크·작업 기록이 그대로다.

### 작업 `JobKind::BatchRename`

- 새 변형 `facet_core::job::JobKind::BatchRename`. `App::start_batch_rename`이 앱의 작업 대기열에 넣어 **차례를 기다리지 않고 곧바로 시작**하고(삭제·실행취소와 같은 `start_now` 길) 작업 영역에 더한다(`JobEntry::kind == JobKind::BatchRename`, `target`은 첫 바뀌는 항목의 폴더, `sources`는 `rename_count`). 진행(항목 수)·상태·취소·끝남 사건은 다른 작업과 같고 `App::poll_jobs`가 받는다. 작업에는 바뀌는 항목만 넘긴다.
- **실행 전 확인**(아무것도 바꾸기 전): 디스크에서 항목마다 원본이 있는지, 새 이름 자리에 이 작업이 비우지 않을 다른 항목이 없는지(대소문자 무시, 대소문자만 바꾸는 이름은 같은 항목) 본다. 하나라도 걸리면 아무것도 바꾸지 않고 `JobState::Failed`로 끝나며 `Summary::failures`에 그 원본 경로와 까닭(`FileOpErrorKind::SourceMissing`·`TargetExists`)을 담는다. 검색 결과 위치처럼 미리 보기가 그 폴더의 이름을 다 모를 때의 겹침은 여기서 막힌다.
- **바꾸기**: `file_ops::rename`(같은 폴더 안 이름 바꾸기). 다른 바뀌는 항목이 비울 이름을 쓰는 항목(사슬·맞바꿈·대소문자만)은 같은 폴더 안 임시 이름을 거쳐 겹침 없이 바꾼다. 일시정지·취소는 항목 사이에서 듣는다. 항목별 오류 질문은 하지 않는다.
- **전부 아니면 전무**: 바꾸는 중 파일 연산이 실패하면(잠김·접근 거부 등) 묻지 않고 이미 바꾼 항목(임시 이름 포함)을 모두 원래 이름으로 되돌린 뒤 `JobState::Failed`(실패한 항목은 `Summary::failures`), 취소하면 같이 되돌린 뒤 `JobState::Cancelled`로 끝난다. 이때 작업 기록에는 아무것도 남지 않는다. 되돌리기마저 실패한 항목(드묾)은 `Summary::failures`에 지금 경로로 더 담고, 실제로 바뀐 채 남은 항목만 `OperationKind::BatchRename` 기록으로 남겨 실행취소할 수 있게 한다.
- **결과 요약**: 새 필드 `Summary::names_unchanged: u64`(규칙이 바꾸지 않아 건너뛴 항목 수, 미리 보기의 `unchanged_count`)를 더하고 `Summary::items_skipped()`에 넣는다. 끝나면(`Finished`) `Summary::items_renamed == rename_count`, `items_done == rename_count + unchanged_count`, 그래서 `items_completed() == rename_count`, `items_skipped() == unchanged_count`. 실패·취소로 모두 되돌렸으면 `items_renamed == 0`.

### 작업 기록 (`facet_core::history`)

- 새 변형 `OperationKind::BatchRename`. 끝난(`Finished`) 일괄 이름 바꾸기는 기록 **하나**로 남는다: 항목은 바꾼 항목마다 `Step::Renamed { from, to, stamp }`(넘긴 항목 차례), `Record::items_completed() == rename_count`, `Record::items_skipped() == unchanged_count`(`with_skipped`), 되돌릴 수 있음. `HISTORY_LIMIT`·다시 실행 목록 비우기 규칙은 그대로.
- **실행취소·다시 실행**(`App::undo`·`redo`, `JobKind::Undo`·`Redo`, 기존 길): `BatchRename` 기록은 전부 아니면 전무로 되돌린다. 먼저 모든 항목을 확인하고(다룰 항목이 있고 기록한 `Stamp`와 맞는지, 되바꿀 자리에 이 기록이 비우지 않을 다른 항목이 없는지) 하나라도 걸리면 아무것도 바꾸지 않고 `JobState::Failed`로 끝나며 걸린 항목마다 `Summary::not_reverted`(`NotReverted { path, reason }`: `Missing`·`Changed`·`NameTaken`)에 담는다. 그러면 기록은 통째로 제자리(실행취소면 작업 기록, 다시 실행이면 다시 실행 목록)에 남는다. 확인을 지나면 사슬·맞바꿈도 겹침 없이 모두 되바꾸고(같은 임시 이름 방식), 중간 실패면 되바꾼 것을 다시 돌려 놓고 실패로 끝난다. 모두 되돌리면 기록 전체가 반대편 목록으로 간다.
- 검색 결과 탭 반영은 기존대로(`PathChange::of_step`·`of_reverted`의 `Renamed`).

### 벤치마크 `facet-bench batch-rename`

- `facet_bench::measure_batch_rename(runs) -> Result<BatchRenameMeasurement, String>`: 시험 폴더(1만 항목, `measure_menu`와 같은 폴더)를 연 `App`에서 1만 개(`BATCH_RENAME_SELECTED`)를 모두 선택하고, 찾아 바꾸기·앞·뒤 붙이기·번호를 모두 쓴 규칙으로 `App::batch_rename_preview`(항목 모으기 포함)를 한 번 데우고 `runs`회(기본 5) 재어 중앙값·최악값을 낸다. 결과의 `rename_count`가 1만이 아니면 실패.
- 기준 `BATCH_RENAME_TARGET` 16ms(한 프레임), CI 상한 `BATCH_RENAME_LIMIT` 160ms(10배, `LIST_LIMIT`과 같은 까닭). 중앙값이 상한을 넘으면 실패한다. `cargo run --release -p facet-bench -- batch-rename [--runs N] [--limit-ms N]`.
- `cargo xtask bench`·`ci`의 단계로 더한다(오른쪽 메뉴 구성 벤치마크 뒤). `AGENTS.md`의 벤치마크 목록·기준을 고친다.

### 앱 (`crates/facet-app`)

- `Execution::BatchRename`을 받으면 창 위에 일괄 이름 바꾸기 창(`batch-*` 속성)을 겹쳐 그린다. 상태(규칙 입력값, 마지막 미리 보기, 고른 줄)는 `src/batch_rename.rs`(단위 테스트). 입력란: 찾을 글자, 바꿀 글자, 앞에 붙일 글자, 뒤에 붙일 글자, 「번호 매기기」 확인란과 시작 번호·자릿수·앞/뒤·구분 글자. 입력이 바뀔 때마다 UI 스레드에서 `App::batch_rename_preview`를 다시 불러(1만 개 16ms) 목록에 「원래 이름 → 바뀔 이름」, 문제 줄은 빨갛게 까닭(`text::batch_problem_text`: 쓸 수 없는 이름은 `validate_name` 까닭 글자와 같음, 겹침, 같은 이름 있음, 읽을 수 없는 이름), 그대로인 줄은 흐리게, 아래에 「바꿈 n · 건너뜀 m · 문제 k」(`text::batch_counts_text`)를 보인다. 목록은 보이는 줄만 그린다(`ListView`).
- 「바꾸기」 단추와 Enter는 `Preview::can_execute`일 때만 `App::start_batch_rename`을 부르고 창을 닫는다(작업 영역에 진행 표시). 막히면(`BatchRenameError`) 까닭을 창 아래에 보이고 창은 그대로. Esc·「취소」·바깥 누르기는 아무것도 바꾸지 않고 닫는다. 입력란은 키 입력란(`keys`) 밖이라 Ctrl+Z 등은 글자 편집이다.
- 작업 영역의 진행 표시 종류(`JobType`)에 「일괄 이름 바꾸기」, 끝나면 「바꿈 n · 건너뜀 m」, 실패·취소면 「모두 되돌림」을 보인다. `text::operation_name`에 `OperationKind::BatchRename`(「일괄 이름 바꾸기(n개)」)을 더해 실행취소 상태 줄·작업 기록 보기가 쓴다.
- `docs/ime-checklist.md`에 「일괄 이름 바꾸기 입력란」 절, `docs/context-menu-checklist.md`의 항목 메뉴 목록에 새 줄을 더한다.

## Acceptance Criteria

- AC-1: `batch_rename::preview`는 앞·뒤 붙이기를 파일의 확장자 앞 줄기에 쓰고 확장자를 그대로 두며, 폴더·확장자 없는 파일·`.gitignore` 같은 이름은 이름 전체를 줄기로 본다. (R-0013.1)
- AC-2: 찾아 바꾸기는 줄기 안의 모든 일치를 대소문자 무시로 바꾸고, 바꿀 글자가 비면 찾은 글자를 지우며, 붙인 글자와 확장자에는 쓰지 않는다. (R-0013.1)
- AC-3: 번호 매기기는 넘긴 항목 차례대로 `start`부터 1씩 늘린 번호를 `digits` 자리 0 채움으로 줄기의 앞(`prefix` 뒤) 또는 뒤(`suffix` 앞)에 구분 글자와 함께 붙이고, 번호가 자릿수를 넘으면 그대로 길어진다. (R-0013.1)
- AC-4: 미리 보기는 넘긴 항목마다 같은 차례로 폴더·원래 이름·바뀔 이름·상태를 하나씩 낸다. (R-0013.2)
- AC-5: 바뀌는 항목끼리 같은 폴더에서 바뀔 이름이 대소문자 무시로 같으면 그 항목들이 모두 `Problem(Duplicate)`이고, 다른 폴더끼리는 같은 이름이어도 문제가 아니다. (R-0013.3, R-0013.9)
- AC-6: 바뀔 이름이 같은 폴더에 남아 있을 다른 항목(선택 밖 항목, 그대로인 선택 항목)의 이름과 대소문자 무시로 같으면 `Problem(NameTaken)`이다. (R-0013.3)
- AC-7: 바뀔 이름이 금지 글자·예약 이름·끝의 점·끝의 공백·빈 이름·255 UTF-16 단위 초과이면 `Problem(Invalid(validate_name의 까닭))`이다. (R-0013.3)
- AC-8: 문제가 하나라도 있으면 `Preview::can_execute`가 거짓이고 `App::start_batch_rename`은 `Err(BatchRenameError::Problems)`를 돌려주며 작업을 만들지 않고 디스크·작업 기록이 그대로다. (R-0013.3)
- AC-9: 다른 바뀌는 항목이 비우는 이름을 쓰는 것(사슬 `a→aa, aa→aaaa`, `aa→a, aaaa→aa`)과 대소문자만 바꾸는 것은 미리 보기에서 문제가 아니고, 실행하면 디스크에서 모두 바뀐다. (R-0013.5)
- AC-10: 규칙이 이름을 바꾸지 않는 항목은 `Unchanged`이고 `unchanged_count`로 세며, 실행하면 그 항목은 디스크에서 그대로이고 결과 요약 `names_unchanged`·`items_skipped()`와 기록의 `items_skipped()`가 그 수다. 모두 그대로면 `can_execute`가 거짓이고 `start_batch_rename`은 `Err(NothingToRename)`이다. (R-0013.7)
- AC-11: `App::start_batch_rename`은 `JobKind::BatchRename` 작업을 곧바로 시작해 작업 영역에 더하고, 끝나면 바뀌는 항목이 모두 새 이름이고 `items_completed()`가 바꾼 수다. (R-0013.1, R-0013.10)
- AC-12: 바꾸는 중 한 항목의 파일 연산이 실패하면(공유 없이 연 잠긴 파일) 작업은 `Failed`로 끝나고 `Summary::failures`에 그 항목이 들며 모든 항목이 원래 이름이고 작업 기록에 새 기록이 없다. (R-0013.4, R-0013.10)
- AC-13: 실행 전 디스크 확인에서 새 이름 자리에 미리 보기가 몰랐던 항목이 있으면(검색 결과 위치) 아무것도 바꾸지 않고 `Failed`로 끝나며 `failures`의 까닭이 `TargetExists`다. (R-0013.4, R-0013.9)
- AC-14: 일괄 이름 바꾸기 작업을 시작 직후 취소하면 끝난 상태가 `Cancelled`면 모든 항목이 원래 이름이고 기록이 없으며, `Finished`면 모든 항목이 새 이름이고 기록이 하나다(일부만 바뀐 상태는 없다). (R-0013.4, R-0013.10)
- AC-15: 끝난 일괄 이름 바꾸기는 작업 기록에 `OperationKind::BatchRename` 기록 하나(바꾼 항목마다 `Step::Renamed`)로 남고, 실행취소 한 번으로 모든 항목이 원래 이름·내용·수정 시각으로 돌아가며, 다시 실행 한 번으로 모두 다시 새 이름이 된다. (R-0013.6)
- AC-16: 실행취소할 때 한 항목이라도 밖에서 바뀌었으면(내용을 고침) 아무 항목도 되돌리지 않고 `Failed`로 끝나며 `not_reverted`에 그 항목이 `Changed`로 들고 기록은 실행취소 대상으로 남는다. (R-0013.6, R-0013.4)
- AC-17: `CommandId::BatchRename`은 `CommandId::ALL`에 있고 id가 `"batch-rename"`이며 기본 단축키가 없고, 위치가 실제 폴더이거나 검색 결과 위치이며 두 항목 이상 선택했을 때만 켜지고(선택 0·1개, 커서만 있으면 꺼짐), 실행하면 `Execution::BatchRename`이다. (R-0013.8)
- AC-18: 항목 메뉴는 16줄이고 `BatchRename` 줄이 `Rename` 바로 아래(`Delete` 위)에 있으며 켜짐이 명령의 켜짐과 같다(앱에서 `App::context_menu(MenuKind::Item)`의 그 줄 `enabled == App::can_execute(BatchRename)`). 명령 팔레트 검색(`App::search_commands`)에도 나오고 켜짐이 같다. (R-0013.8)
- AC-19: 검색 결과 위치에서 여러 폴더의 항목을 골라 실행하면 항목마다 제 폴더에서 바뀌고, 결과 탭의 경로가 새 이름이 되며, 실행취소하면 모두 원래 경로로 돌아온다. (R-0013.9, R-0013.6)
- AC-20: `App::batch_rename_items`는 선택만 목록 순서로 항목마다 제 폴더와 함께 주고, `App::batch_rename_preview`는 같은 폴더의 선택하지 않은 항목 이름과의 겹침을 `NameTaken`으로 낸다. (R-0013.2, R-0013.3)
- AC-21: 선택 1만 개 미리 보기 계산(`App::batch_rename_preview`)은 `facet-bench batch-rename`의 CI 상한 `BATCH_RENAME_LIMIT`(160ms) 안이다. (R-0013.2)
- AC-22: 앱의 일괄 이름 바꾸기 창은 규칙을 칠 때마다 항목마다 원래 이름과 바뀔 이름, 문제 줄의 까닭, 바꿈·건너뜀·문제 수를 보이고, 문제가 있거나 바꿀 것이 없으면 「바꾸기」가 꺼진다. (R-0013.2, R-0013.3, R-0013.7)
- AC-23: 작업 영역에 「일괄 이름 바꾸기」 진행 표시와 「취소」가 보이고, 실행취소하면 상태 줄에 「일괄 이름 바꾸기(n개) 실행취소」가 보인다. (R-0013.4, R-0013.6, R-0013.10)

## E2E Tests

- `acceptance/preview.rs` → `crates/facet-core/tests/spec_batch_rename_preview.rs`: AC-1, AC-2, AC-3, AC-4, AC-5, AC-6, AC-7, AC-8, AC-9, AC-10 (순수 `preview`: 줄기·확장자, 찾아 바꾸기·지우기, 번호, 차례, 겹침·남는 이름·쓸 수 없는 이름, 사슬·대소문자, 그대로인 항목. 모든 환경).
- `acceptance/command.rs` → `crates/facet-core/tests/spec_batch_rename_command.rs`: AC-8, AC-10, AC-17, AC-18, AC-20 (명령 id·단축키·켜짐, 순수 항목 메뉴 16줄, 임시 폴더 `App`의 켜짐·메뉴·팔레트 일치, 항목 모으기·선택 밖 이름 겹침, 실행 전 거절은 작업을 만들지 않음. 모든 환경).
- `acceptance/execute.rs` → `crates/facet-core/tests/spec_batch_rename_execute.rs`: AC-9, AC-10, AC-11, AC-12, AC-13, AC-14, AC-15, AC-16, AC-19 (Windows 전용, 임시 폴더의 실제 파일: 실행·건너뜀·기록 하나·실행취소·다시 실행, 사슬, 잠긴 파일의 전부 되돌림, 취소의 전부 아니면 전무, 밖의 변경이 있는 실행취소, 검색 결과 위치의 여러 폴더와 디스크 확인).
- AC-21은 새 벤치마크 `facet-bench batch-rename`(`cargo xtask ci`의 단계)이 지킨다.

## Manual Checks

- AC-21: 시험 파일이 아니라 벤치마크가 지킨다. Verify는 `cargo xtask ci` 출력에서 「일괄 이름 바꾸기 미리 보기」 단계가 `BATCH_RENAME_LIMIT`(160ms) 안으로 통과했는지 확인한다.
- AC-22 (Windows 기계, `docs/context-menu-checklist.md`·`docs/ime-checklist.md`에 더함):
  - 파일 셋 이상을 고르고 오른쪽 클릭 → 「이름 바꾸기」 바로 아래 「일괄 이름 바꾸기」가 보이고, 하나만 고르면 흐리다. 명령 팔레트에서 「일괄」로 찾아진다.
  - 창에서 앞에 붙일 글자를 치는 즉시 목록의 바뀔 이름이 바뀌고 확장자는 그대로다. 찾을 글자 「 (1)」, 바꿀 글자 비움이면 꼬리말이 지워진다.
  - 「번호 매기기」를 켜고 자릿수 3, 뒤, 구분 「_」이면 목록 순서대로 `_001`, `_002`…가 붙는다.
  - 두 항목이 같은 이름이 되게 하면 두 줄이 빨갛게 「겹침」, 선택 밖 파일 이름이 되게 하면 「같은 이름이 있음」, 「:」를 치면 「쓸 수 없는 글자 :」가 보이고 「바꾸기」가 꺼지며 Enter가 아무것도 하지 않는다.
  - 규칙을 비우면 모든 줄이 흐리고 「바꿈 0 · 건너뜀 n」이며 「바꾸기」가 꺼진다.
  - 한글 IME로 각 입력란에 「사진」을 조합해 치면 조합이 끝난 글자로 미리 보기가 바뀌고, 입력란 안의 Ctrl+Z는 글자만 되돌린다.
  - 1만 개를 골라 창을 열고 글자를 쳐도 끊김 없이 미리 보기가 바뀐다(10만 항목 시험 폴더에서 Ctrl+A 뒤 1만 개는 사각형 선택 등으로).
  - Esc·바깥 누르기·「취소」는 아무것도 바꾸지 않는다.
- AC-23 (Windows 기계):
  - 「바꾸기」를 누르면 창이 닫히고 작업 영역에 「일괄 이름 바꾸기」 진행 표시가 생겼다 끝나면 「바꿈 n · 건너뜀 m」, 목록에 새 이름이 보인다.
  - 그 직후 Ctrl+Z → 상태 줄에 「일괄 이름 바꾸기(n개) 실행취소」, 모든 이름이 돌아온다. Ctrl+Y로 모두 다시 바뀐다. 작업 기록 보기에 한 줄로 보인다.
  - 고른 파일 하나를 다른 프로그램에서 열어 잠근 채 실행하면 진행 표시가 실패와 「모두 되돌림」을 보이고 모든 이름이 원래대로다.
  - 1만 개를 바꾸는 중 「취소」를 누르면 「모두 되돌림」으로 끝나고 모든 이름이 원래대로다.
  - 검색 결과에서 두 폴더의 파일을 골라 바꾸면 각 폴더에서 바뀌고 결과 목록·위치 열이 새 이름을 보인다.

## Decided by the node

- 규칙 셋(찾아 바꾸기 → 붙이기 → 번호)을 한 창에서 함께 쓴다: 카탈로그의 세 예를 덮는 가장 작은 집합이고, 하나만 쓰려면 나머지를 비우면 되어 규칙 목록 화면이 필요 없다. 더 필요하면 `RenameRule`에 필드를 더한다.
- 파일은 확장자 앞 줄기만 바꾼다: 확장자 변경은 범위 밖이고, F2 이름 입력란도 확장자 앞까지 고른다. 규칙은 `name_input`의 확장자 규칙을 그대로 쓴다.
- 번호는 줄기 안쪽(앞이면 `prefix` 뒤, 뒤면 `suffix` 앞)에 구분 글자와 함께 둔다: 「휴가 01-사진」·「IMG_001」 같은 흔한 모양을 붙이기와 번호만으로 만들 수 있다.
- 번호는 선택의 목록 순서로 폴더와 상관없이 이어 매긴다: 카탈로그의 노드 결정과 같다. 문제·그대로인 항목도 번호 자리를 차지한다(미리 보기에서 번호가 어긋나 보이지 않게).
- 찾아 바꾸기의 대소문자 무시는 `name_match`와 같은 단순 접기다: 앱의 다른 이름 일치(필터)와 같은 느낌이어야 한다.
- 그대로인지는 글자 그대로 견준다: 대소문자만 바뀌는 것은 사용자가 바란 이름바꾸기라 바꾼다(Windows도 같은 항목으로 본다).
- 실행 중 실패는 묻지 않고 곧바로 모두 되돌린다: 다시 시도·건너뛰기 질문은 전부 아니면 전무와 맞지 않고, 다시 하려면 원인을 치운 뒤 같은 규칙으로 다시 실행하면 된다.
- 작업은 차례를 기다리지 않고 곧바로 시작한다: 이름 바꾸기는 디스크를 거의 쓰지 않고, 앞의 큰 복사 뒤로 미루면 사용자가 지금 보는 이름이 한참 그대로 남는다(삭제·실행취소와 같은 까닭).
- 맞바꿈·사슬은 같은 폴더 안 임시 이름을 거친다: 같은 볼륨 이름 바꾸기 두 번이라 싸고, 덮어쓰기 없이 어떤 순열도 풀린다. 임시 이름 형식은 Dev 몫(복사의 `~facet-<pid>-<n>.tmp`를 따름).
- 미리 보기는 UI 스레드에서 입력마다 다시 계산한다: 1만 개 16ms 예산 안이고 일꾼 스레드·늦은 결과 버리기가 필요 없다.
- 검색 결과 위치의 겹침은 결과 목록에 있는 이름으로만 미리 보고 나머지는 실행 직전 디스크 확인이 막는다: 결과 위치는 폴더의 다른 항목을 다 모르고, 미리 보기에서 디스크를 읽으면 순수 함수 규칙과 속도 예산을 깬다.
- 기본 단축키를 두지 않는다: 요구가 팔레트와 오른쪽 메뉴만 들고, 남은 흔한 키(Ctrl+Shift+R 등)를 쓰면 다른 프로그램 습관과 부딪힐 수 있다.

## Out of Scope

- 정규식 규칙, 이름 전체 바꾸기, 대소문자 바꾸기 규칙, 내용·메타데이터(EXIF·수정 날짜) 기반 이름.
- 확장자를 바꾸는 규칙, 폴더를 옮기는 이름바꾸기.
- 규칙 프리셋 저장(세션 필드 없음).
- F2 이름바꾸기의 동작 변경, 빈 곳 메뉴 변경.
- 실행 중 항목별 오류 질문(다시 시도·건너뛰기).
