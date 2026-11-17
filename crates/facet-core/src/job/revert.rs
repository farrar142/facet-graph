//! 실행취소·다시 실행 작업: 작업 기록의 기록 하나를 항목마다 되돌리거나 다시 한다.
//!
//! - **이름바꾸기**: 실행취소는 되바꾸고, 다시 실행은 다시 바꾼다.
//! - **일괄 이름 바꾸기**: 이름바꾸기와 같되 **전부 아니면 전무**다(`run_all_or_nothing`): 모든 항목을 먼저 확인해
//!   하나라도 밖에서 바뀌었으면 아무것도 바꾸지 않고, 사슬·맞바꿈은 임시 이름을 거치며, 중간 실패·취소면 되돌려 놓는다.
//! - **새 폴더**: 실행취소는 비어 있으면 지우고, 다시 실행은 다시 만든다.
//! - **복사**: 실행취소는 만든 항목을 **휴지통으로 보낸다**(영구 삭제하지 않는다, 실행취소가 데이터를 잃지 않게).
//!   만든 폴더가 통째로 그대로면(안의 항목이 모두 기록한 모습이고 다른 항목이 없음) 폴더 하나로 보내고, 아니면 그
//!   안에서 그대로인 항목만 보내고 폴더는 남긴다. 휴지통에 넣을 수 없는 항목(셸이 영구 삭제하려 함)은 지우지 않고
//!   [`RevertSkip::NotRecyclable`]로 건너뛴다. 다시 실행은 같은 원본을 같은 이름으로 다시 복사한다.
//! - **이동**: 실행취소는 대상의 항목을 원래 자리로 되옮긴다(같은 볼륨은 이름 바꾸기, 볼륨을 넘으면 이동 엔진의
//!   복사 뒤 원본 삭제). 이동이 만든 대상 폴더는 비면 지우고, 이동이 지운 원래 폴더는 다시 만든다. 다시 실행은 다시
//!   옮긴다.
//! - **덮어쓰기**(복사·이동이 원래 대상을 휴지통으로 보내고 덮어씀, [`Step::Overwrote`]): 실행취소는 새 파일을
//!   치우고(복사면 휴지통으로, 이동이면 원래 자리로 되옮김) 옛 파일을 휴지통에서 복원한다. 옛 파일의 휴지통 항목이
//!   없으면(휴지통을 비움) [`RevertSkip::RecycledMissing`]로 건너뛴다. 다시 실행은 옛 파일을 다시 휴지통으로 보내고
//!   다시 덮어쓴다(영구 덮어쓰기는 하지 않는다).
//! - **휴지통 삭제**([`Step::Recycled`]): 실행취소는 휴지통 항목을 원래 자리로 복원한다(원래 자리에 같은 이름이
//!   있으면 덮어쓰지 않고 [`RevertSkip::NameTaken`], 휴지통 항목이 없으면 [`RevertSkip::RecycledMissing`]). 다시
//!   실행은 다시 휴지통으로 보낸다(휴지통에 넣을 수 없으면 영구 삭제하지 않고 [`RevertSkip::NotRecyclable`]). 같은
//!   기록에서 휴지통에 들어가지 않아 영구 삭제로 진행한 항목([`Step::Deleted`])은 되살릴 것이 없어 실행취소가
//!   [`RevertSkip::PermanentlyDeleted`]로 알리고, 기록은 휴지통에 보낸 항목과 함께 다시 실행 목록으로 간다(다시
//!   실행은 그 항목에 할 일이 없다).
//!
//! 하기 전에 밖의 변경을 기록해 둔 모습([`Stamp`])으로 본다: 다룰 항목이 없거나 바뀌었거나, 되바꿀(만들·되옮길)
//! 자리에 같은 이름이 있거나, 지울 폴더가 비어 있지 않으면 그 항목을 건너뛰고 까닭을 [`Summary::not_reverted`]에
//! 담는다. 아무것도 덮어쓰거나 영구히 지우지 않는다(이름 바꾸기는 대상이 있으면 실패하고, 폴더는 빈 것만 지우고,
//! 복사본은 휴지통으로 보낸다). 파일 연산 자체의 실패(잠김·접근 거부 등)는 다른 작업과 같은 항목별 오류 질문으로
//! 묻는다.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use facet_win::file_ops::{self, FileOpErrorKind};
use facet_win::recycle::{self, RecycleOutcome, RecycledItem};

use super::copy::{Copier, ErrorStep, FileOutcome, error_kind};
use super::{Context, ItemFailure, Resolution, Summary};
use crate::history::{
    NotReverted, OperationKind, Outcomes, Reversal, RevertSkip, Stamp, Step, path_key,
};

/// 휴지통으로 한 번에 보내는 항목 수. 셸 호출 사이에서 일시정지·취소를 보고 진행을 알린다.
const RECYCLE_CHUNK: usize = 256;

/// 넣기 전에 정한 실행취소·다시 실행 작업.
pub(crate) struct RevertJob {
    reversal: Reversal,
    /// 기록의 작업 종류. 일괄 이름 바꾸기는 전부 아니면 전무로 되돌린다.
    kind: OperationKind,
    /// 할 차례대로의 항목.
    steps: Vec<Step>,
    /// 항목마다(같은 차례) 결과를 적는 곳.
    outcomes: Outcomes,
}

/// 항목 하나를 한 번 해 본 결과.
enum Attempt {
    /// 했다. 반대 방향에 쓸 새 모습의 항목.
    Done(Step),
    /// 밖의 변경으로 건너뛴다.
    Skip(RevertSkip),
    /// 파일 연산이 실패했다. 항목별 오류로 묻는다.
    Failed(FileOpErrorKind),
    /// 하지 못했고 그 까닭은 이미 물어 답대로 했다(복사·이동 엔진이 항목별 오류를 물음, 취소).
    Unfinished,
}

impl RevertJob {
    pub(crate) fn new(
        reversal: Reversal,
        kind: OperationKind,
        steps: Vec<Step>,
        outcomes: Outcomes,
    ) -> Self {
        if let Ok(mut slots) = outcomes.lock() {
            *slots = vec![None; steps.len()];
        }
        RevertJob {
            reversal,
            kind,
            steps,
            outcomes,
        }
    }

    /// 항목 수.
    pub(super) fn root_count(&self) -> u64 {
        self.steps.len() as u64
    }

