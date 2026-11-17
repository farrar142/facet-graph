//! 일괄 이름 바꾸기 작업(`JobQueue::batch_rename`, `App::submit_batch_rename`)과 그 작업 기록·실행취소를 임시 폴더의
//! 실제 파일로 확인한다: 바꾸기와 건너뛴 수, 사슬·맞바꿈·대소문자만 바꾸기, 실행 전 디스크 확인, 잠긴 파일에서 모두
//! 되돌림, 취소의 전부 아니면 전무, 기록 하나와 실행취소·다시 실행 한 번, 밖에서 고친 항목이 있는 실행취소.
//!
//! 이름 바꾸기는 Windows 파일 연산이라 Windows에서만 돈다.
#![cfg(windows)]

use std::ffi::OsString;
use std::fs;
use std::os::windows::fs::OpenOptionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

use facet_core::Location;
use facet_core::history::{NotReverted, OperationKind, RevertSkip, Step};
use facet_core::job::{
    FileOpErrorKind, JobEvent, JobId, JobKind, JobQueue, JobState, RenameItem, Summary,
};
use facet_core::tab::ReadOutcome;
use facet_core::window::App;
use tempfile::TempDir;

const WAIT: Duration = Duration::from_secs(30);

fn folder_with(names: &[&str]) -> TempDir {
    let dir = TempDir::new().unwrap();
    for name in names {
        fs::write(dir.path().join(name), name).unwrap();
    }
    dir
}

fn names_on_disk(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// 이름과 내용(이름 순).
fn contents(dir: &Path) -> Vec<(String, String)> {
    let mut entries: Vec<(String, String)> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().to_string_lossy().into_owned(),
                fs::read_to_string(entry.path()).unwrap(),
            )
        })
        .collect();
    entries.sort();
    entries
}

fn item(dir: &Path, from: &str, to: &str) -> RenameItem {
    RenameItem {
        folder: dir.to_path_buf(),
        from: OsString::from(from),
        to: OsString::from(to),
    }
}

/// 대기열의 작업이 끝날 때까지 사건을 받아 마지막 상태와 결과를 돌려준다.
fn finish(queue: &mut JobQueue, job: JobId) -> (JobState, Summary) {
    let deadline = Instant::now() + WAIT;
    let mut state = None;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match queue
            .wait_event(left)
            .expect("작업이 시간 안에 끝나지 않았다")
        {
            JobEvent::StateChanged { job: id, state: s } if id == job => state = Some(s),
            JobEvent::Finished { job: id, summary } if id == job => {
                return (state.expect("마지막 상태"), summary);
            }
            _ => {}
        }
    }
}

fn app_at(path: &Path) -> App {
    let mut app = App::new(Some(Location::file_system(path)));
    let (_, outcome) = app.wait(WAIT).expect("첫 읽기");
    assert_eq!(outcome, ReadOutcome::Committed);
    app
}

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

#[test]
fn renames_items_and_counts_unchanged_ones_as_skipped() {
    let dir = folder_with(&["사진 1.jpg", "사진 2.jpg", "메모.txt"]);
    let mut queue = JobQueue::new();
    let job = queue.batch_rename(
        vec![
            item(dir.path(), "사진 1.jpg", "휴가 1.jpg"),
            item(dir.path(), "사진 2.jpg", "휴가 2.jpg"),
        ],
        1,
    );
    let (state, summary) = finish(&mut queue, job);
    assert_eq!(state, JobState::Finished);
    assert_eq!(summary.items_renamed, 2);
    assert_eq!(summary.names_unchanged, 1);
    assert_eq!(summary.items_done, 3);
    assert_eq!(summary.items_completed(), 2);
    assert_eq!(summary.items_skipped(), 1);
    assert_eq!(
        contents(dir.path()),
        [
            ("메모.txt".to_string(), "메모.txt".to_string()),
            ("휴가 1.jpg".to_string(), "사진 1.jpg".to_string()),
            ("휴가 2.jpg".to_string(), "사진 2.jpg".to_string()),
        ]
    );
}

