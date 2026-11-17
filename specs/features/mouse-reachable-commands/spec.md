---
feature: mouse-reachable-commands
requirement: R-0001
requirement_version: 42e7e62452e6092d
---

## Problem Statement

마우스만 쓰는 사람은 단축키(Backspace·Alt+←/→/↑, Ctrl+Z, Ctrl+백슬래시, Ctrl+F)와 명령 팔레트(Ctrl+Shift+P)를 모르면 뒤로·앞으로·위로 이동, 실행취소, 패널 나누기/합치기, 검색에 닿을 길이 없다. 지금 오른쪽 메뉴는 항목 메뉴(열기·클립보드·복사/이동·이름 바꾸기·삭제)와 빈 곳 메뉴(붙여넣기·새 폴더·정렬 기준·숨김 항목 보이기·새로 고침·경로 복사)뿐이다. 게다가 목록이 자세히 보기를 꽉 채우면(행 수 × 행 높이 ≥ 목록 높이) 목록 아래 빈 곳의 높이가 0이라 맨 끝까지 넘겨도 빈 곳 메뉴를 마우스로 열 수 없다.

## Solution

1. **빈 곳 메뉴에 명령 여섯 개를 더한다**(`menu::build`, 순수 함수 그대로). 새 고정 순서:

   ```
   뒤로, 앞으로, 위로,
   ─
   붙여넣기, 새 폴더, 실행취소,
   ─
   정렬 기준 ▶, 숨김 항목 보이기, 새로 고침,
   ─
   검색, 패널 나누기/합치기,
   ─
   경로 복사
   ```

   기존 항목(붙여넣기·새 폴더·정렬 기준·숨김 항목 보이기·새로 고침·경로 복사)은 모두 남고 서로의 차례도 그대로다. 항목 메뉴는 바꾸지 않는다(검색 결과 위치의 「폴더에서 보기」 머리도 그대로).
2. **켜짐 규칙은 명령의 규칙과 같다.** 뒤로·앞으로·위로·실행취소는 지금 `menu::command`의 `_ => true`에 빠지므로, 이미 가진 상태를 `MenuContext`의 새 필드 네 개로 받아 켜고 끈다. 검색·패널 나누기/합치기는 명령처럼 늘 켜진다. 할 수 없는 항목은 숨기지 않고 끈다(자리 고정).
3. **실행은 기존 길이다.** 앱의 오른쪽 메뉴는 이미 고른 줄의 `CommandId`를 `View::execute` → `App::execute`로 넘기므로 새 줄도 같은 길로 실행된다. 앱은 새 명령의 이름(`text::command_name`, 이미 있음)과 첫 기본 단축키 글자(`text::shortcut_text`, 이미 있음)를 그대로 보인다. 새 화면 문장은 없다.
4. **빈 곳을 늘 남긴다(앱).** 자세히 보기 목록의 내용 높이를 「행 수 × 행 높이 + 행 높이 하나」로 하여, 끝까지 넘기면 마지막 행 아래에 행 높이 하나만큼 빈 곳이 늘 보이게 한다. 그 빈 곳은 기존 「목록 아래 빈 곳」 `TouchArea`(`ui/app.slint`)가 그대로 받으므로 오른쪽 클릭은 빈 곳 메뉴, 왼쪽 끌기는 사각형 선택, 끌어 놓기는 패널 자리(`file_drop::Place::Panel`)다.
5. 사람 확인 목록 `docs/context-menu-checklist.md`의 빈 곳 메뉴 항목을 새 순서로 고치고 새 항목과 꽉 찬 목록의 빈 곳 확인을 더한다.

## User Stories

- 마우스만 쓰는 사람으로서, 빈 곳을 오른쪽 클릭해 「뒤로」「앞으로」「위로」를 눌러 폴더 사이를 오가고 싶다.
- 마우스만 쓰는 사람으로서, 잘못 지운 파일을 빈 곳 메뉴의 「실행취소」로 되돌리고 싶다.
- 마우스만 쓰는 사람으로서, 빈 곳 메뉴에서 「패널 나누기/합치기」와 「검색」을 열고 싶다.
- 단축키를 배우고 싶은 사람으로서, 메뉴 줄 옆의 단축키 글자(Alt+←, Ctrl+Z …)를 보고 다음에는 키로 하고 싶다.
- 항목이 수천 개인 폴더에서도, 목록 끝까지 넘겨 빈 곳을 오른쪽 클릭할 수 있어야 한다.

## Contract

### `facet_core::menu::MenuContext` (필드 넷 추가)