    /// 작업 스레드에서 끝까지(또는 취소될 때까지) 실행한다.
    pub(super) fn run(self, context: &Context<'_>) -> (Summary, bool) {
        if self.kind == OperationKind::BatchRename {
            return self.run_all_or_nothing(context);
        }
        let mut copier = Copier::new(context, false);
        let totals = std::sync::Arc::clone(&copier.reporter.totals);
        totals
            .items
            .store(self.steps.len() as u64, Ordering::Release);
        totals.counted.store(true, Ordering::Release);
        let mut run = Run {
            copier: &mut copier,
            outcomes: &self.outcomes,
        };
        let copies: Vec<usize> = (0..self.steps.len())
            .filter(|&i| matches!(self.steps[i], Step::Copied { .. }))
            .collect();
        if self.reversal == Reversal::Undo && !copies.is_empty() {
            run.recycle_copies(&self.steps, &copies);
        }
        let mut redone_folders = Vec::new();
        for (index, step) in self.steps.iter().enumerate() {
            if run.copier.cancelled {
                break;
            }
            if self.reversal == Reversal::Undo && matches!(step, Step::Copied { .. }) {
                continue;
            }
            if !context.control.proceed() {
                run.copier.cancelled = true;
                break;
            }
            let path = current_path(step, self.reversal);
            run.copier.reporter.current = Some(path.to_path_buf());
            if let Step::Deleted { path } = step {
                // 휴지통 삭제가 영구 삭제로 진행한 항목: 되살릴 것이 없다. 실행취소는 그것을 알리고, 기록이 다시 실행
                // 목록과 기록 사이를 함께 오가도록 항목은 「한 것」으로 넘긴다(기록에 남아 실행취소 대상을 막지 않게).
                if self.reversal == Reversal::Undo {
                    run.skip(path, RevertSkip::PermanentlyDeleted);
                }
                run.set_outcome(index, step.clone());
                run.copier.reporter.items_done += 1;
                run.copier.reporter.maybe_emit(context.control);
                continue;
            }
            loop {
                match run.attempt(step, self.reversal) {
                    Attempt::Done(changed) => {
                        if let (Reversal::Redo, Step::Recycled { item, .. }) =
                            (self.reversal, &changed)
                        {
                            run.copier.summary.recycled.push(item.clone());
                        }
                        if self.reversal == Reversal::Redo
                            && matches!(step, Step::Copied { .. } | Step::MovedFolder { .. })
                            && is_real_folder(changed.path()) == Some(true)
                        {
                            redone_folders.push(changed.clone());
                        }
                        run.set_outcome(index, changed);
                        break;
                    }
                    Attempt::Skip(reason) => {
                        run.skip(path, reason);
                        break;
                    }
                    Attempt::Unfinished => break,
                    Attempt::Failed(kind) => match run.copier.on_error(path, kind) {
                        ErrorStep::Retry => {}
                        ErrorStep::Skip | ErrorStep::Cancel => break,
                    },
                }
            }
            if run.copier.cancelled {
                break;
            }
            run.copier.reporter.items_done += 1;
            run.copier.reporter.maybe_emit(context.control);
        }
        if !run.copier.cancelled {
            run.finish_redone_folders(&redone_folders);
        }
        copier.finish()
    }
}

impl RevertJob {
    /// 일괄 이름 바꾸기 기록을 **전부 아니면 전무**로 되돌린다(다시 한다). 먼저 모든 항목을 확인하고(다룰 항목이 있고
    /// 기록한 모습과 맞는지, 되바꿀 자리에 이 기록이 비우지 않을 다른 항목이 없는지) 하나라도 걸리면 아무것도 바꾸지
    /// 않고 걸린 항목마다 [`Summary::not_reverted`]에 담는다(기록은 통째로 제자리에 남는다). 확인을 지나면 사슬·맞바꿈도
    /// 임시 이름을 거쳐 모두 바꾸고, 중간에 실패하거나 취소하면 바꾼 것을 다시 돌려 놓는다. 항목별 오류 질문은 하지
    /// 않는다.
    fn run_all_or_nothing(self, context: &Context<'_>) -> (Summary, bool) {
        let mut copier = Copier::new(context, false);
        let totals = std::sync::Arc::clone(&copier.reporter.totals);
        totals
            .items
            .store(self.steps.len() as u64, Ordering::Release);
        totals.counted.store(true, Ordering::Release);

        // 항목마다 지금 경로·옮길 경로·기록한 모습.
        let mut current = Vec::with_capacity(self.steps.len());
        let mut targets = Vec::with_capacity(self.steps.len());
        let mut stamps = Vec::with_capacity(self.steps.len());
        for step in &self.steps {
            let Step::Renamed { from, to, stamp } = step else {
                // 일괄 이름 바꾸기 기록에는 이름바꾸기만 든다.
                copier.summary.not_reverted.push(NotReverted {
                    path: step.path().to_path_buf(),
                    reason: RevertSkip::Missing,
                });
                return copier.finish();
            };
            let (now, target) = match self.reversal {
                Reversal::Undo => (to.clone(), from.clone()),
                Reversal::Redo => (from.clone(), to.clone()),
            };
            current.push(now);
            targets.push(target);
            stamps.push(*stamp);
        }

        // 확인: 아무것도 바꾸기 전에 모두 본다.
        let occupied: std::collections::HashSet<String> =
            current.iter().map(|path| path_key(path)).collect();
        for ((now, target), stamp) in current.iter().zip(&targets).zip(&stamps) {
            let reason = match Stamp::read(now) {
                None => Some(RevertSkip::Missing),
                Some(seen) if !stamp.matches(&seen) => Some(RevertSkip::Changed),
                Some(_)
                    if !occupied.contains(&path_key(target))
                        && fs::symlink_metadata(target).is_ok() =>
                {
                    Some(RevertSkip::NameTaken)
                }
                Some(_) => None,
            };
            if let Some(reason) = reason {
                copier.summary.not_reverted.push(NotReverted {
                    path: now.clone(),
                    reason,
                });
            }
        }
        if !copier.summary.not_reverted.is_empty() {
            return copier.finish();
        }

        let originals = current.clone();
        let control = context.control;
        let outcome = {
            let reporter = &mut copier.reporter;
            super::batch_rename::permute(&mut current, &targets, &mut |op| match op {
                super::batch_rename::Op::Before => control.proceed(),
                super::batch_rename::Op::Placed(path) => {
                    reporter.items_done += 1;
                    reporter.current = Some(path.to_path_buf());
                    reporter.maybe_emit(control);
                    true
                }
            })
        };
        match outcome {
            Ok(()) => {
                // 대소문자만 바꾼 항목처럼 이미 제자리였던 항목도 한 것으로 센다.
                copier.reporter.items_done = self.steps.len() as u64;
                if let Ok(mut slots) = self.outcomes.lock() {
                    for (index, step) in self.steps.iter().enumerate() {
                        let Step::Renamed { from, to, stamp } = step else {
                            continue;
                        };
                        let stamp = Stamp::read(&targets[index]).unwrap_or(*stamp);
                        if let Some(slot) = slots.get_mut(index) {
                            *slot = Some(Step::Renamed {
                                from: from.clone(),
                                to: to.clone(),
                                stamp,
                            });
                        }
                    }
                }
            }
            Err(stop) => {
                match stop {
                    super::batch_rename::Stop::Failed { path, kind } => {
                        copier.summary.failures.push(ItemFailure { source: path, kind });
                    }
                    super::batch_rename::Stop::Cancelled => copier.cancelled = true,
                }
                for (path, kind) in super::batch_rename::roll_back(&mut current, &originals) {
                    copier.summary.failures.push(ItemFailure { source: path, kind });
                }
                // 되돌리지 못하고 반대편 자리에 놓인 항목은 그쪽 목록으로 보낸다(기록과 디스크가 어긋나지 않게).
                if let Ok(mut slots) = self.outcomes.lock() {
                    for (index, step) in self.steps.iter().enumerate() {
                        if current[index].as_os_str() != targets[index].as_os_str() {
                            continue;
                        }
                        if let (Step::Renamed { from, to, .. }, Some(stamp)) =
                            (step, Stamp::read(&targets[index]))
                            && let Some(slot) = slots.get_mut(index)
                        {
                            *slot = Some(Step::Renamed {
                                from: from.clone(),
                                to: to.clone(),
                                stamp,
                            });
                        }
                    }
                }
                copier.reporter.items_done = 0;
            }
        }
        copier.finish()
    }
}

