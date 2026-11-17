// target: crates/facet-core/tests/spec_mouse_reachable_commands_layout.rs
// AC: AC-1, AC-2, AC-3, AC-4, AC-5, AC-6, AC-8
//! 빈 곳 메뉴에 마우스로 닿는 명령(뒤로·앞으로·위로·실행취소·검색·패널 나누기/합치기)을 더한 순수 구성
//! (`menu::build`)을 확인한다: 16줄 고정 순서, 기존 항목의 차례, 맥락과 상관없는 순서, 새 필드에 따른 켜짐·꺼짐,
//! 항목 메뉴 불변, 새 명령의 기본 단축키.

use facet_core::command::CommandId;
use facet_core::details::{Column, Direction, Sort};
use facet_core::menu::{Mark, Menu, MenuContext, MenuEntry, MenuKind, SubmenuId, build};

/// 새로 더한 명령 여섯.
const NEW_COMMANDS: [CommandId; 6] = [
    CommandId::Back,
    CommandId::Forward,
    CommandId::Up,
    CommandId::Undo,
    CommandId::Search,
    CommandId::TogglePanelSplit,
];

fn context(kind: MenuKind) -> MenuContext {
    MenuContext {
        kind,
        selected_count: 0,
        has_cursor: false,
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
        can_go_back: true,
        can_go_forward: true,
        can_go_up: true,
        can_undo: true,
    }
}

/// 메뉴의 한 줄을 견주기 쉬운 모양으로 펼친다.
#[derive(Debug, PartialEq, Eq)]
enum Row {
    Command(CommandId),
    Submenu(SubmenuId),
    Separator,
}

fn rows(menu: &Menu) -> Vec<Row> {
    menu.entries
        .iter()
        .map(|entry| match entry {
            MenuEntry::Command(c) => Row::Command(c.id),
            MenuEntry::Submenu(s) => Row::Submenu(s.id),
            MenuEntry::Separator => Row::Separator,
        })
        .collect()
}

fn enabled(menu: &Menu, id: CommandId) -> bool {
    menu.commands()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("{id:?} 줄이 없다"))
        .enabled
}

fn background_layout() -> Vec<Row> {
    use CommandId::*;
    vec![
        Row::Command(Back),
        Row::Command(Forward),
        Row::Command(Up),
        Row::Separator,
        Row::Command(Paste),
        Row::Command(NewFolder),
        Row::Command(Undo),
        Row::Separator,
        Row::Submenu(SubmenuId::SortBy),
        Row::Command(ToggleHidden),
        Row::Command(Refresh),
        Row::Separator,
        Row::Command(Search),
        Row::Command(TogglePanelSplit),
        Row::Separator,
        Row::Command(CopyPath),
    ]
}

#[test]
fn background_menu_has_the_new_fixed_order() {
    let menu = build(&context(MenuKind::Background));
    assert_eq!(menu.kind, MenuKind::Background);
    assert_eq!(rows(&menu), background_layout());
    let MenuEntry::Submenu(sort) = &menu.entries[8] else {
        panic!("자리 8은 정렬 하위 메뉴여야 한다");
    };
    let sorts: Vec<_> = sort.entries.iter().map(|c| c.id).collect();
    assert_eq!(
        sorts,
        [
            CommandId::SortByName,
            CommandId::SortByModified,
            CommandId::SortByType,
            CommandId::SortBySize
        ]
    );
}

#[test]
fn existing_background_items_keep_their_relative_order() {
    use CommandId::*;
    let menu = build(&context(MenuKind::Background));
    let position = |row: Row| rows(&menu).iter().position(|r| *r == row).unwrap();
    let existing = [
        position(Row::Command(Paste)),
        position(Row::Command(NewFolder)),
        position(Row::Submenu(SubmenuId::SortBy)),
        position(Row::Command(ToggleHidden)),
        position(Row::Command(Refresh)),
        position(Row::Command(CopyPath)),
    ];
    assert!(
        existing.windows(2).all(|pair| pair[0] < pair[1]),
        "기존 빈 곳 항목은 모두 남고 차례가 같다: {existing:?}"
    );
}

#[test]
fn background_order_is_the_same_whatever_the_context() {
    let mut orders = Vec::new();
    for selected_count in [0, 1, 10_000] {
        for panel_count in [1, 2] {
            for clipboard in [false, true] {
                for (folder, results) in [(true, false), (false, false), (false, true)] {
                    for flags in 0u8..16 {
                        let mut c = context(MenuKind::Background);
                        c.selected_count = selected_count;
                        c.has_cursor = selected_count > 0;
                        c.panel_count = panel_count;
                        c.opposite_is_folder = panel_count == 2;
                        c.clipboard_has_files = clipboard;
                        c.location_is_folder = folder;
                        c.location_is_results = results;
                        c.can_go_back = flags & 1 != 0;
                        c.can_go_forward = flags & 2 != 0;
                        c.can_go_up = flags & 4 != 0;
                        c.can_undo = flags & 8 != 0;
                        orders.push(rows(&build(&c)));
                    }
                }
            }
        }
    }
    assert!(orders.iter().all(|order| *order == background_layout()));
}

