// target: crates/facet-core/tests/spec_batch_rename_execute.rs
// AC: AC-9, AC-10, AC-11, AC-12, AC-13, AC-14, AC-15, AC-16, AC-19
//! 일괄 이름 바꾸기 작업(`App::start_batch_rename`, `JobKind::BatchRename`)을 임시 폴더의 실제 파일로 확인한다: 바꾸기와
//! 건너뛴 항목, 작업 기록 한 건과 실행취소·다시 실행 한 번, 사슬과 대소문자만 바꾸기, 잠긴 파일에서 모두 되돌림, 취소의
//! 전부 아니면 전무, 밖에서 바뀐 항목이 있는 실행취소, 검색 결과 위치의 여러 폴더와 실행 전 디스크 확인.
//!
//! 이름 바꾸기는 Windows 파일 연산(`file_ops::rename`)이라 Windows에서만 돈다.
#![cfg(windows)]

use std::fs;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use facet_core::Location;
use facet_core::batch_rename::RenameRule;
use facet_core::command::CommandId;
use facet_core::history::{NotReverted, OperationKind, RevertSkip, Step};
use facet_core::job::{FileOpErrorKind, JobId, JobKind, JobState, Summary};
use facet_core::tab::ReadOutcome;
use facet_core::window::{App, Execution};
use tempfile::TempDir;

/// CI 러너가 느려도 흔들리지 않게 넉넉히 기다린다. 결과가 오면 곧바로 끝난다.
const WAIT: Duration = Duration::from_secs(30);

/// 폴더 안 항목의 이름·내용·수정 시각(이름 순).
type Snapshot = Vec<(String, String, SystemTime)>;

/// 지난 시각: 되돌린 뒤 수정 시각이 같은지 볼 수 있게 지금과 다르게 둔다.
fn old_time() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000)
}

/// 내용이 제 이름인 파일을 만든다.
fn write_file(path: &Path) {
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    fs::write(path, name).unwrap();
    let file = fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_modified(old_time()).unwrap();
}

fn folder_with(names: &[&str]) -> TempDir {
    let dir = TempDir::new().unwrap();
    for name in names {
        write_file(&dir.path().join(name));
    }
    dir
}

fn app_at(path: &Path) -> App {
    let mut app = App::new(Some(Location::file_system(path)));
    let (_, outcome) = app.wait(WAIT).expect("첫 읽기");
    assert_eq!(outcome, ReadOutcome::Committed);
    app
}

fn select_all(app: &mut App) {
    assert_eq!(app.execute(CommandId::SelectAll), Execution::Done);
}

fn rule(find: &str, with: &str) -> RenameRule {
    RenameRule {
        find: find.to_string(),
        replace: with.to_string(),
        ..RenameRule::default()
    }
}

fn prefix(text: &str) -> RenameRule {
    RenameRule {
        prefix: text.to_string(),
        ..RenameRule::default()
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

fn snapshot(dir: &Path) -> Snapshot {
    let mut entries: Snapshot = fs::read_dir(dir)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().to_string_lossy().into_owned(),
                fs::read_to_string(entry.path()).unwrap(),
                entry.metadata().unwrap().modified().unwrap(),
            )
        })
        .collect();
    entries.sort();
    entries
}