#[test]
fn chains_swaps_and_case_only_changes_go_through_temporary_names() {
    // 사슬: a → aa, aa → aaaa.
    let dir = folder_with(&["a.txt", "aa.txt"]);
    let mut queue = JobQueue::new();
    let job = queue.batch_rename(
        vec![
            item(dir.path(), "a.txt", "aa.txt"),
            item(dir.path(), "aa.txt", "aaaa.txt"),
        ],
        0,
    );
    assert_eq!(finish(&mut queue, job).0, JobState::Finished);
    assert_eq!(
        contents(dir.path()),
        [
            ("aa.txt".to_string(), "a.txt".to_string()),
            ("aaaa.txt".to_string(), "aa.txt".to_string()),
        ]
    );

    // 맞바꿈과 대소문자만 바꾸기, 셋 돌리기.
    let dir = folder_with(&["x.txt", "y.txt", "readme.md", "1", "2", "3"]);
    let job = queue.batch_rename(
        vec![
            item(dir.path(), "x.txt", "y.txt"),
            item(dir.path(), "y.txt", "x.txt"),
            item(dir.path(), "readme.md", "README.md"),
            item(dir.path(), "1", "2"),
            item(dir.path(), "2", "3"),
            item(dir.path(), "3", "1"),
        ],
        0,
    );
    let (state, summary) = finish(&mut queue, job);
    assert_eq!(state, JobState::Finished);
    assert_eq!(summary.items_renamed, 6);
    assert_eq!(
        contents(dir.path()),
        [
            ("1".to_string(), "3".to_string()),
            ("2".to_string(), "1".to_string()),
            ("3".to_string(), "2".to_string()),
            ("README.md".to_string(), "readme.md".to_string()),
            ("x.txt".to_string(), "y.txt".to_string()),
            ("y.txt".to_string(), "x.txt".to_string()),
        ],
        "임시 이름이 남지 않는다"
    );
}

#[test]
fn disk_check_refuses_before_changing_anything() {
    let dir = folder_with(&["a.txt", "b.txt", "결과.txt"]);
    let mut queue = JobQueue::new();
    let job = queue.batch_rename(
        vec![
            item(dir.path(), "a.txt", "새 a.txt"),
            // 대소문자만 다른 다른 항목이 이미 있다.
            item(dir.path(), "b.txt", "결과.TXT"),
            item(dir.path(), "없음.txt", "있음.txt"),
        ],
        0,
    );
    let (state, summary) = finish(&mut queue, job);
    assert_eq!(state, JobState::Failed);
    assert_eq!(summary.items_renamed, 0);
    let failures: Vec<_> = summary
        .failures
        .iter()
        .map(|failure| (failure.source.clone(), failure.kind))
        .collect();
    assert_eq!(
        failures,
        [
            (dir.path().join("b.txt"), FileOpErrorKind::TargetExists),
            (dir.path().join("없음.txt"), FileOpErrorKind::SourceMissing),
        ]
    );
    assert_eq!(
        names_on_disk(dir.path()),
        ["a.txt", "b.txt", "결과.txt"],
        "아무것도 바꾸지 않는다"
    );
}

#[test]
fn a_locked_item_rolls_every_rename_back_and_leaves_no_record() {
    let dir = folder_with(&["1.txt", "2.txt", "3.txt"]);
    let mut app = app_at(dir.path());
    let lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(dir.path().join("2.txt"))
        .unwrap();
    let job = app.submit_batch_rename(
        vec![
            item(dir.path(), "1.txt", "새 1.txt"),
            item(dir.path(), "2.txt", "새 2.txt"),
            item(dir.path(), "3.txt", "새 3.txt"),
        ],
        0,
    );
    assert_eq!(
        app.work_area().entry(job).map(|entry| entry.kind),
        Some(JobKind::BatchRename)
    );
    let (state, summary) = wait_job(&mut app, job);
    drop(lock);
    assert_eq!(state, JobState::Failed);
    assert_eq!(summary.items_renamed, 0);
    assert_eq!(summary.failures.len(), 1);
    assert_eq!(summary.failures[0].source, dir.path().join("2.txt"));
    assert_eq!(names_on_disk(dir.path()), ["1.txt", "2.txt", "3.txt"]);
    assert!(app.history().is_empty());
}

