//! **일괄 이름 바꾸기** 작업: 여러 항목의 이름을 같은 폴더 안에서 한꺼번에 바꾼다. **전부 아니면 전무**다.
//!
//! - **실행 전 확인**(아무것도 바꾸기 전): 항목마다 원본이 있는지, 새 이름 자리에 이 작업이 비우지 않을 다른 항목이
//!   없는지(대소문자 무시, 대소문자만 바꾸는 이름은 같은 항목) 디스크에서 본다. 하나라도 걸리면 아무것도 바꾸지 않고
//!   그 항목들을 [`Summary::failures`]에 담아 실패로 끝난다(`SourceMissing`·`TargetExists`).
//! - **바꾸기**는 같은 볼륨 이름 바꾸기([`file_ops::rename`])다. 다른 바뀌는 항목이 비울 이름을 쓰는 항목(사슬
//!   `a→aa, aa→aaaa`, 맞바꿈)과 대소문자만 바꾸는 항목은 먼저 같은 폴더 안 임시 이름(`~facet-<pid>-<n>.tmp`)으로
//!   옮겨 두고, 다른 항목을 다 바꾼 뒤 제 이름으로 놓는다([`permute`]). 그래서 어떤 순열도 덮어쓰기 없이 풀린다.
//! - 일시정지·취소는 이름 바꾸기 사이에서 듣는다. 항목별 오류 질문은 하지 않는다.
//! - 바꾸는 중 파일 연산이 실패하거나 취소하면 이미 바꾼 항목(임시 이름 포함)을 모두 원래 이름으로 되돌린다.
//!   되돌리기마저 실패한 항목(드묾)은 지금 경로로 [`Summary::failures`]에 더하고, 바뀐 채 남은 항목만 작업 일지에
//!   적어 작업 기록으로 남게 한다(실행취소로 되돌릴 수 있게).
//!
//! 진행은 항목 수(바꿀 항목 + 규칙이 바꾸지 않아 건너뛴 항목)로 센다. 건너뛴 항목은 처음부터 한 것으로 센다.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use facet_win::file_ops::{self, FileOpErrorKind};

use super::copy::{Copier, temp_path};
use super::{Context, ItemFailure, JournalSink, Summary};
use crate::history::{Stamp, Step, path_key};

/// 일괄 이름 바꾸기의 항목 하나: 든 폴더, 원래 이름, 새 이름. 같은 폴더 안에서 바꾼다.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RenameItem {
    /// 든 폴더.
    pub folder: PathBuf,
    /// 원래 이름.
    pub from: OsString,
    /// 새 이름.
    pub to: OsString,
}

impl RenameItem {
    /// 원래 경로.
    pub fn source(&self) -> PathBuf {
        self.folder.join(&self.from)
    }

    /// 새 경로.
    pub fn target(&self) -> PathBuf {
        self.folder.join(&self.to)
    }
}

/// 넣기 전에 정한 일괄 이름 바꾸기 작업.
pub(super) struct BatchRenameJob {
    /// 바꿀 항목(넘긴 차례대로).
    items: Vec<RenameItem>,
    /// 규칙이 바꾸지 않아 건너뛴 항목 수.
    unchanged: u64,
    /// 한 일을 적을 곳(작업 기록에 남길 작업만).
    pub(super) journal: Option<JournalSink>,
}

impl BatchRenameJob {
    pub(super) fn new(items: Vec<RenameItem>, unchanged: u64) -> Self {
        BatchRenameJob {
            items,
            unchanged,
            journal: None,
        }
    }

    /// 시작하지 않은 채 취소할 때 남은 항목 수.
    pub(super) fn root_count(&self) -> u64 {
        self.items.len() as u64 + self.unchanged
    }