#[test]
fn new_rows_are_disabled_not_hidden_when_they_cannot_run() {
    let mut c = context(MenuKind::Background);
    c.can_go_back = false;
    c.can_go_forward = false;
    c.can_go_up = false;
    c.can_undo = false;
    let menu = build(&c);
    assert_eq!(rows(&menu), background_layout(), "꺼져도 자리는 그대로");
    assert!(!enabled(&menu, CommandId::Back));
    assert!(!enabled(&menu, CommandId::Forward));
    assert!(!enabled(&menu, CommandId::Up));
    assert!(!enabled(&menu, CommandId::Undo));
    assert!(enabled(&menu, CommandId::Search), "검색은 늘 켜짐");
    assert!(
        enabled(&menu, CommandId::TogglePanelSplit),
        "패널 나누기/합치기는 늘 켜짐"
    );
}

/// 맥락의 새 필드 하나를 끄는 함수.
type TurnOff = fn(&mut MenuContext);

#[test]
fn each_flag_drives_only_its_own_row() {
    let cases: [(TurnOff, CommandId); 4] = [
        (|c| c.can_go_back = false, CommandId::Back),
        (|c| c.can_go_forward = false, CommandId::Forward),
        (|c| c.can_go_up = false, CommandId::Up),
        (|c| c.can_undo = false, CommandId::Undo),
    ];
    for (turn_off, id) in cases {
        let mut c = context(MenuKind::Background);
        turn_off(&mut c);
        let menu = build(&c);
        for other in NEW_COMMANDS {
            assert_eq!(
                enabled(&menu, other),
                other != id,
                "{id:?}을 끄면 {other:?}의 켜짐"
            );
        }
    }
}

#[test]
fn new_rows_work_in_search_results_and_without_a_folder() {
    // 결과 위치·실제 폴더 아닌 위치에서도 이동·실행취소·검색·패널은 위치와 상관없는 규칙을 따른다.
    for (folder, results) in [(false, true), (false, false)] {
        let mut c = context(MenuKind::Background);
        c.location_is_folder = folder;
        c.location_is_results = results;
        let menu = build(&c);
        for id in NEW_COMMANDS {
            assert!(enabled(&menu, id), "{id:?}");
        }
        assert!(!enabled(&menu, CommandId::NewFolder));
        assert!(!enabled(&menu, CommandId::Paste));
    }
}

#[test]
fn new_rows_have_no_mark() {
    let mut c = context(MenuKind::Background);
    c.panel_count = 2;
    c.show_hidden = true;
    let menu = build(&c);
    for id in NEW_COMMANDS {
        let row = menu.commands().find(|r| r.id == id).unwrap();
        assert_eq!(row.mark, Mark::None, "{id:?}");
    }
}

#[test]
fn item_menu_is_unchanged() {
    use CommandId::*;
    let mut c = context(MenuKind::Item);
    c.selected_count = 1;
    c.has_cursor = true;
    let expected = vec![
        Row::Command(Open),
        Row::Command(OpenInNewTab),
        Row::Separator,
        Row::Command(Cut),
        Row::Command(Copy),
        Row::Command(Paste),
        Row::Command(CopyPath),
        Row::Separator,
        Row::Command(CopyToOtherPanel),
        Row::Command(MoveToOtherPanel),
        Row::Command(CopyTo),
        Row::Command(MoveTo),
        Row::Separator,
        Row::Command(Rename),
        Row::Command(Delete),
    ];
    assert_eq!(rows(&build(&c)), expected);
    c.can_go_back = false;
    c.can_undo = false;
    assert_eq!(
        rows(&build(&c)),
        expected,
        "새 필드는 항목 메뉴를 바꾸지 않는다"
    );
    c.location_is_results = true;
    let menu = build(&c);
    assert_eq!(rows(&menu)[0], Row::Command(ShowInFolder));
    assert_eq!(rows(&menu)[1], Row::Separator);
    assert_eq!(rows(&menu)[2..], expected[..]);
}

#[test]
fn new_commands_all_have_a_default_shortcut() {
    for id in NEW_COMMANDS {
        assert!(
            !id.shortcuts().is_empty(),
            "{id:?}의 첫 기본 단축키를 메뉴 옆에 보여야 한다"
        );
    }
}