/// 작업이 끝날 때까지 작업 사건(과 탭 쪽 결과)을 받고 마지막 상태와 결과를 돌려준다.
fn wait_job(app: &mut App, job: JobId) -> (JobState, Summary) {
    let deadline = Instant::now() + WAIT;
    loop {
        app.poll_jobs();
        app.poll();
        if let Some(entry) = app.work_area().entry(job)
            && let Some(summary) = &entry.summary
        {
            return (entry.state, summary.clone());
        }
        assert!(Instant::now() < deadline, "작업이 시간 안에 끝나지 않았다");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn run(app: &mut App, rule: &RenameRule) -> (JobState, Summary) {
    let job = app
        .start_batch_rename(rule)
        .expect("일괄 이름 바꾸기를 시작한다");
    assert_eq!(
        app.work_area().entry(job).map(|entry| entry.kind),
        Some(JobKind::BatchRename)
    );
    wait_job(app, job)
}

fn undo(app: &mut App) -> (JobState, Summary) {
    let job = app.undo().expect("실행취소를 시작한다");
    wait_job(app, job)
}

fn redo(app: &mut App) -> (JobState, Summary) {
    let job = app.redo().expect("다시 실행을 시작한다");
    wait_job(app, job)
}

#[test]
fn renames_all_skips_unchanged_and_undoes_and_redoes_in_one_step() {
    let dir = folder_with(&["사진 1.jpg", "사진 2.jpg", "메모.txt"]);
    let mut app = app_at(dir.path());
    select_all(&mut app);
    let before = snapshot(dir.path());
    let rule = rule("사진", "휴가");

    let preview = app.batch_rename_preview(&rule).unwrap();
    assert_eq!((preview.rename_count(), preview.unchanged_count()), (2, 1));

    let (state, summary) = run(&mut app, &rule);
    assert_eq!(state, JobState::Finished);
    assert_eq!(summary.items_renamed, 2);
    assert_eq!(summary.items_completed(), 2);
    assert_eq!(summary.names_unchanged, 1);
    assert_eq!(summary.items_skipped(), 1);
    assert_eq!(
        names_on_disk(dir.path()),
        ["메모.txt", "휴가 1.jpg", "휴가 2.jpg"]
    );
    let after = snapshot(dir.path());
    assert_eq!(
        after
            .iter()
            .map(|(_, content, modified)| (content.as_str(), *modified))
            .collect::<Vec<_>>(),
        [
            ("메모.txt", old_time()),
            ("사진 1.jpg", old_time()),
            ("사진 2.jpg", old_time())
        ],
        "내용과 수정 시각은 그대로다"
    );

    assert_eq!(app.history().records().len(), 1, "기록은 한 건");
    let record = app.history().undo_target().unwrap();
    assert_eq!(record.kind(), OperationKind::BatchRename);
    assert!(record.is_undoable());
    assert_eq!(record.items_completed(), 2);
    assert_eq!(record.items_skipped(), 1);
    let renamed: Vec<_> = record
        .steps()
        .iter()
        .map(|step| match step {
            Step::Renamed { from, to, .. } => (from.clone(), to.clone()),
            other => panic!("이름바꾸기 항목이어야 한다: {other:?}"),
        })
        .collect();
    assert_eq!(
        renamed,
        [
            (dir.path().join("사진 1.jpg"), dir.path().join("휴가 1.jpg")),
            (dir.path().join("사진 2.jpg"), dir.path().join("휴가 2.jpg")),
        ]
    );

    let (state, summary) = undo(&mut app);
    assert_eq!(state, JobState::Finished);
    assert_eq!(summary.items_completed(), 2);
    assert_eq!(
        snapshot(dir.path()),
        before,
        "실행취소 한 번에 모두 돌아온다"
    );
    assert!(app.history().undo_target().is_none());

    let (state, _) = redo(&mut app);
    assert_eq!(state, JobState::Finished);
    assert_eq!(
        snapshot(dir.path()),
        after,
        "다시 실행 한 번에 모두 다시 바뀐다"
    );
    assert_eq!(app.history().records().len(), 1);
}

#[test]
fn chains_and_case_only_changes_rename_on_disk() {
    // a → aa, aa → aaaa: 차례대로 하면 aa가 아직 있다.
    let dir = folder_with(&["a.txt", "aa.txt"]);
    let mut app = app_at(dir.path());
    select_all(&mut app);
    let before = snapshot(dir.path());
    let (state, _) = run(&mut app, &rule("a", "aa"));
    assert_eq!(state, JobState::Finished);
    assert_eq!(
        snapshot(dir.path())
            .into_iter()
            .map(|(name, content, _)| (name, content))
            .collect::<Vec<_>>(),
        [
            ("aa.txt".to_string(), "a.txt".to_string()),
            ("aaaa.txt".to_string(), "aa.txt".to_string())
        ]
    );
    // 실행취소는 반대 방향 사슬이다.
    let (state, _) = undo(&mut app);
    assert_eq!(state, JobState::Finished);
    assert_eq!(snapshot(dir.path()), before);

    // aa → a, aaaa → aa.
    let dir = folder_with(&["aa.txt", "aaaa.txt"]);
    let mut app = app_at(dir.path());
    select_all(&mut app);
    let (state, _) = run(&mut app, &rule("aa", "a"));
    assert_eq!(state, JobState::Finished);
    assert_eq!(names_on_disk(dir.path()), ["a.txt", "aa.txt"]);
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "aa.txt"
    );

    // 대소문자만 바꾸기.
    let dir = folder_with(&["readme.md", "notes.md"]);
    let mut app = app_at(dir.path());
    select_all(&mut app);
    let (state, summary) = run(&mut app, &rule("readme", "README"));
    assert_eq!(state, JobState::Finished);
    assert_eq!(summary.names_unchanged, 1);
    assert_eq!(names_on_disk(dir.path()), ["README.md", "notes.md"]);
}

#[test]
fn a_locked_item_rolls_every_rename_back() {
    let names = ["1.txt", "2.txt", "3.txt"];
    let dir = folder_with(&names);
    let mut app = app_at(dir.path());
    select_all(&mut app);
    let before = snapshot(dir.path());

    // 공유 없이 열어 이름을 바꿀 수 없게 한다(잠김). 앞의 1.txt는 바뀔 수 있다.
    let lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(dir.path().join("2.txt"))
        .unwrap();
    let (state, summary) = run(&mut app, &prefix("새 "));
    drop(lock);

    assert_eq!(state, JobState::Failed);
    assert!(
        summary
            .failures
            .iter()
            .any(|failure| failure.source == dir.path().join("2.txt")),
        "실패한 항목이 결과에 든다: {:?}",
        summary.failures
    );
    assert_eq!(summary.items_renamed, 0);
    assert_eq!(
        snapshot(dir.path()),
        before,
        "모든 항목이 원래 이름이고 임시 이름도 남지 않는다"
    );
    assert!(app.history().is_empty(), "기록이 없다");
}

#[test]
fn cancelling_leaves_all_or_nothing() {
    let names: Vec<String> = (0..2000).map(|n| format!("f{n:04}.txt")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let dir = folder_with(&refs);
    let mut app = app_at(dir.path());
    select_all(&mut app);

    let job = app.start_batch_rename(&prefix("x")).unwrap();
    app.jobs_mut().cancel(job);
    let (state, _) = wait_job(&mut app, job);

    let renamed: Vec<String> = names.iter().map(|name| format!("x{name}")).collect();
    match state {
        JobState::Cancelled => {
            assert_eq!(names_on_disk(dir.path()), names, "모두 원래 이름");
            assert!(app.history().is_empty());
        }
        JobState::Finished => {
            assert_eq!(names_on_disk(dir.path()), renamed, "모두 새 이름");
            assert_eq!(app.history().records().len(), 1);
        }
        other => panic!("취소했거나 다 끝났어야 한다: {other:?}"),
    }
}

#[test]
fn undo_changes_nothing_if_any_item_was_changed_outside() {
    let dir = folder_with(&["a.txt", "b.txt"]);
    let mut app = app_at(dir.path());
    select_all(&mut app);
    let (state, _) = run(&mut app, &prefix("x"));
    assert_eq!(state, JobState::Finished);

    // 바꾼 파일 하나를 밖에서 고친다.
    fs::write(dir.path().join("xb.txt"), "고친 내용").unwrap();

    let (state, summary) = undo(&mut app);
    assert_eq!(state, JobState::Failed);
    assert_eq!(
        summary.not_reverted,
        [NotReverted {
            path: dir.path().join("xb.txt"),
            reason: RevertSkip::Changed,
        }]
    );
    assert_eq!(
        names_on_disk(dir.path()),
        ["xa.txt", "xb.txt"],
        "아무 항목도 되돌리지 않는다"
    );
    let target = app.history().undo_target().expect("기록이 남는다");
    assert_eq!(target.kind(), OperationKind::BatchRename);
    assert_eq!(target.steps().len(), 2);
}

/// 활성 탭의 이동이 확정될 때까지 기다린다.
fn wait_read(app: &mut App) {
    loop {
        let (tab, outcome) = app.wait(WAIT).expect("읽기 결과");
        if tab == app.active_tab_id() {
            assert_eq!(outcome, ReadOutcome::Committed);
            return;
        }
    }
}

/// 활성 탭의 검색이 끝날 때까지 결과를 받는다.
fn wait_search(app: &mut App) {
    let deadline = Instant::now() + WAIT;
    loop {
        app.poll();
        let status = app.active_tab().search().expect("검색 결과 위치");
        if status.summary.is_some() && !status.running {
            return;
        }
        assert!(Instant::now() < deadline, "검색이 끝나지 않았다");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// 보이는 결과의 전체 경로(정렬함).
fn result_paths(app: &App) -> Vec<PathBuf> {
    let tab = app.active_tab();
    let mut paths: Vec<PathBuf> = (0..tab.len()).filter_map(|i| tab.item_path(i)).collect();
    paths.sort();
    paths
}

#[test]
fn search_results_rename_each_item_in_its_own_folder() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("a")).unwrap();
    fs::create_dir_all(root.join("b")).unwrap();
    write_file(&root.join("a").join("보고서.txt"));
    write_file(&root.join("b").join("보고서.txt"));
    // 검색에 걸리지 않아 미리 보기가 모르는 이름.
    write_file(&root.join("a").join("결과.txt"));

    let mut app = app_at(root);
    app.start_search("보고서").unwrap();
    wait_read(&mut app);
    wait_search(&mut app);
    let a = root.join("a").join("보고서.txt");
    let b = root.join("b").join("보고서.txt");
    assert_eq!(result_paths(&app), [a.clone(), b.clone()]);
    select_all(&mut app);
    let mut folders: Vec<PathBuf> = app
        .batch_rename_items()
        .unwrap()
        .into_iter()
        .map(|item| item.folder)
        .collect();
    folders.sort();
    assert_eq!(folders, [root.join("a"), root.join("b")]);

    // 미리 보기는 a/결과.txt를 모르지만 실행 전 디스크 확인이 막는다.
    let clash = rule("보고서", "결과");
    assert!(app.batch_rename_preview(&clash).unwrap().can_execute());
    let (state, summary) = run(&mut app, &clash);
    assert_eq!(state, JobState::Failed);
    assert_eq!(summary.failures.len(), 1);
    assert_eq!(summary.failures[0].source, a);
    assert_eq!(summary.failures[0].kind, FileOpErrorKind::TargetExists);
    assert!(a.exists() && b.exists(), "아무것도 바꾸지 않는다");
    assert!(app.history().is_empty());

    // 겹치지 않으면 항목마다 제 폴더에서 바뀌고 결과도 새 이름이 된다.
    let (state, _) = run(&mut app, &rule("보고서", "최종"));
    assert_eq!(state, JobState::Finished);
    let a2 = root.join("a").join("최종.txt");
    let b2 = root.join("b").join("최종.txt");
    assert!(a2.exists() && b2.exists());
    assert!(root.join("a").join("결과.txt").exists());
    assert_eq!(result_paths(&app), [a2, b2]);
    assert_eq!(app.history().records().len(), 1);

    // 실행취소 한 번에 두 폴더 모두 돌아온다.
    let (state, _) = undo(&mut app);
    assert_eq!(state, JobState::Finished);
    assert!(a.exists() && b.exists());
    assert_eq!(result_paths(&app), [a, b]);
}