/// 도는 실행취소·다시 실행: 엔진(질문·진행)과 결과를 적는 곳.
struct Run<'r, 'a> {
    copier: &'r mut Copier<'a>,
    outcomes: &'r Outcomes,
}

/// 실행취소가 복사본 하나를 본 결과.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fate {
    /// 기록한 그대로다(폴더면 안도 모두 그대로): 휴지통으로 보낸다.
    Intact,
    /// 밖의 변경으로 건너뛴다.
    Skip(RevertSkip),
    /// 살피다 항목별 오류를 물어 답대로 했다(건너뜀·취소).
    Asked,
}

impl Run<'_, '_> {
    fn set_outcome(&self, index: usize, step: Step) {
        if let Ok(mut slots) = self.outcomes.lock()
            && let Some(slot) = slots.get_mut(index)
        {
            *slot = Some(step);
        }
    }

    fn skip(&mut self, path: &Path, reason: RevertSkip) {
        self.copier.summary.not_reverted.push(NotReverted {
            path: path.to_path_buf(),
            reason,
        });
    }

    fn attempt(&mut self, step: &Step, reversal: Reversal) -> Attempt {
        match (step, reversal) {
            (Step::Renamed { from, to, stamp }, Reversal::Undo) => rename_back(to, from, stamp)
                .map_or_else(Into::into, |stamp| {
                    Attempt::Done(Step::Renamed {
                        from: from.clone(),
                        to: to.clone(),
                        stamp,
                    })
                }),
            (Step::Renamed { from, to, stamp }, Reversal::Redo) => rename_back(from, to, stamp)
                .map_or_else(Into::into, |stamp| {
                    Attempt::Done(Step::Renamed {
                        from: from.clone(),
                        to: to.clone(),
                        stamp,
                    })
                }),
            (Step::CreatedFolder { path, stamp }, Reversal::Undo) => remove_new_folder(path)
                .map_or_else(Into::into, |()| {
                    Attempt::Done(Step::CreatedFolder {
                        path: path.clone(),
                        stamp: *stamp,
                    })
                }),
            (Step::CreatedFolder { path, .. }, Reversal::Redo) => {
                create_again(path).map_or_else(Into::into, |stamp| {
                    Attempt::Done(Step::CreatedFolder {
                        path: path.clone(),
                        stamp,
                    })
                })
            }
            (
                Step::Copied {
                    source,
                    path,
                    stamp,
                },
                Reversal::Redo,
            ) => self.copy_again(source, path, stamp),
            // 복사의 실행취소는 `recycle_copies`가 한꺼번에 한다.
            (Step::Copied { .. }, Reversal::Undo) => Attempt::Unfinished,
            (Step::Moved { from, to, stamp }, Reversal::Undo) => self
                .move_between(to, from, stamp)
                .map_or_else(Into::into, |s| {
                    Attempt::Done(Step::Moved {
                        from: from.clone(),
                        to: to.clone(),
                        stamp: s,
                    })
                }),
            (Step::Moved { from, to, stamp }, Reversal::Redo) => self
                .move_between(from, to, stamp)
                .map_or_else(Into::into, |s| {
                    Attempt::Done(Step::Moved {
                        from: from.clone(),
                        to: to.clone(),
                        stamp: s,
                    })
                }),
            (
                Step::MovedFolder {
                    from, to, created, ..
                },
                Reversal::Undo,
            ) => restore_moved_folder(from, to, *created).map_or_else(Into::into, |stamp| {
                Attempt::Done(Step::MovedFolder {
                    from: from.clone(),
                    to: to.clone(),
                    created: *created,
                    stamp,
                })
            }),
            (
                Step::Overwrote {
                    source,
                    path,
                    moved,
                    stamp,
                    replaced,
                    replaced_stamp,
                },
                Reversal::Undo,
            ) => self.undo_overwrite(source, path, *moved, stamp, replaced, replaced_stamp),
            (
                Step::Overwrote {
                    source,
                    path,
                    moved,
                    stamp,
                    replaced_stamp,
                    ..
                },
                Reversal::Redo,
            ) => self.redo_overwrite(source, path, *moved, stamp, replaced_stamp),
            (Step::MovedFolder { from, to, .. }, Reversal::Redo) => prepare_moved_folder(from, to)
                .map_or_else(Into::into, |(stamp, made)| {
                    Attempt::Done(Step::MovedFolder {
                        from: from.clone(),
                        to: to.clone(),
                        created: made,
                        stamp,
                    })
                }),
            (Step::Recycled { path, item, .. }, Reversal::Undo) => restore_recycled(path, item),
            (Step::Recycled { path, stamp, .. }, Reversal::Redo) => recycle_again(path, stamp),
            // 영구 삭제한 항목은 `run`이 따로 다룬다(되돌릴 수 없는 기록의 것은 여기 오지 않는다).
            (Step::Deleted { .. }, _) => Attempt::Skip(RevertSkip::PermanentlyDeleted),
        }
    }

