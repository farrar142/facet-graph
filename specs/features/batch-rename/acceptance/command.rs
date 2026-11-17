// target: crates/facet-core/tests/spec_batch_rename_command.rs
// AC: AC-8, AC-10, AC-17, AC-18, AC-20
//! 일괄 이름 바꾸기 명령(`CommandId::BatchRename`)의 id·단축키·켜짐, 항목 오른쪽 메뉴의 새 줄(16줄), 임시 폴더의 앱에서
//! 메뉴·팔레트 켜짐이 `App::can_execute`와 같음, 선택 항목 모으기와 선택 밖 이름과의 겹침, 실행 전에 막힌 실행이
//! 작업을 만들지 않음을 확인한다. 파일을 바꾸지 않으므로 모든 환경에서 돈다.

use std::fs;
use std::path::Path;
use std::time::Duration;

use facet_core::Location;
use facet_core::batch_rename::{BatchProblem, EntryStatus, RenameRule};
use facet_core::command::CommandId;
use facet_core::details::{Column, Direction, Sort};
use facet_core::menu::{Mark, Menu, MenuContext, MenuEntry, MenuKind, build};
use facet_core::tab::ReadOutcome;
use facet_core::window::{App, BatchRenameError, Execution};
use tempfile::TempDir;

/// CI 러너가 느려도 흔들리지 않게 넉넉히 기다린다. 결과가 오면 곧바로 끝난다.
const WAIT: Duration = Duration::from_secs(20);

const NAMES: [&str; 4] = ["a.txt", "b.txt", "c.txt", "d.txt"];

fn folder() -> TempDir {
    let dir = TempDir::new().unwrap();
    for name in NAMES {
        fs::write(dir.path().join(name), name).unwrap();
    }
    dir
}

fn app_at(path: &Path) -> App {
    let mut app = App::new(Some(Location::file_system(path)));
    let (_, outcome) = app.wait(WAIT).expect("첫 읽기");
    assert_eq!(outcome, ReadOutcome::Committed);
    app
}

fn index_of(app: &App, name: &str) -> usize {
    let tab = app.active_tab();
    (0..tab.len())
        .find(|&i| tab.get(i).is_some_and(|item| item.name == name))
        .unwrap_or_else(|| panic!("목록에 {name}이 없다"))
}

/// 이름들을 고른다(첫 이름만 고른 뒤 나머지를 더함).
fn select(app: &mut App, names: &[&str]) {
    let indices: Vec<usize> = names.iter().map(|name| index_of(app, name)).collect();
    let tab = app.active_tab_mut();
    tab.select_only(indices[0]);
    for &index in &indices[1..] {
        tab.toggle_select(index);
    }
}

