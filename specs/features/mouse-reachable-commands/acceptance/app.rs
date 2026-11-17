// target: crates/facet-core/tests/spec_mouse_reachable_commands_app.rs
// AC: AC-1, AC-2, AC-3, AC-4, AC-5, AC-7, AC-10
//! 앱의 빈 곳 메뉴에서 새 줄(뒤로·앞으로·위로·실행취소·검색·패널 나누기/합치기)과 새 폴더 줄을 골라 기존 명령 실행 길
//! (`App::execute`)로 실행한 결과, 메뉴 켜짐과 `App::can_execute`의 일치, 메뉴로 실행한 결과와 첫 기본 단축키로 실행한
//! 결과가 같음을 임시 폴더의 실제 앱으로 확인한다.

use std::fs;
use std::path::Path;
use std::time::Duration;

use facet_core::Location;
use facet_core::command::{CommandId, find_by_shortcut};
use facet_core::history::{NotUndoable, OperationKind, Record, UndoError};
use facet_core::menu::{Menu, MenuKind};
use facet_core::tab::ReadOutcome;
use facet_core::window::{App, Execution};
use tempfile::TempDir;

/// CI 러너가 느려도 흔들리지 않게 넉넉히 기다린다. 결과가 오면 곧바로 끝난다.
const WAIT: Duration = Duration::from_secs(20);

fn tree() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join("a").join("a1")).unwrap();
    fs::write(dir.path().join("파일.txt"), b"x").unwrap();
    dir
}

fn app_at(path: &Path) -> App {
    let mut app = App::new(Some(Location::file_system(path)));
    let (_, outcome) = app.wait(WAIT).expect("첫 읽기");
    assert_eq!(outcome, ReadOutcome::Committed);
    app
}

/// 빈 곳을 오른쪽 클릭해 빈 곳 메뉴를 연다.
fn background_menu(app: &mut App) -> Menu {
    assert_eq!(app.context_click(0, None), Some(MenuKind::Background));
    app.context_menu(MenuKind::Background)
}

/// 빈 곳 메뉴에서 그 줄을 찾아(켜져 있어야 함) 앱이 하듯 그 줄의 명령을 실행한다.
fn click(app: &mut App, id: CommandId) -> Execution {
    let menu = background_menu(app);
    let row = menu
        .commands()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("빈 곳 메뉴에 {id:?} 줄이 없다"));
    assert!(row.enabled, "{id:?} 줄이 꺼져 있다");
    app.execute(row.id)
}

fn row_enabled(app: &mut App, id: CommandId) -> bool {
    background_menu(app)
        .commands()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("빈 곳 메뉴에 {id:?} 줄이 없다"))
        .enabled
}

fn location(app: &App) -> Location {
    app.active_tab().location().clone()
}

/// `dir` 아래 `a` 폴더로 들어간 앱(뒤로 갈 기록이 있음).
fn app_inside_a(dir: &Path) -> App {
    let mut app = app_at(dir);
    let index = app
        .active_tab()
        .items()
        .position(|i| i.name == "a")
        .unwrap();
    app.active_tab_mut().select_only(index);
    assert_eq!(app.execute(CommandId::Open), Execution::Done);
    app.wait(WAIT).expect("읽기");
    assert_eq!(location(&app), Location::file_system(dir.join("a")));
    app
}

#[test]
fn back_forward_and_up_from_the_background_menu() {
    let dir = tree();
    let mut app = app_inside_a(dir.path());

    assert!(!row_enabled(&mut app, CommandId::Forward), "앞으로 갈 기록이 없다");
    assert_eq!(click(&mut app, CommandId::Back), Execution::Done);
    app.wait(WAIT).expect("읽기");
    assert_eq!(location(&app), Location::file_system(dir.path()));

    assert!(row_enabled(&mut app, CommandId::Forward), "앞으로 갈 기록이 생겼다");
    assert_eq!(click(&mut app, CommandId::Forward), Execution::Done);
    app.wait(WAIT).expect("읽기");
    assert_eq!(location(&app), Location::file_system(dir.path().join("a")));

    assert_eq!(click(&mut app, CommandId::Up), Execution::Done);
    app.wait(WAIT).expect("읽기");
    assert_eq!(location(&app), Location::file_system(dir.path()));
}

#[test]
fn undo_from_the_background_menu() {
    let dir = tree();
    let mut app = app_at(dir.path());
    assert!(!row_enabled(&mut app, CommandId::Undo), "작업 기록이 없다");

    app.record_operation(Record::not_undoable(
        OperationKind::Rename,
        Vec::new(),
        NotUndoable::PermanentDelete,
    ));
    assert!(row_enabled(&mut app, CommandId::Undo));
    assert_eq!(
        click(&mut app, CommandId::Undo),
        Execution::UndoRefused(UndoError::NotUndoable(NotUndoable::PermanentDelete)),
        "메뉴 줄도 실행취소 길을 탄다"
    );
    assert!(app.work_area().is_empty(), "되돌릴 수 없는 기록은 작업을 만들지 않는다");
}