    /// 다시 실행으로 다시 만든 폴더들을 마무리한다(안의 항목을 다 한 뒤, 안쪽부터): 다시 복사한 폴더는 시각·속성을
    /// 원본과 맞추고, 다시 옮긴 폴더는 비게 된 원래 폴더를 지운다(처음 이동처럼, 빈 폴더만).
    fn finish_redone_folders(&mut self, folders: &[Step]) {
        for step in folders.iter().rev() {
            match step {
                Step::Copied { source, path, .. } => self.copier.finish_folder(source, path, None),
                Step::MovedFolder { from, .. } => {
                    let _ = file_ops::remove_empty_folder(from);
                }
                _ => {}
            }
        }
    }

    /// 다시 실행: 같은 원본 `source`(기록한 모습 `stamp`)를 `path`에 다시 복사한다. 폴더는 만들기만 하고(안의 항목은
    /// 따로 든다) 시각·속성은 끝에 맞춘다.
    fn copy_again(&mut self, source: &Path, path: &Path, stamp: &Stamp) -> Attempt {
        let Some(now) = Stamp::read(source) else {
            return Attempt::Skip(RevertSkip::Missing);
        };
        if !stamp.matches(&now) {
            return Attempt::Skip(RevertSkip::Changed);
        }
        if fs::symlink_metadata(path).is_ok() {
            return Attempt::Skip(RevertSkip::NameTaken);
        }
        // 담을 폴더가 없다(그 폴더를 다시 만들지 못했다).
        if path.parent().is_some_and(|parent| !parent.is_dir()) {
            return Attempt::Skip(RevertSkip::Missing);
        }
        let is_link = fs::symlink_metadata(source).is_ok_and(|m| m.file_type().is_symlink());
        if now.is_folder && !is_link {
            return match file_ops::create_folder(path) {
                Ok(()) => Stamp::read(path).map_or(Attempt::Skip(RevertSkip::Missing), |stamp| {
                    Attempt::Done(Step::Copied {
                        source: source.to_path_buf(),
                        path: path.to_path_buf(),
                        stamp,
                    })
                }),
                Err(error) if error.kind() == FileOpErrorKind::TargetExists => {
                    Attempt::Skip(RevertSkip::NameTaken)
                }
                Err(error) => Attempt::Failed(error.kind()),
            };
        }
        let failures = self.copier.summary.failures.len();
        let skipped = self.copier.summary.conflicts_skipped;
        if is_link {
            self.copier.copy_link(source, path);
        } else {
            self.copier
                .reporter
                .totals
                .bytes
                .fetch_add(now.size, Ordering::AcqRel);
            self.copier.copy_file(source, path, now.size);
        }
        let failed = self.copier.cancelled
            || self.copier.summary.failures.len() > failures
            || self.copier.summary.conflicts_skipped > skipped;
        match Stamp::read(path) {
            Some(stamp) if !failed => Attempt::Done(Step::Copied {
                source: source.to_path_buf(),
                path: path.to_path_buf(),
                stamp,
            }),
            _ => Attempt::Unfinished,
        }
    }

    /// 항목 하나를 `source`(기록한 모습 `stamp`)에서 `target`으로 옮긴다: 이름 바꾸기, 다른 볼륨이면 이동 엔진의 복사
    /// 뒤 원본 삭제. 옮긴 뒤의 모습을 돌려준다. 옮길 자리의 상위 폴더가 없으면(이동이 비워 지운 원래 폴더) 만든다.
    fn move_between(
        &mut self,
        source: &Path,
        target: &Path,
        stamp: &Stamp,
    ) -> Result<Stamp, Refusal> {
        let now = Stamp::read(source).ok_or(Refusal::Skip(RevertSkip::Missing))?;
        if !stamp.matches(&now) {
            return Err(Refusal::Skip(RevertSkip::Changed));
        }
        if !same_entry(source, target) && fs::symlink_metadata(target).is_ok() {
            return Err(Refusal::Skip(RevertSkip::NameTaken));
        }
        if let Some(parent) = target.parent()
            && !parent.as_os_str().is_empty()
            && fs::symlink_metadata(parent).is_err()
        {
            fs::create_dir_all(parent).map_err(|error| Refusal::Failed(error_kind(&error)))?;
        }
        match file_ops::rename(source, target) {
            Ok(()) => Ok(Stamp::read(target).unwrap_or(now)),
            Err(error) => match error.kind() {
                FileOpErrorKind::CrossVolume => self.move_across(source, target, &now),
                FileOpErrorKind::TargetExists => Err(Refusal::Skip(RevertSkip::NameTaken)),
                FileOpErrorKind::SourceMissing => Err(Refusal::Skip(RevertSkip::Missing)),
                kind => Err(Refusal::Failed(kind)),
            },
        }
    }

    /// 볼륨을 넘어 옮긴다: 이동 작업과 같은 엔진(복사를 마친 원본을 지우고, 안을 다 옮긴 빈 폴더만 지움). 엔진이
    /// 항목별 오류를 묻고 답대로 하므로, 다 옮기지 못했으면 [`Refusal::Asked`]다.
    fn move_across(&mut self, source: &Path, target: &Path, now: &Stamp) -> Result<Stamp, Refusal> {
        let is_link = fs::symlink_metadata(source).is_ok_and(|m| m.file_type().is_symlink());
        let totals = std::sync::Arc::clone(&self.copier.reporter.totals);
        let items_total = totals.items.load(Ordering::Acquire);
        let items_done = self.copier.reporter.items_done;
        self.copier.moving = true;
        if now.is_folder && !is_link {
            self.copier
                .run_pipeline(vec![(source.to_path_buf(), Some(target.to_path_buf()))]);
        } else if is_link {
            self.copier.copy_link(source, target);
        } else {
            totals.bytes.fetch_add(now.size, Ordering::AcqRel);
            self.copier.copy_file(source, target, now.size);
        }
        self.copier.moving = false;
        // 엔진이 센 항목은 이 항목 하나로 친다(진행의 항목 수가 기록의 항목 수와 맞게).
        totals.items.store(items_total, Ordering::Release);
        self.copier.reporter.items_done = items_done;
        if self.copier.cancelled || fs::symlink_metadata(source).is_ok() {
            return Err(Refusal::Asked);
        }
        Stamp::read(target).ok_or(Refusal::Asked)
    }