```rust
pub struct MenuContext {
    // … 기존 필드(kind, selected_count, has_cursor, cursor_is_folder, single_folder_selected,
    //    location_is_folder, location_is_results, panel_count, opposite_is_folder,
    //    clipboard_has_files, sort, show_hidden) 그대로 …
    /// 활성 탭에 뒤로 갈 기록이 있는가(`Tab::can_go_back`).
    pub can_go_back: bool,
    /// 활성 탭에 앞으로 갈 기록이 있는가(`Tab::can_go_forward`).
    pub can_go_forward: bool,
    /// 활성 탭에 상위 위치가 있는가(`Tab::can_go_up`, 드라이브 루트에서 거짓).
    pub can_go_up: bool,
    /// 실행취소 대상이 있는가(`History::undo_target().is_some()`: 기록이 없거나 실행취소·다시 실행이 도는 중이면 거짓.
    /// 마지막 기록이 되돌릴 수 없는 작업이어도 참이다).
    pub can_undo: bool,
}
```

- 여전히 `Debug, Clone, Copy, PartialEq, Eq`이고, 만들 때 목록을 훑지 않는다.
- `App::menu_context(kind)`가 활성 창의 활성 패널의 활성 탭과 앱의 작업 기록에서 네 값을 채운다(`tab.can_go_back()`, `tab.can_go_forward()`, `tab.can_go_up()`, `self.history().undo_target().is_some()`).

### `facet_core::menu::build` (빈 곳 메뉴 순서 변경)

`MenuKind::Background`의 `Menu::entries`는 맥락과 상관없이 늘 이 16줄이다(번호는 `entries`의 자리):

| 자리 | 줄 |
|---|---|
| 0 | `MenuEntry::Command` `CommandId::Back` |
| 1 | `CommandId::Forward` |
| 2 | `CommandId::Up` |
| 3 | `MenuEntry::Separator` |
| 4 | `CommandId::Paste` |
| 5 | `CommandId::NewFolder` |
| 6 | `CommandId::Undo` |
| 7 | `MenuEntry::Separator` |
| 8 | `MenuEntry::Submenu` `SubmenuId::SortBy`(안은 그대로: 이름·수정한 날짜·유형·크기) |
| 9 | `CommandId::ToggleHidden` |
| 10 | `CommandId::Refresh` |
| 11 | `MenuEntry::Separator` |
| 12 | `CommandId::Search` |
| 13 | `CommandId::TogglePanelSplit` |
| 14 | `MenuEntry::Separator` |
| 15 | `CommandId::CopyPath` |

새 명령 줄의 `enabled`(`MenuCommand::enabled`):

- `Back` = `can_go_back`, `Forward` = `can_go_forward`, `Up` = `can_go_up`, `Undo` = `can_undo`.
- `Search`, `TogglePanelSplit` = 늘 `true`(명령 규칙과 같음: `CommandId::is_enabled`가 늘 참이고 창이 있으면 `App::can_execute`도 참).
- 새 줄의 `mark`는 모두 `Mark::None`.

기존 줄의 켜짐·표시 규칙, 항목 메뉴(`MenuKind::Item`)의 순서·켜짐, 검색 결과 위치의 머리 줄은 바뀌지 않는다.

### 기존 테스트·코드의 맞춤(Dev 몫)

- `crates/facet-core/tests/menu.rs`의 `context` 도우미에 새 필드를 채우고, 빈 곳 메뉴 순서를 단언하는 `background_menu_has_the_fixed_order_and_a_sort_submenu`와 정렬 하위 메뉴를 `entries[3]`으로 찾는 곳을 새 순서(자리 8)로 고친다.
- `crates/facet-app/src/context_menu.rs` 단위 테스트의 `context` 도우미와 빈 곳 메뉴를 쓰는 테스트를 새 필드·새 순서에 맞춘다.
- `facet-bench menu`(`measure_menu`)는 `App::context_menu`를 그대로 재며 기준 `MENU_TARGET`(1ms)·상한 `MENU_LIMIT`(10ms)는 그대로다.

### 앱(`crates/facet-app`)

- 오른쪽 메뉴 그리기(`ContextMenuView`)·키 처리(`src/context_menu.rs`)·실행(`View::execute`)은 바꾸지 않는다. 메뉴 높이는 줄 수에서 계산하므로 16줄도 `place`가 창 안에 둔다.
- 자세히 보기 목록(`ui/app.slint`)의 넘김 내용 높이에 행 높이 하나를 더한다(`rows.length * row-height + row-height`). 「목록 아래 빈 곳」 `TouchArea`는 그대로이고, 끝까지 넘기면 높이가 행 높이 하나 이상이 된다. 화면 문장은 더하지 않는다(ADR-0007).
- `docs/context-menu-checklist.md`를 아래 「Manual Checks」대로 고친다.

## Acceptance Criteria

