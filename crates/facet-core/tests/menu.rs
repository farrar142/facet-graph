//! 오른쪽 메뉴의 구성과 오른쪽 클릭의 선택 규칙을 공개 인터페이스로 확인한다.
//!
//! - 항목 순서와 켜짐·꺼짐(파일 하나·폴더 하나·여럿·빈 곳, 패널 하나·둘, 클립보드 있음·없음), 순서가 맥락과 상관없이 같음
//! - 메뉴 항목이 앱의 명령 실행 가능 여부와 같음
//! - 오른쪽 클릭이 선택 밖 항목이면 그 항목 하나로, 선택 안이면 그대로, 빈 곳이면 선택을 풂

use std::fs;
use std::time::Duration;

use facet_core::Location;
use facet_core::command::CommandId;
use facet_core::details::{Column, Direction, Sort};
use facet_core::menu::{Mark, Menu, MenuContext, MenuEntry, MenuKind, SubmenuId, build};
use facet_core::tab::ReadOutcome;
use facet_core::window::{App, Execution};
use tempfile::TempDir;

fn context(kind: MenuKind) -> MenuContext {
    MenuContext {
        kind,
        selected_count: 1,
        has_cursor: true,
        cursor_is_folder: false,
        single_folder_selected: false,
        location_is_folder: true,
        location_is_results: false,
        panel_count: 1,
        opposite_is_folder: false,
        clipboard_has_files: false,
        sort: Sort {
            column: Column::Name,
            direction: Direction::Ascending,
        },
        show_hidden: false,
        can_go_back: false,
        can_go_forward: false,
        can_go_up: false,
        can_undo: false,
    }
}

/// 명령 항목의 ID를 구분선·하위 메뉴는 `None`으로 펼친다.
fn ids(menu: &Menu) -> Vec<Option<CommandId>> {
    menu.entries
        .iter()
        .map(|entry| match entry {
            MenuEntry::Command(c) => Some(c.id),
            MenuEntry::Submenu(_) | MenuEntry::Separator => None,
        })
        .collect()
}

fn enabled(menu: &Menu, id: CommandId) -> bool {
    menu.commands().find(|c| c.id == id).unwrap().enabled
}

fn mark(menu: &Menu, id: CommandId) -> Mark {
    menu.commands().find(|c| c.id == id).unwrap().mark
}

#[test]
fn item_menu_has_the_fixed_order() {
    use CommandId::*;
    let menu = build(&context(MenuKind::Item));
    assert_eq!(
        ids(&menu),
        vec![
            Some(Open),
            Some(OpenInNewTab),
            None,
            Some(Cut),
            Some(Copy),
            Some(Paste),
            Some(CopyPath),
            None,
            Some(CopyToOtherPanel),
            Some(MoveToOtherPanel),
            Some(CopyTo),
            Some(MoveTo),
            None,
            Some(Rename),
            Some(BatchRename),
            Some(Delete),
        ]
    );
}

#[test]
fn background_menu_has_the_fixed_order_and_a_sort_submenu() {
    use CommandId::*;
    let menu = build(&context(MenuKind::Background));
    assert_eq!(
        ids(&menu),
        vec![
            Some(Back),
            Some(Forward),
            Some(Up),
            None,
            Some(Paste),
            Some(NewFolder),
            Some(Undo),
            None,
            None, // 정렬 기준 하위 메뉴
            Some(ToggleHidden),
            Some(Refresh),
            None,
            Some(Search),
            Some(TogglePanelSplit),
            None,
            Some(CopyPath),
        ]
    );
    let MenuEntry::Submenu(sort) = &menu.entries[8] else {
        panic!("정렬 하위 메뉴가 아니다");
    };
    assert_eq!(sort.id, SubmenuId::SortBy);
    assert!(sort.enabled);
    let sorts: Vec<_> = sort.entries.iter().map(|c| c.id).collect();
    assert_eq!(sorts, [SortByName, SortByModified, SortByType, SortBySize]);
}