    /// 덮어쓰기의 실행취소: `path`의 새 파일(기록한 모습 `stamp`)을 치우고(복사면 휴지통으로 보내고, 이동이면
    /// `source`로 되옮김) 덮어쓰기 전에 휴지통으로 보낸 옛 파일 `replaced`를 `path`로 복원한다.
    ///
    /// 새 파일이 없거나 바뀌었거나 옛 파일의 휴지통 항목이 없으면(휴지통을 비움) 아무것도 하지 않고 건너뛴다. 새
    /// 파일을 치운 뒤 옛 파일을 복원하지 못하면 새 파일을 제자리로 되돌린다(휴지통에서 복원하거나 되옮김): 그래서
    /// 실행취소 전 상태로 남아 고친 뒤 다시 해 볼 수 있다. 되돌리기마저 실패하면(볼륨을 넘어 되옮긴 새 파일, 그 사이
    /// 그 자리를 다른 항목이 차지함) 새 파일은 `source`나 휴지통에 남는다(영구히 지우지 않는다).
    fn undo_overwrite(
        &mut self,
        source: &Path,
        path: &Path,
        moved: bool,
        stamp: &Stamp,
        replaced: &RecycledItem,
        replaced_stamp: &Stamp,
    ) -> Attempt {
        let Some(now) = Stamp::read(path) else {
            return Attempt::Skip(RevertSkip::Missing);
        };
        if !stamp.matches(&now) {
            return Attempt::Skip(RevertSkip::Changed);
        }
        if fs::symlink_metadata(replaced.recycled()).is_err() {
            return Attempt::Skip(RevertSkip::RecycledMissing);
        }
        // 새 파일을 치운다.
        let cleared = if moved {
            match self.move_between(path, source, stamp) {
                Ok(moved_back) => Cleared::MovedBack(moved_back),
                Err(refusal) => return refusal.into(),
            }
        } else {
            match recycle::recycle(&[path.to_path_buf()]).pop() {
                Some(RecycleOutcome::Recycled(item)) => Cleared::Recycled(item),
                Some(RecycleOutcome::NotRecyclable) => {
                    return Attempt::Skip(RevertSkip::NotRecyclable);
                }
                Some(RecycleOutcome::Failed(error)) => return Attempt::Failed(error.kind()),
                Some(RecycleOutcome::NotAttempted) | None => {
                    return Attempt::Failed(FileOpErrorKind::Other);
                }
            }
        };
        // 옛 파일을 되살린다.
        match recycle::restore(replaced) {
            Ok(_) => {
                let new_stamp = match cleared {
                    Cleared::Recycled(item) => {
                        self.copier.summary.recycled.push(item);
                        *stamp
                    }
                    Cleared::MovedBack(moved_back) => moved_back,
                };
                Attempt::Done(Step::Overwrote {
                    source: source.to_path_buf(),
                    path: path.to_path_buf(),
                    moved,
                    stamp: new_stamp,
                    replaced: replaced.clone(),
                    replaced_stamp: Stamp::read(path).unwrap_or(*replaced_stamp),
                })
            }
            Err(error) => {
                // 옛 파일을 되살리지 못했다: 새 파일을 제자리로 되돌려 실행취소 전 상태로 둔다.
                match cleared {
                    Cleared::Recycled(item) => {
                        if recycle::restore(&item).is_err() {
                            self.copier.summary.recycled.push(item);
                        }
                    }
                    Cleared::MovedBack(_) => {
                        let _ = file_ops::rename(source, path);
                    }
                }
                match error.kind() {
                    FileOpErrorKind::SourceMissing => Attempt::Skip(RevertSkip::RecycledMissing),
                    FileOpErrorKind::TargetExists => Attempt::Skip(RevertSkip::NameTaken),
                    kind => Attempt::Failed(kind),
                }
            }
        }
    }