- AC-1: 빈 곳 메뉴(`menu::build`, `MenuKind::Background`)에 `CommandId::Back`·`Forward`·`Up` 줄이 있고, 앱의 빈 곳 메뉴에서 그 줄의 명령을 `App::execute`로 실행하면 활성 탭이 뒤로·앞으로·상위 위치로 간다. (R-0001.1)
- AC-2: 빈 곳 메뉴에 `CommandId::Undo` 줄이 있고, 작업 기록이 있으면 켜지며 그 줄의 명령을 실행하면 실행취소 길(`App::execute(CommandId::Undo)`, 되돌릴 수 없는 기록이면 `Execution::UndoRefused`)을 탄다. (R-0001.2)
- AC-3: 빈 곳 메뉴에 `CommandId::TogglePanelSplit` 줄이 있고, 그 줄의 명령을 실행하면 패널이 하나일 때 둘로 나뉘고 둘일 때 하나로 합쳐진다. (R-0001.3)
- AC-4: 빈 곳 메뉴에 `CommandId::Search` 줄이 늘 켜져 있고, 그 줄의 명령을 실행하면 `Execution::Search`가 돌아온다. (R-0001.4)
- AC-5: 빈 곳 메뉴에 `CommandId::Paste`·`NewFolder` 줄이 그대로 있고 기존 빈 곳 항목(붙여넣기·새 폴더·정렬 기준·숨김 항목 보이기·새로 고침·경로 복사)이 모두 기존 차례대로 남으며, 새 폴더 줄의 명령을 실행하면 `Execution::NewFolder`가 돌아온다. (R-0001.5)
- AC-6: 빈 곳 메뉴는 맥락(선택 수·패널 수·클립보드·위치 종류·`can_go_back`·`can_go_forward`·`can_go_up`·`can_undo`)과 상관없이 Contract의 16줄 순서이고, 새 줄은 할 수 없을 때 빠지지 않고 `enabled == false`로 남는다. (R-0001.6)
- AC-7: 앱의 빈 곳 메뉴(`App::context_menu(MenuKind::Background)`)의 모든 명령 줄의 `enabled`가 `App::can_execute(그 명령)`과 같다(새 기록 없음·뒤로 갈 기록 있음·앞으로 갈 기록 있음·실행취소 대상 있음·패널 둘 상태에서). (R-0001.6)
- AC-8: 새로 더한 명령 여섯(`Back`·`Forward`·`Up`·`Undo`·`TogglePanelSplit`·`Search`)은 모두 기본 단축키가 하나 이상 있다(`CommandId::shortcuts`가 비지 않음). (R-0001.7)
- AC-9: 오른쪽 메뉴 줄 옆에 그 명령의 첫 기본 단축키 글자가 보인다: 뒤로 Backspace, 앞으로 Alt+→, 위로 Alt+↑, 실행취소 Ctrl+Z, 검색 Ctrl+F, 패널 나누기/합치기 Ctrl+\(팔레트와 같은 글자). (R-0001.7)
- AC-10: 새 줄의 명령을 `App::execute`로 실행한 결과는 그 명령의 첫 기본 단축키를 `find_by_shortcut`으로 찾아 실행한 결과와 같다(`find_by_shortcut(첫 단축키) == Some(그 명령)`이고, 같은 상태의 두 앱에서 돌아온 `Execution`과 활성 탭 위치·패널 수가 같다). (R-0001.8)
- AC-11: 목록이 자세히 보기를 꽉 채운 폴더에서도 끝까지 넘기면 마지막 행 아래에 빈 곳이 보이고, 거기를 오른쪽 클릭하면 빈 곳 메뉴가 열려 새 줄을 마우스로 누르면 실행된다. (R-0001.1, R-0001.2, R-0001.3, R-0001.4, R-0001.5)
- AC-12: 선택 1만 개에서 빈 곳 메뉴·항목 메뉴 구성은 `facet-bench menu`의 CI 상한 `MENU_LIMIT`(10ms) 안이다. (R-0001.6)

## E2E Tests

- `acceptance/layout.rs` → `crates/facet-core/tests/spec_mouse_reachable_commands_layout.rs`: AC-1, AC-2, AC-3, AC-4, AC-5, AC-6, AC-8 (순수 `menu::build`: 16줄 순서, 기존 항목 차례, 맥락을 바꿔도 같은 순서, 새 필드에 따른 켜짐·꺼짐, 항목 메뉴 불변, 새 명령의 기본 단축키).
- `acceptance/app.rs` → `crates/facet-core/tests/spec_mouse_reachable_commands_app.rs`: AC-1, AC-2, AC-3, AC-4, AC-5, AC-7, AC-10 (임시 폴더의 `App`: 메뉴 줄로 실행한 뒤로·앞으로·위로·패널 나누기/합치기·검색·새 폴더·실행취소, 메뉴 켜짐과 `App::can_execute`의 일치, 메뉴 실행과 단축키 실행의 결과 비교).
- AC-12는 기존 벤치마크 `facet-bench menu`(`cargo xtask ci`의 오른쪽 메뉴 구성 벤치마크 단계)가 지킨다.