    /// 작업 스레드에서 끝까지(또는 취소될 때까지) 실행한다.
    pub(super) fn run(self, context: &Context<'_>) -> (Summary, bool) {
        let mut engine = Copier::new(context, false);
        let totals = Arc::clone(&engine.reporter.totals);
        totals.items.store(self.root_count(), Ordering::Release);
        totals.counted.store(true, Ordering::Release);
        engine.summary.names_unchanged = self.unchanged;
        engine.reporter.items_done = self.unchanged;

        let sources: Vec<PathBuf> = self.items.iter().map(RenameItem::source).collect();
        let targets: Vec<PathBuf> = self.items.iter().map(RenameItem::target).collect();

        let problems = check_before(&sources, &targets);
        if !problems.is_empty() {
            engine.summary.failures = problems;
            engine.reporter.items_done = 0;
            return engine.finish();
        }

        let mut current = sources.clone();
        let control = context.control;
        let outcome = {
            let reporter = &mut engine.reporter;
            permute(&mut current, &targets, &mut |op| match op {
                Op::Before => control.proceed(),
                Op::Placed(path) => {
                    reporter.items_done += 1;
                    reporter.current = Some(path.to_path_buf());
                    reporter.maybe_emit(control);
                    true
                }
            })
        };
        match outcome {
            Ok(()) => {
                engine.summary.items_renamed = self.items.len() as u64;
                self.write_journal(&sources, &current);
            }
            Err(stop) => {
                if let Stop::Failed { path, kind } = stop {
                    engine
                        .summary
                        .failures
                        .push(ItemFailure { source: path, kind });
                } else {
                    engine.cancelled = true;
                }
                // 모두 되돌린다. 되돌리지 못한 항목은 지금 경로로 실패에 더한다.
                for (path, kind) in roll_back(&mut current, &sources) {
                    engine
                        .summary
                        .failures
                        .push(ItemFailure { source: path, kind });
                }
                engine.reporter.items_done = 0;
                engine.summary.items_renamed = 0;
                self.write_journal(&sources, &current);
            }
        }
        let (summary, cancelled) = engine.finish();
        (summary, cancelled)
    }

    /// 바뀐 채 있는 항목(지금 경로가 원래 경로와 다름)을 넘긴 차례대로 일지에 적는다.
    fn write_journal(&self, sources: &[PathBuf], current: &[PathBuf]) {
        let Some(journal) = &self.journal else {
            return;
        };
        let steps: Vec<Step> = sources
            .iter()
            .zip(current)
            .filter(|(from, to)| from.as_os_str() != to.as_os_str())
            .filter_map(|(from, to)| {
                Stamp::read(to).map(|stamp| Step::Renamed {
                    from: from.clone(),
                    to: to.clone(),
                    stamp,
                })
            })
            .collect();
        super::lock(journal).steps.extend(steps);
    }
}

/// 실행 전 확인: 원본이 없으면 `SourceMissing`, 새 이름 자리에 이 작업이 비우지 않을 항목이 있거나(디스크) 다른
/// 항목과 새 이름이 겹치면 `TargetExists`. 걸린 항목의 원래 경로와 까닭.
fn check_before(sources: &[PathBuf], targets: &[PathBuf]) -> Vec<ItemFailure> {
    let source_keys: HashMap<String, usize> = sources
        .iter()
        .enumerate()
        .map(|(i, path)| (path_key(path), i))
        .collect();
    let mut seen_targets = std::collections::HashSet::new();
    let mut problems = Vec::new();
    for (source, target) in sources.iter().zip(targets) {
        if fs::symlink_metadata(source).is_err() {
            problems.push(ItemFailure {
                source: source.clone(),
                kind: FileOpErrorKind::SourceMissing,
            });
            continue;
        }
        let key = path_key(target);
        let vacated = source_keys.contains_key(&key);
        let taken = !seen_targets.insert(key) || (!vacated && fs::symlink_metadata(target).is_ok());
        if taken {
            problems.push(ItemFailure {
                source: source.clone(),
                kind: FileOpErrorKind::TargetExists,
            });
        }
    }
    problems
}

/// [`permute`]가 부르는 쪽에 알리는 때.
pub(super) enum Op<'p> {
    /// 이름 바꾸기 하나를 하기 전. `false`를 돌려주면(취소) 멈춘다.
    Before,
    /// 항목 하나가 제 자리(`path`)에 놓였다. `false`를 돌려주면 멈춘다.
    Placed(&'p Path),
}

/// [`permute`]가 멈춘 까닭.
#[derive(Debug)]
pub(super) enum Stop {
    /// 부르는 쪽이 멈췄다(취소).
    Cancelled,
    /// `path`의 이름 바꾸기가 실패했다.
    Failed {
        path: PathBuf,
        kind: FileOpErrorKind,
    },
}