    /// 덮어쓰기의 다시 실행: 다시 덮어쓴다. `path`에 돌아와 있는 옛 파일(기록한 모습 `replaced_stamp`)을 다시
    /// 휴지통으로 보내고 그 자리에 `source`(기록한 모습 `stamp`)를 다시 복사하거나(복사) 다시 옮긴다(이동). 처음
    /// 덮어쓰기와 같은 엔진이다(임시 이름에 다 쓴 뒤 놓기).
    ///
    /// 원본이나 옛 파일이 없거나 바뀌었으면 건너뛴다. 영구 덮어쓰기는 하지 않는다: 휴지통이 없는 위치이거나 셸이
    /// 휴지통에 넣지 않으려 하면 [`RevertSkip::NotRecyclable`]로 건너뛴다.
    fn redo_overwrite(
        &mut self,
        source: &Path,
        path: &Path,
        moved: bool,
        stamp: &Stamp,
        replaced_stamp: &Stamp,
    ) -> Attempt {
        let Some(now) = Stamp::read(source) else {
            return Attempt::Skip(RevertSkip::Missing);
        };
        if !stamp.matches(&now) {
            return Attempt::Skip(RevertSkip::Changed);
        }
        let Some(old) = Stamp::read(path) else {
            return Attempt::Skip(RevertSkip::Missing);
        };
        if !replaced_stamp.matches(&old) {
            return Attempt::Skip(RevertSkip::Changed);
        }
        if !recycle::has_recycle_bin(path) {
            return Attempt::Skip(RevertSkip::NotRecyclable);
        }
        let is_link = fs::symlink_metadata(source).is_ok_and(|m| m.file_type().is_symlink());
        let failures = self.copier.summary.failures.len();
        let skipped = self.copier.summary.conflicts_skipped;
        self.copier.permanent_overwrite = false;
        self.copier.refused = false;
        self.copier.last_replaced = None;
        let mut copy = !moved;
        if moved {
            match self.copier.move_over(source, path) {
                FileOutcome::Copied { .. } => {}
                FileOutcome::Failed(FileOpErrorKind::CrossVolume) => copy = true,
                FileOutcome::Refused => self.copier.refused = true,
                FileOutcome::Failed(kind) => {
                    self.copier.permanent_overwrite = true;
                    return Attempt::Failed(kind);
                }
                FileOutcome::Cancelled | FileOutcome::Skipped | FileOutcome::Exists => {
                    self.copier.permanent_overwrite = true;
                    return Attempt::Failed(FileOpErrorKind::TargetExists);
                }
            }
        }
        if copy {
            // 다른 볼륨으로 옮기기(복사 뒤 원본 삭제)와 복사는 복사의 덮어쓰기로: 이미 정한 답으로 묻지 않는다.
            self.copier.moving = moved;
            self.copier.preset = Some(Resolution::Overwrite);
            if is_link {
                self.copier.copy_link(source, path);
            } else {
                self.copier
                    .reporter
                    .totals
                    .bytes
                    .fetch_add(now.size, Ordering::AcqRel);
                self.copier.copy_file(source, path, now.size);
            }
            self.copier.preset = None;
            self.copier.moving = false;
        }
        self.copier.permanent_overwrite = true;
        if self.copier.refused {
            self.copier.refused = false;
            return Attempt::Skip(RevertSkip::NotRecyclable);
        }
        let failed = self.copier.cancelled
            || self.copier.summary.failures.len() > failures
            || self.copier.summary.conflicts_skipped > skipped;
        match (self.copier.last_replaced.take(), Stamp::read(path)) {
            (Some((item, old)), Some(stamp)) if !failed => Attempt::Done(Step::Overwrote {
                source: source.to_path_buf(),
                path: path.to_path_buf(),
                moved,
                stamp,
                replaced: item,
                replaced_stamp: old,
            }),
            _ => Attempt::Unfinished,
        }
    }

    /// 복사의 실행취소: `indices`의 복사본들(기록의 거꾸로, 안쪽 항목이 바깥 폴더보다 먼저)을 살펴 그대로인 것을
    /// 휴지통으로 보낸다. 그대로인 폴더 안의 항목은 폴더와 함께 간다.
    fn recycle_copies(&mut self, steps: &[Step], indices: &[usize]) {
        let index_of: HashMap<String, usize> = indices
            .iter()
            .map(|&i| (path_key(steps[i].path()), i))
            .collect();
        let mut fates: HashMap<usize, Fate> = HashMap::new();
        for &i in indices {
            if !self.copier.context.control.proceed() {
                self.copier.cancelled = true;
                return;
            }
            let Step::Copied { path, stamp, .. } = &steps[i] else {
                continue;
            };
            self.copier.reporter.current = Some(path.clone());
            let fate = self.judge(path, stamp, &index_of, &fates);
            if self.copier.cancelled {
                return;
            }
            fates.insert(i, fate);
        }
        // 맨 위의 그대로인 항목마다 함께 갈 항목들(자신과 그 아래). 바깥 폴더부터 보아 안쪽은 바깥의 무리에 든다.
        let mut group_of: HashMap<usize, usize> = HashMap::new();
        let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
        let mut roots = Vec::new();
        for &i in indices.iter().rev() {
            match fates.get(&i) {
                Some(Fate::Intact) => {}
                Some(Fate::Skip(reason)) => {
                    let path = steps[i].path().to_path_buf();
                    self.skip(&path, *reason);
                    self.copier.reporter.items_done += 1;
                    continue;
                }
                Some(Fate::Asked) => {
                    self.copier.reporter.items_done += 1;
                    continue;
                }
                None => continue,
            }
            let parent_group = steps[i]
                .path()
                .parent()
                .and_then(|parent| index_of.get(&path_key(parent)))
                .and_then(|parent| group_of.get(parent))
                .copied();
            let root = parent_group.unwrap_or_else(|| {
                roots.push(i);
                i
            });
            group_of.insert(i, root);
            groups.entry(root).or_default().push(i);
        }
        self.copier.reporter.maybe_emit(self.copier.context.control);
        let mut pending: VecDeque<usize> = roots.into();
        while !pending.is_empty() {
            if !self.copier.context.control.proceed() {
                self.copier.cancelled = true;
                return;
            }
            let chunk: Vec<usize> = pending.drain(..pending.len().min(RECYCLE_CHUNK)).collect();
            let paths: Vec<PathBuf> = chunk
                .iter()
                .map(|&i| steps[i].path().to_path_buf())
                .collect();
            self.copier.reporter.current = paths.first().cloned();
            let results = recycle::recycle(&paths);
            let mut again = Vec::new();
            let mut settled = false;
            for (position, &root) in chunk.iter().enumerate() {
                let members = groups.get(&root).map_or(1, Vec::len) as u64;
                match results.get(position) {
                    Some(RecycleOutcome::Recycled(item)) => {
                        settled = true;
                        for &member in groups.get(&root).into_iter().flatten() {
                            self.set_outcome(member, steps[member].clone());
                        }
                        self.copier.summary.recycled.push(item.clone());
                        self.copier.reporter.items_done += members;
                    }
                    Some(RecycleOutcome::NotRecyclable) => {
                        settled = true;
                        // 영구 삭제하지 않는다: 그 항목(폴더면 그 아래 전부)은 남고 기록에도 남는다.
                        self.skip(&paths[position], RevertSkip::NotRecyclable);
                        self.copier.reporter.items_done += members;
                    }
                    Some(RecycleOutcome::Failed(error)) => {
                        settled = true;
                        match self.copier.on_error(&paths[position], error.kind()) {
                            ErrorStep::Retry => again.push(root),
                            ErrorStep::Skip => self.copier.reporter.items_done += members,
                            ErrorStep::Cancel => return,
                        }
                    }
                    Some(RecycleOutcome::NotAttempted) | None => again.push(root),
                }
            }
            if !settled {
                // 셸이 아무 항목도 다루지 않았다: 같은 호출을 되풀이하지 않게 첫 항목을 오류로 묻는다.
                let root = again.remove(0);
                let path = steps[root].path().to_path_buf();
                let members = groups.get(&root).map_or(1, Vec::len) as u64;
                match self.copier.on_error(&path, FileOpErrorKind::Other) {
                    ErrorStep::Retry => again.insert(0, root),
                    ErrorStep::Skip => self.copier.reporter.items_done += members,
                    ErrorStep::Cancel => return,
                }
            }
            for root in again.into_iter().rev() {
                pending.push_front(root);
            }
            self.copier.reporter.maybe_emit(self.copier.context.control);
        }
    }