#[test]
fn cancelling_leaves_all_or_nothing() {
    let names: Vec<String> = (0..2000).map(|n| format!("f{n:04}.txt")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let dir = folder_with(&refs);
    let mut app = app_at(dir.path());
    let items = names
        .iter()
        .map(|name| item(dir.path(), name, &format!("x{name}")))
        .collect();
    let job = app.submit_batch_rename(items, 0);
    app.jobs_mut().cancel(job);
    let (state, summary) = wait_job(&mut app, job);
    let renamed: Vec<String> = names.iter().map(|name| format!("x{name}")).collect();
    match state {
        JobState::Cancelled => {
            assert_eq!(names_on_disk(dir.path()), names);
            assert_eq!(summary.items_renamed, 0);
            assert!(app.history().is_empty());
        }
        JobState::Finished => {
            assert_eq!(names_on_disk(dir.path()), renamed);
            assert_eq!(app.history().records().len(), 1);
        }
        other => panic!("취소했거나 다 끝났어야 한다: {other:?}"),
    }
}

#[test]
fn one_record_undoes_and_redoes_every_rename_at_once() {
    let dir = folder_with(&["a.txt", "aa.txt", "c.txt"]);
    let mut app = app_at(dir.path());
    let before = contents(dir.path());
    let job = app.submit_batch_rename(
        vec![
            item(dir.path(), "a.txt", "aa.txt"),
            item(dir.path(), "aa.txt", "aaaa.txt"),
        ],
        1,
    );
    let (state, _) = wait_job(&mut app, job);
    assert_eq!(state, JobState::Finished);
    let after = contents(dir.path());

    assert_eq!(app.history().records().len(), 1);
    let record = app.history().undo_target().unwrap();
    assert_eq!(record.kind(), OperationKind::BatchRename);
    assert_eq!(record.items_completed(), 2);
    assert_eq!(record.items_skipped(), 1);
    let steps: Vec<_> = record
        .steps()
        .iter()
        .map(|step| match step {
            Step::Renamed { from, to, .. } => (from.clone(), to.clone()),
            other => panic!("이름바꾸기여야 한다: {other:?}"),
        })
        .collect();
    assert_eq!(
        steps,
        [
            (dir.path().join("a.txt"), dir.path().join("aa.txt")),
            (dir.path().join("aa.txt"), dir.path().join("aaaa.txt")),
        ]
    );

    let job = app.undo().unwrap();
    let (state, summary) = wait_job(&mut app, job);
    assert_eq!(state, JobState::Finished);
    assert_eq!(summary.items_completed(), 2);
    assert_eq!(contents(dir.path()), before);
    assert!(app.history().undo_target().is_none());
    assert_eq!(app.history().redo_records().len(), 1);

    let job = app.redo().unwrap();
    let (state, _) = wait_job(&mut app, job);
    assert_eq!(state, JobState::Finished);
    assert_eq!(contents(dir.path()), after);
    assert_eq!(app.history().records().len(), 1);
    assert_eq!(app.history().redo_records().len(), 0);
}

#[test]
fn undo_changes_nothing_if_any_item_was_changed_outside() {
    let dir = folder_with(&["a.txt", "b.txt"]);
    let mut app = app_at(dir.path());
    let job = app.submit_batch_rename(
        vec![
            item(dir.path(), "a.txt", "xa.txt"),
            item(dir.path(), "b.txt", "xb.txt"),
        ],
        0,
    );
    assert_eq!(wait_job(&mut app, job).0, JobState::Finished);
    fs::write(dir.path().join("xb.txt"), "고친 내용").unwrap();

    let job = app.undo().unwrap();
    let (state, summary) = wait_job(&mut app, job);
    assert_eq!(state, JobState::Failed);
    assert_eq!(
        summary.not_reverted,
        [NotReverted {
            path: dir.path().join("xb.txt"),
            reason: RevertSkip::Changed,
        }]
    );
    assert_eq!(names_on_disk(dir.path()), ["xa.txt", "xb.txt"]);
    let target = app.history().undo_target().expect("기록이 남는다");
    assert_eq!(target.kind(), OperationKind::BatchRename);
    assert_eq!(target.steps().len(), 2);

    // 되바꿀 자리를 밖에서 차지해도 아무것도 바꾸지 않는다.
    fs::write(dir.path().join("xb.txt"), "b.txt").unwrap();
    let dir2 = folder_with(&["p.txt", "q.txt"]);
    let mut app = app_at(dir2.path());
    let job = app.submit_batch_rename(
        vec![
            item(dir2.path(), "p.txt", "xp.txt"),
            item(dir2.path(), "q.txt", "xq.txt"),
        ],
        0,
    );
    assert_eq!(wait_job(&mut app, job).0, JobState::Finished);
    fs::write(dir2.path().join("q.txt"), "남").unwrap();
    let job = app.undo().unwrap();
    let (state, summary) = wait_job(&mut app, job);
    assert_eq!(state, JobState::Failed);
    assert_eq!(
        summary.not_reverted,
        [NotReverted {
            path: dir2.path().join("xq.txt"),
            reason: RevertSkip::NameTaken,
        }]
    );
    assert_eq!(names_on_disk(dir2.path()), ["q.txt", "xp.txt", "xq.txt"]);
    assert!(app.history().undo_target().is_some());
}