#[test]
fn order_is_the_same_whatever_the_context() {
    for kind in [MenuKind::Item, MenuKind::Background] {
        let mut orders = Vec::new();
        for selected_count in [0, 1, 5] {
            for panel_count in [1, 2] {
                for clipboard in [false, true] {
                    for folder in [false, true] {
                        let mut c = context(kind);
                        c.selected_count = selected_count;
                        c.has_cursor = selected_count > 0;
                        c.panel_count = panel_count;
                        c.clipboard_has_files = clipboard;
                        c.location_is_folder = folder;
                        orders.push(ids(&build(&c)));
                    }
                }
            }
        }
        assert!(orders.windows(2).all(|pair| pair[0] == pair[1]));
    }
}

#[test]
fn file_selected() {
    let mut c = context(MenuKind::Item);
    c.clipboard_has_files = true;
    let menu = build(&c);
    assert!(enabled(&menu, CommandId::Open));
    assert!(
        !enabled(&menu, CommandId::OpenInNewTab),
        "파일은 새 탭에서 열 수 없다"
    );
    assert!(enabled(&menu, CommandId::Cut));
    assert!(enabled(&menu, CommandId::Copy));
    assert!(
        !enabled(&menu, CommandId::Paste),
        "붙여넣기는 폴더 하나를 골랐을 때"
    );
    assert!(enabled(&menu, CommandId::CopyPath));
    assert!(enabled(&menu, CommandId::CopyTo));
    assert!(enabled(&menu, CommandId::Rename));
    assert!(enabled(&menu, CommandId::Delete));
}

#[test]
fn folder_selected() {
    let mut c = context(MenuKind::Item);
    c.cursor_is_folder = true;
    c.single_folder_selected = true;
    assert!(enabled(&build(&c), CommandId::OpenInNewTab));
    assert!(
        !enabled(&build(&c), CommandId::Paste),
        "클립보드에 파일이 없다"
    );
    c.clipboard_has_files = true;
    assert!(enabled(&build(&c), CommandId::Paste));
    c.location_is_folder = false;
    let menu = build(&c);
    assert!(!enabled(&menu, CommandId::Paste));
    assert!(!enabled(&menu, CommandId::Delete));
    assert!(!enabled(&menu, CommandId::Rename));
    assert!(!enabled(&menu, CommandId::Cut));
    assert!(enabled(&menu, CommandId::Open));
}

#[test]
fn many_selected() {
    let mut c = context(MenuKind::Item);
    c.selected_count = 10_000;
    c.clipboard_has_files = true;
    let menu = build(&c);
    assert!(enabled(&menu, CommandId::Cut));
    assert!(enabled(&menu, CommandId::Delete));
    assert!(!enabled(&menu, CommandId::Paste), "폴더 하나가 아니다");
}

#[test]
fn other_panel_items_follow_the_panels() {
    let mut c = context(MenuKind::Item);
    c.panel_count = 1;
    let menu = build(&c);
    assert!(
        enabled(&menu, CommandId::CopyToOtherPanel),
        "패널이 하나면 선택 창을 연다"
    );
    assert!(enabled(&menu, CommandId::MoveToOtherPanel));
    c.panel_count = 2;
    c.opposite_is_folder = false;
    let menu = build(&c);
    assert!(!enabled(&menu, CommandId::CopyToOtherPanel));
    assert!(!enabled(&menu, CommandId::MoveToOtherPanel));
    assert!(
        enabled(&menu, CommandId::CopyTo),
        "위치를 골라 복사는 패널 수와 상관없다"
    );
    c.opposite_is_folder = true;
    assert!(enabled(&build(&c), CommandId::CopyToOtherPanel));
}

#[test]
fn background_menu_enablement_and_marks() {
    let mut c = context(MenuKind::Background);
    c.selected_count = 0;
    c.has_cursor = false;
    let menu = build(&c);
    assert!(!enabled(&menu, CommandId::Paste));
    assert!(enabled(&menu, CommandId::NewFolder));
    assert!(enabled(&menu, CommandId::Refresh));
    assert!(enabled(&menu, CommandId::CopyPath));
    assert_eq!(mark(&menu, CommandId::ToggleHidden), Mark::None);

    c.clipboard_has_files = true;
    c.show_hidden = true;
    c.sort = Sort {
        column: Column::Size,
        direction: Direction::Descending,
    };
    let menu = build(&c);
    assert!(enabled(&menu, CommandId::Paste));
    assert_eq!(mark(&menu, CommandId::ToggleHidden), Mark::Checked);
    let MenuEntry::Submenu(sort) = &menu.entries[8] else {
        panic!("정렬 하위 메뉴가 아니다");
    };
    let marks: Vec<_> = sort.entries.iter().map(|c| c.mark).collect();
    assert_eq!(
        marks,
        [
            Mark::None,
            Mark::None,
            Mark::None,
            Mark::Sorted(Direction::Descending)
        ]
    );

    c.location_is_folder = false;
    let menu = build(&c);
    assert!(!enabled(&menu, CommandId::Paste));
    assert!(!enabled(&menu, CommandId::NewFolder));
    assert!(!enabled(&menu, CommandId::CopyPath));
}