    /// 복사본 하나가 기록한 그대로인지 본다. 폴더(링크가 아닌)는 안의 항목이 모두 이 작업의 복사본이고 그대로여야
    /// 그대로다(안쪽 항목은 이미 보았다).
    fn judge(
        &mut self,
        path: &Path,
        stamp: &Stamp,
        index_of: &HashMap<String, usize>,
        fates: &HashMap<usize, Fate>,
    ) -> Fate {
        let Ok(metadata) = fs::symlink_metadata(path) else {
            return Fate::Skip(RevertSkip::Missing);
        };
        let Some(now) = Stamp::read(path) else {
            return Fate::Skip(RevertSkip::Missing);
        };
        if !stamp.matches(&now) {
            return Fate::Skip(RevertSkip::Changed);
        }
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Fate::Intact;
        }
        let listing = loop {
            match fs::read_dir(path).and_then(|listing| {
                listing
                    .map(|entry| entry.map(|e| e.path()))
                    .collect::<std::io::Result<Vec<_>>>()
            }) {
                Ok(listing) => break listing,
                Err(error) => match self.copier.on_error(path, error_kind(&error)) {
                    ErrorStep::Retry => {}
                    ErrorStep::Skip | ErrorStep::Cancel => return Fate::Asked,
                },
            }
        };
        let intact = listing.iter().all(|child| {
            index_of
                .get(&path_key(child))
                .is_some_and(|i| fates.get(i) == Some(&Fate::Intact))
        });
        if intact {
            Fate::Intact
        } else {
            Fate::Skip(RevertSkip::NotEmpty)
        }
    }
}

/// 그 방향에서 다룰 항목이 지금 있어야 하는 경로.
fn current_path(step: &Step, reversal: Reversal) -> &Path {
    match (step, reversal) {
        (
            Step::Renamed { to, .. } | Step::Moved { to, .. } | Step::MovedFolder { to, .. },
            Reversal::Undo,
        ) => to,
        (
            Step::Renamed { from, .. } | Step::Moved { from, .. } | Step::MovedFolder { from, .. },
            Reversal::Redo,
        ) => from,
        (
            Step::CreatedFolder { path, .. } | Step::Deleted { path } | Step::Recycled { path, .. },
            _,
        )
        | (Step::Copied { path, .. } | Step::Overwrote { path, .. }, Reversal::Undo) => path,
        (Step::Copied { source, .. } | Step::Overwrote { source, .. }, Reversal::Redo) => source,
    }
}

/// 덮어쓰기의 실행취소가 새 파일을 치운 방법.
enum Cleared {
    /// 복사본을 휴지통으로 보냈다.
    Recycled(RecycledItem),
    /// 옮겨 온 파일을 원래 자리로 되옮겼다(그 뒤의 모습).
    MovedBack(Stamp),
}

/// 하지 못한 까닭: 밖의 변경(건너뜀), 파일 연산의 실패(물음), 이미 물어 답대로 함.
enum Refusal {
    Skip(RevertSkip),
    Failed(FileOpErrorKind),
    Asked,
}

impl From<Refusal> for Attempt {
    fn from(refusal: Refusal) -> Self {
        match refusal {
            Refusal::Skip(reason) => Attempt::Skip(reason),
            Refusal::Failed(kind) => Attempt::Failed(kind),
            Refusal::Asked => Attempt::Unfinished,
        }
    }
}

/// 같은 경로인가(Windows는 대소문자를 가리지 않는다): 대소문자만 바꾼 이름바꾸기의 두 경로는 같은 항목이다.
fn same_entry(a: &Path, b: &Path) -> bool {
    super::same_path(a, b)
}

/// `source`(기록한 모습 `stamp`)의 이름을 `target`으로 바꾼다. 바꾼 뒤의 모습을 돌려준다.
fn rename_back(source: &Path, target: &Path, stamp: &Stamp) -> Result<Stamp, Refusal> {
    let now = Stamp::read(source).ok_or(Refusal::Skip(RevertSkip::Missing))?;
    if !stamp.matches(&now) {
        return Err(Refusal::Skip(RevertSkip::Changed));
    }
    if !same_entry(source, target) && fs::symlink_metadata(target).is_ok() {
        return Err(Refusal::Skip(RevertSkip::NameTaken));
    }
    match file_ops::rename(source, target) {
        Ok(()) => Ok(Stamp::read(target).unwrap_or(now)),
        Err(error) => Err(match error.kind() {
            FileOpErrorKind::TargetExists => Refusal::Skip(RevertSkip::NameTaken),
            FileOpErrorKind::SourceMissing => Refusal::Skip(RevertSkip::Missing),
            kind => Refusal::Failed(kind),
        }),
    }
}

/// 휴지통 삭제의 실행취소: 휴지통 항목 `item`을 원래 자리 `path`로 복원한다. 원래 자리에 같은 이름이 있으면
/// 덮어쓰지 않고, 휴지통 항목이 없으면(휴지통을 비움) 건너뛴다.
fn restore_recycled(path: &Path, item: &RecycledItem) -> Attempt {
    if fs::symlink_metadata(item.recycled()).is_err() {
        return Attempt::Skip(RevertSkip::RecycledMissing);
    }
    if fs::symlink_metadata(path).is_ok() {
        return Attempt::Skip(RevertSkip::NameTaken);
    }
    match recycle::restore(item) {
        // 미리 본 뒤 셸이 옮기기 직전에 같은 이름이 생기면 셸이 다른 이름으로 되돌린다: 다시 실행은 그 항목을 다룬다.
        Ok(restored) => match Stamp::read(&restored) {
            Some(stamp) => Attempt::Done(Step::Recycled {
                path: restored,
                item: item.clone(),
                stamp,
            }),
            None => Attempt::Skip(RevertSkip::Missing),
        },
        Err(error) => match error.kind() {
            FileOpErrorKind::SourceMissing => Attempt::Skip(RevertSkip::RecycledMissing),
            FileOpErrorKind::TargetExists => Attempt::Skip(RevertSkip::NameTaken),
            kind => Attempt::Failed(kind),
        },
    }
}