## Manual Checks

`docs/context-menu-checklist.md`를 고쳐 다음을 Windows 기계에서 확인한다.

- AC-9: 빈 곳 메뉴의 뒤로·앞으로·위로·실행취소·검색·패널 나누기/합치기 줄 옆에 Backspace, Alt+→, Alt+↑, Ctrl+Z, Ctrl+F, Ctrl+\ 가 보이고 명령 팔레트의 첫 단축키 글자와 같다.
- AC-11: 10만 항목 시험 폴더(`cargo xtask test-folder`)를 열고 목록 끝까지 넘기면 마지막 행 아래 한 행 높이의 빈 곳이 보이고, 그곳 오른쪽 클릭은 빈 곳 메뉴, 왼쪽 끌기는 사각형 선택, 다른 앱에서 끌어 온 파일을 놓으면 현재 폴더로 들어간다.
- 「열기」 절의 빈 곳 메뉴 항목 목록을 새 순서(뒤로, 앞으로, 위로, | 붙여넣기, 새 폴더, 실행취소, | 정렬 기준 ▶, 숨김 항목 보이기, 새로 고침, | 검색, 패널 나누기/합치기, | 경로 복사)로 바꾼다.
- 빈 곳 메뉴에서 뒤로·앞으로·위로를 누르면 이동하고, 기록이 없으면(새 탭) 뒤로·앞으로가 흐리고 눌러도 아무 일이 없으며, 드라이브 루트에서 위로가 흐리다.
- 파일을 지운 뒤 빈 곳 메뉴의 실행취소를 누르면 상태 줄에 「〈작업 이름〉 실행취소」가 보이고 파일이 돌아온다. 작업 기록이 없으면 실행취소가 흐리다.
- 빈 곳 메뉴의 패널 나누기/합치기를 누르면 패널이 나뉘고(오른쪽 패널이 활성), 그 패널의 빈 곳에서 다시 누르면 합쳐진다.
- 빈 곳 메뉴의 검색을 누르면 활성 패널의 검색창이 열리고 포커스를 받는다.
- 16줄 메뉴가 창 아래 끝 가까이에서 열려도 창 밖으로 넘치지 않는다. ↑↓가 꺼진 줄과 구분선을 건너뛴다.
- 검색 결과 위치의 빈 곳 메뉴에서도 같은 순서이고, 뒤로를 누르면 검색한 폴더로 돌아간다.

## Decided by the node

- 새 줄은 빈 곳 메뉴에만 둔다: 탭·앱 상태를 대상으로 하는 명령이라 항목 선택과 무관하고, 항목 메뉴를 늘리면 셸 항목이 붙을 자리(ADR-0006)와 항목 작업이 묻힌다.
- 꽉 찬 목록에서도 빈 곳에 닿도록 목록 끝에 행 높이 하나의 빈 곳을 늘 둔다: 그렇지 않으면 빈 곳 메뉴가 꽉 찬 폴더에서 마우스로 닿지 않아 R-0001.1~5를 못 지키고, 항목 메뉴에 같은 줄을 반복하는 것보다 작은 변경이다. 높이는 화면 상수라 나중에 싸게 바꾼다.
- 순서는 탐색기 빈 곳 메뉴의 흐름을 따른다: 이동(뒤로·앞으로·위로)을 맨 위, 만들기·붙여넣기·실행취소를 한 묶음(탐색기도 붙여넣기 옆에 실행취소), 보기 묶음(정렬·숨김·새로 고침), 도구 묶음(검색·패널), 경로 복사를 맨 아래(기존과 같음).
- 다시 실행(`Redo`)·필터·새 탭 등은 더하지 않는다: 요구 항목에 없고 메뉴 길이를 늘린다.
- 「패널 나누기/합치기」에 켜짐 표시(✓)를 붙이지 않는다: 이름이 이미 두 방향을 담고 있고, 표시를 위해 맥락 필드를 더 늘릴 까닭이 없다.
- 켜짐 값은 `MenuContext`의 bool 필드로 받는다: `menu::build`를 앱 상태에 묶지 않는 순수 함수로 두는 기존 방식(ADR-0006, 벤치마크)과 같다.

## Out of Scope

- 상시 도구 모음·리본·메뉴 막대, 탭 줄·머리글·주소창의 오른쪽 메뉴.
- 항목 메뉴의 변경, 셸 항목.
- 새 명령, 기본 단축키 변경, 새 화면 문장.
- 그리드 보기, 터치 조작.