// ── 앱 상태와 오른쪽 클릭 ─────────────────────────────────────────────────────────────────

fn app_with_files() -> (TempDir, App) {
    let dir = TempDir::new().unwrap();
    fs::create_dir(dir.path().join("a폴더")).unwrap();
    fs::write(dir.path().join("b.txt"), b"b").unwrap();
    fs::write(dir.path().join("c.txt"), b"c").unwrap();
    let mut app = App::new(Some(Location::file_system(dir.path())));
    let (_, outcome) = app.wait(Duration::from_secs(20)).expect("첫 읽기");
    assert_eq!(outcome, ReadOutcome::Committed);
    (dir, app)
}

fn selected(app: &App) -> Vec<String> {
    app.active_tab()
        .selected_items()
        .map(|i| i.name.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn right_click_outside_the_selection_selects_that_item_only() {
    let (_dir, mut app) = app_with_files();
    app.active_tab_mut().select_only(1);
    app.active_tab_mut().toggle_select(2);
    assert_eq!(selected(&app), ["b.txt", "c.txt"]);
    // 선택 밖(0번 폴더): 그 항목 하나만 선택하고 커서도 거기 둔다.
    assert_eq!(app.context_click(0, Some(0)), Some(MenuKind::Item));
    assert_eq!(selected(&app), ["a폴더"]);
    assert_eq!(app.active_tab().cursor(), Some(0));
}

#[test]
fn right_click_inside_the_selection_keeps_it() {
    let (_dir, mut app) = app_with_files();
    app.active_tab_mut().select_only(1);
    app.active_tab_mut().toggle_select(2);
    assert_eq!(app.context_click(0, Some(1)), Some(MenuKind::Item));
    assert_eq!(selected(&app), ["b.txt", "c.txt"]);
    assert_eq!(app.active_tab().cursor(), Some(1));
}

#[test]
fn right_click_on_blank_clears_the_selection_and_opens_the_background_menu() {
    let (_dir, mut app) = app_with_files();
    app.active_tab_mut().select_only(1);
    assert_eq!(app.context_click(0, None), Some(MenuKind::Background));
    assert!(selected(&app).is_empty());
    // 목록 밖 번호도 빈 곳이다. 없는 패널은 열지 않는다.
    assert_eq!(app.context_click(0, Some(99)), Some(MenuKind::Background));
    assert_eq!(app.context_click(1, Some(0)), None);
}

#[test]
fn right_click_makes_that_panel_active() {
    let (_dir, mut app) = app_with_files();
    assert_eq!(app.execute(CommandId::TogglePanelSplit), Execution::Done);
    assert_eq!(app.active_window().active_panel_index(), 1);
    assert_eq!(app.context_click(0, None), Some(MenuKind::Background));
    assert_eq!(app.active_window().active_panel_index(), 0);
}

#[test]
fn app_menu_agrees_with_command_enablement() {
    let (_dir, mut app) = app_with_files();
    app.context_click(0, Some(1));
    let menu = app.context_menu(MenuKind::Item);
    for command in menu.commands() {
        // 붙여넣기는 항목 메뉴에서 폴더 하나를 골랐을 때만이라 명령보다 좁다.
        if command.id == CommandId::Paste {
            assert!(!command.enabled);
            continue;
        }
        assert_eq!(
            command.enabled,
            app.can_execute(command.id),
            "{:?}",
            command.id
        );
    }
    app.context_click(0, None);
    let menu = app.context_menu(MenuKind::Background);
    for command in menu.commands() {
        assert_eq!(
            command.enabled,
            app.can_execute(command.id),
            "{:?}",
            command.id
        );
    }
}