#[test]
fn split_and_merge_panels_from_the_background_menu() {
    let dir = tree();
    let mut app = app_at(dir.path());
    assert_eq!(app.active_window().panels().len(), 1);

    assert_eq!(click(&mut app, CommandId::TogglePanelSplit), Execution::Done);
    assert_eq!(app.active_window().panels().len(), 2);
    assert_eq!(app.active_window().active_panel_index(), 1);

    // 새 오른쪽 패널(활성)의 빈 곳 메뉴에서 다시 누르면 합쳐진다.
    assert_eq!(app.context_click(1, None), Some(MenuKind::Background));
    let menu = app.context_menu(MenuKind::Background);
    let row = menu
        .commands()
        .find(|c| c.id == CommandId::TogglePanelSplit)
        .unwrap();
    assert!(row.enabled);
    assert_eq!(app.execute(row.id), Execution::Done);
    assert_eq!(app.active_window().panels().len(), 1);
}

#[test]
fn search_and_new_folder_from_the_background_menu() {
    let dir = tree();
    let mut app = app_at(dir.path());
    assert_eq!(click(&mut app, CommandId::Search), Execution::Search);
    assert_eq!(click(&mut app, CommandId::NewFolder), Execution::NewFolder);
    assert!(
        background_menu(&mut app)
            .commands()
            .any(|c| c.id == CommandId::Paste),
        "붙여넣기 줄은 그대로 있다"
    );
}

fn assert_menu_agrees(app: &mut App, state: &str) {
    let menu = background_menu(app);
    for command in menu.commands() {
        assert_eq!(
            command.enabled,
            app.can_execute(command.id),
            "{state}: {:?}",
            command.id
        );
    }
}

#[test]
fn background_menu_agrees_with_command_enablement() {
    let dir = tree();
    let mut app = app_at(dir.path());
    assert_menu_agrees(&mut app, "새 탭(기록 없음)");

    let mut app = app_inside_a(dir.path());
    assert_menu_agrees(&mut app, "뒤로 갈 기록 있음");

    assert_eq!(app.execute(CommandId::Back), Execution::Done);
    app.wait(WAIT).expect("읽기");
    assert_menu_agrees(&mut app, "앞으로 갈 기록 있음");

    app.record_operation(Record::not_undoable(
        OperationKind::Rename,
        Vec::new(),
        NotUndoable::PermanentDelete,
    ));
    assert_menu_agrees(&mut app, "실행취소 대상 있음");

    assert_eq!(app.execute(CommandId::TogglePanelSplit), Execution::Done);
    app.wait(WAIT).expect("새 패널 읽기");
    assert_menu_agrees(&mut app, "패널 둘");
}

/// 같은 상태의 두 앱에서 하나는 메뉴 줄로, 하나는 첫 기본 단축키로 찾은 명령으로 실행해 결과를 견준다.
#[test]
fn menu_row_and_first_shortcut_give_the_same_result() {
    let dir = tree();
    for id in [
        CommandId::Back,
        CommandId::Up,
        CommandId::TogglePanelSplit,
        CommandId::Search,
        CommandId::Undo,
    ] {
        let shortcut = id.shortcuts()[0];
        assert_eq!(
            find_by_shortcut(shortcut),
            Some(id),
            "{id:?}의 첫 단축키가 그 명령을 찾아야 한다"
        );

        let mut by_menu = app_inside_a(dir.path());
        let mut by_key = app_inside_a(dir.path());
        for app in [&mut by_menu, &mut by_key] {
            app.record_operation(Record::not_undoable(
                OperationKind::Rename,
                Vec::new(),
                NotUndoable::PermanentDelete,
            ));
        }

        let from_menu = click(&mut by_menu, id);
        by_key.context_click(0, None);
        let from_key = by_key.execute(find_by_shortcut(shortcut).unwrap());
        assert_eq!(from_menu, from_key, "{id:?}의 실행 결과");

        if matches!(id, CommandId::Back | CommandId::Up) {
            by_menu.wait(WAIT).expect("메뉴 쪽 읽기");
            by_key.wait(WAIT).expect("단축키 쪽 읽기");
        }
        assert_eq!(location(&by_menu), location(&by_key), "{id:?}의 위치");
        assert_eq!(
            by_menu.active_window().panels().len(),
            by_key.active_window().panels().len(),
            "{id:?}의 패널 수"
        );
    }
}