/// 휴지통 삭제의 다시 실행: 복원해 둔 `path`(복원한 뒤의 모습 `stamp`)를 다시 휴지통으로 보낸다. 없거나 바뀌었으면
/// 건너뛰고, 휴지통에 넣을 수 없으면 영구 삭제하지 않고 건너뛴다.
fn recycle_again(path: &Path, stamp: &Stamp) -> Attempt {
    let Some(now) = Stamp::read(path) else {
        return Attempt::Skip(RevertSkip::Missing);
    };
    if !stamp.matches(&now) {
        return Attempt::Skip(RevertSkip::Changed);
    }
    match recycle::recycle(&[path.to_path_buf()]).pop() {
        Some(RecycleOutcome::Recycled(item)) => Attempt::Done(Step::Recycled {
            path: path.to_path_buf(),
            item,
            stamp: now,
        }),
        Some(RecycleOutcome::NotRecyclable) => Attempt::Skip(RevertSkip::NotRecyclable),
        Some(RecycleOutcome::Failed(error)) if error.kind() == FileOpErrorKind::SourceMissing => {
            Attempt::Skip(RevertSkip::Missing)
        }
        Some(RecycleOutcome::Failed(error)) => Attempt::Failed(error.kind()),
        Some(RecycleOutcome::NotAttempted) | None => Attempt::Failed(FileOpErrorKind::Other),
    }
}

/// 실제 폴더(링크가 아닌)인가.
fn is_real_folder(path: &Path) -> Option<bool> {
    fs::symlink_metadata(path)
        .ok()
        .map(|m| m.is_dir() && !m.file_type().is_symlink())
}

/// 만든 폴더를 비어 있으면 지운다.
fn remove_new_folder(path: &Path) -> Result<(), Refusal> {
    match is_real_folder(path) {
        None => return Err(Refusal::Skip(RevertSkip::Missing)),
        Some(false) => return Err(Refusal::Skip(RevertSkip::Changed)),
        Some(true) => {}
    }
    remove_if_empty(path)
}

/// 폴더를 비어 있을 때만 지운다(안에 무엇이 있으면 [`RevertSkip::NotEmpty`]).
fn remove_if_empty(path: &Path) -> Result<(), Refusal> {
    let mut listing = fs::read_dir(path).map_err(|error| Refusal::Failed(error_kind(&error)))?;
    if listing.next().is_some() {
        return Err(Refusal::Skip(RevertSkip::NotEmpty));
    }
    drop(listing);
    match file_ops::remove_empty_folder(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == FileOpErrorKind::SourceMissing => {
            Err(Refusal::Skip(RevertSkip::Missing))
        }
        // 그 사이 안에 무언가 생겼으면 지우지 못한다(빈 폴더만 지운다).
        Err(_) if fs::read_dir(path).is_ok_and(|mut l| l.next().is_some()) => {
            Err(Refusal::Skip(RevertSkip::NotEmpty))
        }
        Err(error) => Err(Refusal::Failed(error.kind())),
    }
}

/// 지웠던 새 폴더를 다시 만든다. 그 자리에 무엇이 있으면 건너뛴다.
fn create_again(path: &Path) -> Result<Stamp, Refusal> {
    if fs::symlink_metadata(path).is_ok() {
        return Err(Refusal::Skip(RevertSkip::NameTaken));
    }
    match file_ops::create_folder(path) {
        Ok(()) => Stamp::read(path).ok_or(Refusal::Skip(RevertSkip::Missing)),
        Err(error) if error.kind() == FileOpErrorKind::TargetExists => {
            Err(Refusal::Skip(RevertSkip::NameTaken))
        }
        Err(error) => Err(Refusal::Failed(error.kind())),
    }
}

/// 이동의 실행취소에서 안의 항목을 하나씩 옮긴 폴더(안의 항목은 이미 되옮겼다): 원래 폴더 `from`이 없으면(이동이
/// 비워 지웠다) 다시 만들고, 이동이 만든 `to`(`created`)는 비었으면 지운다. `from`의 모습을 돌려준다.
fn restore_moved_folder(from: &Path, to: &Path, created: bool) -> Result<Stamp, Refusal> {
    match is_real_folder(from) {
        Some(true) => {}
        Some(false) => return Err(Refusal::Skip(RevertSkip::NameTaken)),
        None => fs::create_dir_all(from).map_err(|error| Refusal::Failed(error_kind(&error)))?,
    }
    if created {
        match is_real_folder(to) {
            Some(true) => remove_if_empty(to)?,
            // 이미 없다(밖에서 지웠다): 지울 것이 없다.
            None => {}
            Some(false) => return Err(Refusal::Skip(RevertSkip::Changed)),
        }
    }
    Stamp::read(from).ok_or(Refusal::Skip(RevertSkip::Missing))
}

/// 이동의 다시 실행에서 안의 항목을 하나씩 옮길 폴더(안의 항목은 뒤에 옮긴다): 대상 `to`가 없으면 만들고(처음
/// 이동이 만들었던 것처럼) 같은 이름 폴더가 있으면 합친다. 원래 폴더 `from`은 안의 항목을 다 옮긴 뒤 비면 지운다.
/// `to`의 모습과 이번에 만들었는지를 돌려준다(그 사이 밖에서 생긴 폴더는 이동이 만든 것이 아니라 실행취소가 지우지
/// 않게).
fn prepare_moved_folder(from: &Path, to: &Path) -> Result<(Stamp, bool), Refusal> {
    if is_real_folder(from) != Some(true) {
        return Err(Refusal::Skip(RevertSkip::Missing));
    }
    let made = match is_real_folder(to) {
        Some(true) => false,
        Some(false) => return Err(Refusal::Skip(RevertSkip::NameTaken)),
        None => match file_ops::create_folder(to) {
            Ok(()) => true,
            Err(error) if error.kind() == FileOpErrorKind::TargetExists => {
                return Err(Refusal::Skip(RevertSkip::NameTaken));
            }
            Err(error) => return Err(Refusal::Failed(error.kind())),
        },
    };
    let stamp = Stamp::read(to).ok_or(Refusal::Skip(RevertSkip::Missing))?;
    Ok((stamp, made))
}