/// 항목들을 `current[i]`에서 `targets[i]`로 옮긴다(같은 폴더 안 이름 바꾸기). 옮긴 만큼 `current`를 고친다.
///
/// 제 자리가 다른 항목이 지금 있는 자리이거나 대소문자만 다른 자리인 항목은 먼저 같은 폴더 안 임시 이름으로 옮겨
/// 두고, 그렇지 않은 항목을 모두 옮긴 뒤 임시 이름에서 제 자리로 놓는다. 그래서 사슬·맞바꿈도 덮어쓰기 없이 풀린다.
/// 제 자리는 비어 있거나 다른 항목이 지금 있는 자리여야 한다(부르는 쪽이 확인한다).
pub(super) fn permute(
    current: &mut [PathBuf],
    targets: &[PathBuf],
    notify: &mut dyn FnMut(Op<'_>) -> bool,
) -> Result<(), Stop> {
    let occupied: HashMap<String, usize> = current
        .iter()
        .enumerate()
        .map(|(i, path)| (path_key(path), i))
        .collect();
    let mut via_temp = Vec::new();
    let mut direct = Vec::new();
    for (i, (now, target)) in current.iter().zip(targets).enumerate() {
        if now.as_os_str() == target.as_os_str() {
            continue;
        }
        // 다른 항목이 있는 자리이거나 자기 자신(대소문자만 다름)이면 임시 이름을 거친다.
        if occupied.contains_key(&path_key(target)) {
            via_temp.push(i);
        } else {
            direct.push(i);
        }
    }
    for &i in &via_temp {
        let temp = temp_path(&targets[i]);
        step(current, i, temp, notify)?;
    }
    for &i in direct.iter().chain(&via_temp) {
        step(current, i, targets[i].clone(), notify)?;
        if !notify(Op::Placed(&targets[i])) {
            return Err(Stop::Cancelled);
        }
    }
    Ok(())
}

/// 항목 `i`를 `to`로 옮긴다.
fn step(
    current: &mut [PathBuf],
    i: usize,
    to: PathBuf,
    notify: &mut dyn FnMut(Op<'_>) -> bool,
) -> Result<(), Stop> {
    if !notify(Op::Before) {
        return Err(Stop::Cancelled);
    }
    match file_ops::rename(&current[i], &to) {
        Ok(()) => {
            current[i] = to;
            Ok(())
        }
        Err(error) => Err(Stop::Failed {
            path: current[i].clone(),
            kind: error.kind(),
        }),
    }
}

/// 항목들을 `current`에서 `originals`로 모두 되돌린다(취소를 듣지 않는다). 한 번에 다 되돌리지 못하면 남은 항목만
/// 다시 해 보고, 더 나아지지 않으면 그만둔다. 되돌리지 못한 항목의 지금 경로와 까닭을 돌려준다.
pub(super) fn roll_back(
    current: &mut [PathBuf],
    originals: &[PathBuf],
) -> Vec<(PathBuf, FileOpErrorKind)> {
    let mut last_failure = None;
    // 항목 수만큼이면 실패가 하나씩만 풀려도 끝까지 간다.
    for _ in 0..=current.len() {
        let pending: Vec<usize> = (0..current.len())
            .filter(|&i| current[i].as_os_str() != originals[i].as_os_str())
            .collect();
        if pending.is_empty() {
            return Vec::new();
        }
        let mut now: Vec<PathBuf> = pending.iter().map(|&i| current[i].clone()).collect();
        let wanted: Vec<PathBuf> = pending.iter().map(|&i| originals[i].clone()).collect();
        let result = permute(&mut now, &wanted, &mut |_| true);
        let moved = pending
            .iter()
            .zip(&now)
            .filter(|(i, path)| current[**i] != **path)
            .count();
        for (&i, path) in pending.iter().zip(now) {
            current[i] = path;
        }
        match result {
            Ok(()) => return Vec::new(),
            Err(Stop::Failed { kind, .. }) => last_failure = Some(kind),
            Err(Stop::Cancelled) => {}
        }
        if moved == 0 {
            break;
        }
    }
    let kind = last_failure.unwrap_or(FileOpErrorKind::Other);
    (0..current.len())
        .filter(|&i| current[i].as_os_str() != originals[i].as_os_str())
        .map(|i| (current[i].clone(), kind))
        .collect()
}