fn names_on_disk(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn replace(find: &str, with: &str) -> RenameRule {
    RenameRule {
        find: find.to_string(),
        replace: with.to_string(),
        ..RenameRule::default()
    }
}

fn context(selected_count: usize, folder: bool, results: bool) -> MenuContext {
    MenuContext {
        kind: MenuKind::Item,
        selected_count,
        has_cursor: true,
        cursor_is_folder: false,
        single_folder_selected: false,
        location_is_folder: folder,
        location_is_results: results,
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
        can_go_up: true,
        can_undo: false,
    }
}

/// 메뉴의 줄: 명령이면 그 id, 구분선이면 `None`.
fn layout(menu: &Menu) -> Vec<Option<CommandId>> {
    menu.entries
        .iter()
        .map(|entry| match entry {
            MenuEntry::Command(c) => Some(c.id),
            MenuEntry::Submenu(_) => panic!("항목 메뉴에는 하위 메뉴가 없다"),
            MenuEntry::Separator => None,
        })
        .collect()
}

fn batch_row_enabled(menu: &Menu) -> bool {
    menu.commands()
        .find(|c| c.id == CommandId::BatchRename)
        .expect("항목 메뉴에 일괄 이름 바꾸기 줄이 있다")
        .enabled
}

#[test]
fn the_command_has_an_id_no_shortcut_and_sits_after_rename() {
    let all = CommandId::ALL;
    let at = all
        .iter()
        .position(|c| *c == CommandId::BatchRename)
        .expect("모든 명령에 있다");
    assert_eq!(all[at - 1], CommandId::Rename);
    assert_eq!(CommandId::BatchRename.as_str(), "batch-rename");
    assert_eq!(
        CommandId::from_id("batch-rename"),
        Some(CommandId::BatchRename)
    );
    assert!(CommandId::BatchRename.shortcuts().is_empty());
}

#[test]
fn item_menu_has_batch_rename_right_below_rename() {
    use CommandId::*;
    let expected = vec![
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
    ];
    for selected in [0, 1, 2, 10_000] {
        let menu = build(&context(selected, true, false));
        assert_eq!(layout(&menu), expected, "선택 {selected}개");
        assert_eq!(batch_row_enabled(&menu), selected >= 2, "선택 {selected}개");
    }
    let batch = build(&context(2, true, false))
        .commands()
        .find(|c| c.id == BatchRename)
        .copied()
        .unwrap();
    assert_eq!(batch.mark, Mark::None);

    // 검색 결과 위치: 머리 두 줄 뒤는 같은 순서이고 두 항목 이상이면 켜진다.
    let menu = build(&context(2, false, true));
    let rows = layout(&menu);
    assert_eq!(rows[..2], [Some(ShowInFolder), None]);
    assert_eq!(rows[2..], expected[..]);
    assert!(batch_row_enabled(&menu));

    // 실제 폴더도 검색 결과도 아니면 꺼진다.
    assert!(!batch_row_enabled(&build(&context(2, false, false))));
}

#[test]
fn command_menu_and_palette_agree_on_when_it_is_enabled() {
    let dir = folder();
    let mut app = app_at(dir.path());
    let name = |c: CommandId| {
        if c == CommandId::BatchRename {
            "일괄 이름 바꾸기"
        } else {
            c.as_str()
        }
    };
    let palette_enabled = |app: &App| {
        app.search_commands("일괄", &name)
            .iter()
            .find(|e| e.command == CommandId::BatchRename)
            .expect("팔레트에서 찾아진다")
            .enabled
    };

    // 선택 없음(커서만 있음)은 꺼진다.
    app.active_tab_mut().clear_selection();
    assert!(!app.can_execute(CommandId::BatchRename));
    assert!(!batch_row_enabled(&app.context_menu(MenuKind::Item)));
    assert!(!palette_enabled(&app));
    assert_eq!(app.execute(CommandId::BatchRename), Execution::Disabled);

    // 하나는 꺼지고, 둘·모두는 켜진다.
    let cases: [(&[&str], bool); 3] = [
        (&["a.txt"], false),
        (&["a.txt", "c.txt"], true),
        (&NAMES, true),
    ];
    for (names, enabled) in cases {
        select(&mut app, names);
        assert_eq!(
            app.can_execute(CommandId::BatchRename),
            enabled,
            "선택 {names:?}"
        );
        assert_eq!(
            batch_row_enabled(&app.context_menu(MenuKind::Item)),
            enabled
        );
        assert_eq!(palette_enabled(&app), enabled);
    }

    assert_eq!(app.execute(CommandId::BatchRename), Execution::BatchRename);
    assert_eq!(
        names_on_disk(dir.path()),
        NAMES,
        "실행만으로는 바꾸지 않는다"
    );
}

#[test]
fn items_are_the_selection_in_list_order_and_unselected_names_are_taken() {
    let dir = folder();
    let mut app = app_at(dir.path());
    select(&mut app, &["c.txt", "a.txt"]);

    let items = app.batch_rename_items().expect("두 항목을 골랐다");
    let names: Vec<_> = items.iter().map(|item| item.name.clone()).collect();
    assert_eq!(names, ["a.txt", "c.txt"], "목록 순서");
    for item in &items {
        assert_eq!(item.folder, dir.path());
        assert!(!item.is_folder);
    }

    // a → b는 선택하지 않은 b.txt와 겹치고, c는 그대로다.
    let preview = app
        .batch_rename_preview(&replace("a", "b"))
        .expect("미리 보기");
    let statuses: Vec<_> = preview.entries().iter().map(|e| e.status).collect();
    assert_eq!(
        statuses,
        [
            EntryStatus::Problem(BatchProblem::NameTaken),
            EntryStatus::Unchanged
        ]
    );
    assert!(!preview.can_execute());

    // 겹치지 않으면 실행할 수 있다.
    let preview = app.batch_rename_preview(&replace("a", "z")).unwrap();
    assert_eq!(preview.entries()[0].new_name, "z.txt");
    assert!(preview.can_execute());
}

#[test]
fn rejected_runs_create_no_job_and_change_nothing() {
    let dir = folder();
    let mut app = app_at(dir.path());
    select(&mut app, &["a.txt", "c.txt"]);

    assert_eq!(
        app.start_batch_rename(&replace("a", "b")),
        Err(BatchRenameError::Problems)
    );
    assert_eq!(
        app.start_batch_rename(&RenameRule::default()),
        Err(BatchRenameError::NothingToRename)
    );
    assert_eq!(
        app.start_batch_rename(&replace("zzz", "y")),
        Err(BatchRenameError::NothingToRename)
    );

    select(&mut app, &["a.txt"]);
    assert!(app.batch_rename_items().is_none());
    assert!(app.batch_rename_preview(&replace("a", "z")).is_none());
    assert_eq!(
        app.start_batch_rename(&replace("a", "z")),
        Err(BatchRenameError::Unavailable)
    );

    app.poll_jobs();
    assert!(app.work_area().is_empty(), "작업을 만들지 않는다");
    assert!(app.history().is_empty(), "기록이 없다");
    assert_eq!(names_on_disk(dir.path()), NAMES);
}
