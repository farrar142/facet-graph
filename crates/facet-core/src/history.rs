//! **작업 기록**과 **실행취소**·**다시 실행**의 상태(ADR-0005).
//!
//! 작업 기록([`History`])은 앱 전체에 하나이고([`crate::window::App::history`]) 메모리에만 있다(디스크에 남기지
//! 않는다). 끝난 작업만 들어가며 최근 [`HISTORY_LIMIT`]개까지 남고, 넘치면 가장 오래된 기록이 빠진다. 기록
//! ([`Record`])마다 작업 종류([`OperationKind`]), 항목별 결과(되돌릴 단위 [`Step`]: 원래 경로·새 경로·만든 항목과
//! 그때의 크기·수정 시각 [`Stamp`]), 되돌릴 수 없으면 그 까닭([`NotUndoable`])을 갖는다. 이름바꾸기·새 폴더와
//! 복사·이동(폴더 안의 항목도 하나씩, [`Step::Copied`]·[`Step::Moved`]·[`Step::MovedFolder`], 원래 대상을 휴지통으로
//! 보내고 덮어쓴 파일은 그 휴지통 항목과 함께 [`Step::Overwrote`])이 기록된다. **영구 삭제**는 되돌릴 수 없는
//! 기록([`NotUndoable::PermanentDelete`], 넘긴 원본마다 [`Step::Deleted`])으로 남는다. **휴지통 삭제**는 휴지통에
//! 보낸 항목마다 [`Step::Recycled`](원래 경로와 휴지통 항목)로 남고, 실행취소는 휴지통에서 복원한다. **일괄 이름
//! 바꾸기**는 기록 하나(바꾼 항목마다 [`Step::Renamed`], [`OperationKind::BatchRename`])로 남고 실행취소·다시 실행이
//! 전부 아니면 전무다.
//!
//! - **실행취소 대상**은 끝난 작업 가운데 마지막 기록이다([`History::undo_target`]). 그것이 되돌릴 수 없는 작업이면
//!   실행취소는 까닭을 돌려주고 그 앞 기록으로 넘어가지 않는다.
//! - 실행취소한 기록은 **다시 실행** 목록으로 간다. 다시 실행은 그 목록의 마지막을 다시 하고 기록으로 되돌린다.
//!   새 작업이 기록되면([`History::record`]) 다시 실행 목록을 비운다.
//! - 실행취소·다시 실행 자체는 작업 대기열의 작업이다([`crate::window::App::undo`]). 그것이 도는 동안 대상 기록은
//!   기록에서 빠져 있고, 다른 실행취소·다시 실행은 시작하지 않는다([`UndoError::Busy`]). 끝나면 되돌린 항목은
//!   반대편 목록으로, 밖의 변경·오류로 되돌리지 못한 항목은 원래 목록의 끝으로 돌아간다(밖의 변경을 고친 뒤 다시
//!   해 볼 수 있게).
//!
//! 화면 문장은 없다(ADR-0007): 작업 이름과 까닭은 종류로만 준다.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use facet_win::recycle::RecycledItem;

use crate::edit::{EditKind, Edited};
use crate::job::JobId;

/// 작업 기록에 남는 기록 수의 상한. 넘치면 가장 오래된 기록이 빠진다.
pub const HISTORY_LIMIT: usize = 100;

/// 기록의 번호. 작업 기록 안에서 바뀌지 않고 다시 쓰이지 않는다(실행취소·다시 실행으로 옮겨 다녀도 같다).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RecordId(u64);

impl RecordId {
    /// 번호 값(디버그용).
    pub fn get(self) -> u64 {
        self.0
    }
}

/// 기록한 작업의 종류. 화면은 이것으로 「〈작업 이름〉 실행취소」의 이름을 정한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperationKind {
    /// 그 자리 이름바꾸기.
    Rename,
    /// 새 폴더. 만든 뒤 이름 입력란에서 이름을 바꿨으면 그 이름바꾸기까지 이 작업 하나다.
    NewFolder,
    /// 복사(반대편 패널로, 복사 위치…). 항목은 [`Step::Copied`]다.
    Copy,
    /// 이동(반대편 패널로, 이동 위치…). 항목은 [`Step::Moved`]·[`Step::MovedFolder`]다.
    Move,
    /// 삭제. 영구 삭제면 되돌릴 수 없는 기록([`NotUndoable::PermanentDelete`])이고 항목은 [`Step::Deleted`]다.
    /// 휴지통 삭제가 휴지통에 보낸 것 없이 모든 항목을 영구 삭제로 진행했으면 이것이다.
    Delete,
    /// **휴지통 삭제**. 항목은 휴지통에 보낸 [`Step::Recycled`]이고, 휴지통에 들어가지 않아 영구 삭제로 진행한
    /// 항목은 [`Step::Deleted`]로 함께 든다(그 항목만 되돌릴 수 없고 기록 전체는 되돌릴 수 있다).
    Recycle,
    /// **일괄 이름 바꾸기**([`crate::job::JobQueue::batch_rename`]). 항목은 바꾼 항목마다 [`Step::Renamed`](넘긴
    /// 차례)이고 규칙이 바꾸지 않아 건너뛴 항목 수가 [`Record::items_skipped`]다. 실행취소·다시 실행은 **전부 아니면
    /// 전무**다: 한 항목이라도 밖에서 바뀌었으면 아무것도 바꾸지 않고 기록이 통째로 제자리에 남는다.
    BatchRename,
}

/// 되돌릴 수 없는 까닭의 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotUndoable {
    /// 영구 삭제(휴지통을 거치지 않음, 덮어쓰기 포함): 되살릴 것이 남지 않는다.
    PermanentDelete,
    /// 복사·이동이 이름 충돌에서 대상 파일을 휴지통을 거치지 않고 덮어썼다(휴지통이 없는 위치, 휴지통에 들어가지
    /// 않는 파일): 덮어쓴 파일은 되살릴 수 없으므로 그 작업 전체를 되돌릴 수 없다고 둔다(되돌릴 수 있다고 표시한
    /// 작업은 반드시 되돌아가야 하므로). 휴지통으로 보내고 덮어쓴 것은 [`Step::Overwrote`]로 되돌릴 수 있다.
    Overwritten,
}

/// 기록할 때 본 항목의 모습: 밖에서 바뀌었는지 판단하는 데 쓴다(내용 해시는 쓰지 않는다).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    /// 폴더(폴더 링크·정션 포함)인가.
    pub is_folder: bool,
    /// 크기(바이트).
    pub size: u64,
    /// 수정 시각.
    pub modified: SystemTime,
}

impl Stamp {
    /// 지금 `path`에 있는 항목의 모습. 없거나 읽지 못하면 `None`. 링크는 따라가지 않는다(종류만 가리키는 쪽으로).
    pub fn read(path: &Path) -> Option<Stamp> {
        let metadata = std::fs::symlink_metadata(path).ok()?;
        let is_folder = metadata.is_dir()
            || (metadata.file_type().is_symlink()
                && std::fs::metadata(path).is_ok_and(|m| m.is_dir()));
        Some(Stamp {
            is_folder,
            size: metadata.len(),
            modified: metadata.modified().ok()?,
        })
    }

    /// `now`가 기록한 이 모습과 같은 항목으로 보이는가. 종류가 같아야 하고, 파일은 크기와 수정 시각도 같아야 한다.
    /// 폴더는 안의 항목이 바뀌면 수정 시각이 바뀌지만 이름을 되바꾸거나 빈 폴더를 지우는 데는 잃는 것이 없으므로
    /// 종류만 본다.
    pub fn matches(&self, now: &Stamp) -> bool {
        self.is_folder == now.is_folder
            && (self.is_folder || (self.size == now.size && self.modified == now.modified))
    }
}

/// 기록의 항목 하나: 끝난 작업이 한 일과 되돌리는 데 필요한 것.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// `from`의 이름을 `to`로 바꿨다. `stamp`는 `to`에 있는 항목의 모습이다. 되돌리기는 되바꾸기다.
    Renamed {
        from: PathBuf,
        to: PathBuf,
        stamp: Stamp,
    },
    /// `path`에 폴더를 만들었다. 되돌리기는 비어 있으면 지우기다.
    CreatedFolder { path: PathBuf, stamp: Stamp },
    /// 복사가 `source`를 복사해 `path`에 만든 항목(파일, 링크 자체, 만든 폴더). 폴더 안의 항목도 하나씩 따로 든다
    /// (폴더가 먼저, 그 안의 항목이 뒤). 대상에 원래 있어 합친 폴더는 만든 것이 아니라 들지 않는다. 「- 복사본」·
    /// 「이름 (2)」로 만들었으면 `path`가 그 이름이다. `stamp`는 `path`에 있는 항목의 모습이다.
    ///
    /// 되돌리기는 휴지통으로 보내기다(영구 삭제하지 않는다). 다시 하기는 같은 원본을 다시 복사하기다.
    Copied {
        source: PathBuf,
        path: PathBuf,
        stamp: Stamp,
    },
    /// 이동이 `from`의 항목을 통째로 `to`로 옮겼다(같은 볼륨의 이름 바꾸기, 다른 볼륨이면 복사 뒤 원본 삭제한 파일·
    /// 링크). `stamp`는 `to`에 있는 항목의 모습이다. 되돌리기는 되옮기기다.
    Moved {
        from: PathBuf,
        to: PathBuf,
        stamp: Stamp,
    },
    /// 이동이 폴더 `from`을 통째로 옮기지 않고 안의 항목을 하나씩 옮긴 폴더(대상에 같은 이름 폴더가 있어 합쳤거나
    /// 다른 볼륨이라 `to`를 만들어 옮겼다). 안의 항목은 이 뒤에 따로 든다. `created`면 이동이 `to`를 만들었다.
    ///
    /// 되돌리기는 `from` 폴더를 (없으면) 다시 만들고, 이동이 만든 `to`가 비었으면 지우기다.
    MovedFolder {
        from: PathBuf,
        to: PathBuf,
        created: bool,
        stamp: Stamp,
    },
    /// 복사·이동이 이름 충돌에서 `path`의 파일(파일 링크)을 덮어썼다: 덮어쓰기 전에 원래 파일을 휴지통으로 보냈고
    /// (`replaced`, 그때의 모습 `replaced_stamp`) 그 자리에 `source`의 파일을 놓았다(`moved`면 옮겼다, 아니면
    /// 복사했다). `stamp`는 놓은 새 파일의 모습이다.
    ///
    /// 되돌리기는 새 파일을 치우고(복사면 휴지통으로, 이동이면 `source`로 되옮김) 옛 파일을 휴지통에서 복원하기다.
    /// 다시 하기는 다시 덮어쓰기다(옛 파일을 다시 휴지통으로 보내고 같은 원본을 다시 놓음). 휴지통이 없는 위치나
    /// 휴지통에 들어가지 않는 파일을 영구 덮어쓴 것은 이것이 아니라 [`NotUndoable::Overwritten`]으로 기록한다.
    ///
    /// 실행취소한 뒤의 이 항목(다시 실행 목록)에서 `replaced`는 이미 복원한 휴지통 항목이라 쓰지 않는다.
    Overwrote {
        source: PathBuf,
        path: PathBuf,
        moved: bool,
        stamp: Stamp,
        replaced: RecycledItem,
        replaced_stamp: Stamp,
    },
    /// 영구 삭제가 넘긴 원본 `path`(나 그 안의 항목)를 지웠다. 폴더 안의 항목은 따로 들지 않는다: 되살릴 것이
    /// 남지 않아 항목별 모습이 필요 없다(지운 항목 수는 [`Record::items_completed`]). 건너뛴 항목이 있어 폴더가
    /// 남았어도 그 안에서 지운 것이 있으면 든다. 되돌릴 수 없는 기록에만 있다.
    Deleted { path: PathBuf },
    /// 휴지통 삭제가 `path`의 항목(폴더면 그 아래 전부)을 휴지통으로 보냈다(휴지통 항목 `item`). `stamp`는 보내기
    /// 전의 모습이다.
    ///
    /// 되돌리기는 휴지통에서 `path`로 복원하기다(원래 자리에 같은 이름이 있으면 [`RevertSkip::NameTaken`], 휴지통
    /// 항목이 없으면 [`RevertSkip::RecycledMissing`]으로 건너뛴다). 다시 하기는 다시 휴지통으로 보내기다(영구 삭제는
    /// 하지 않는다). 실행취소한 뒤의 이 항목(다시 실행 목록)에서 `item`은 이미 복원한 휴지통 항목이라 쓰지 않고,
    /// `stamp`는 복원한 뒤의 모습이다.
    Recycled {
        path: PathBuf,
        item: RecycledItem,
        stamp: Stamp,
    },
}

impl Step {
    /// 작업 뒤에 그 항목이 있는 경로(실행취소가 다루는 경로). 지운 항목은 지우기 전에 있던 경로다.
    pub fn path(&self) -> &Path {
        match self {
            Step::Renamed { to, .. } => to,
            Step::CreatedFolder { path, .. }
            | Step::Copied { path, .. }
            | Step::Overwrote { path, .. }
            | Step::Deleted { path }
            | Step::Recycled { path, .. } => path,
            Step::Moved { to, .. } | Step::MovedFolder { to, .. } => to,
        }
    }

    /// 작업 전에 그 항목이 있던 경로. 새로 만든 항목(새 폴더, 복사본)은 없다.
    pub fn original(&self) -> Option<&Path> {
        match self {
            Step::Renamed { from, .. }
            | Step::Moved { from, .. }
            | Step::MovedFolder { from, .. } => Some(from),
            Step::Recycled { path, .. } => Some(path),
            Step::Overwrote {
                source,
                moved: true,
                ..
            } => Some(source),
            Step::CreatedFolder { .. }
            | Step::Copied { .. }
            | Step::Overwrote { .. }
            | Step::Deleted { .. } => None,
        }
    }

    /// 복사한 항목의 원본 경로. 복사가 아니면 `None`.
    pub fn source(&self) -> Option<&Path> {
        match self {
            Step::Copied { source, .. }
            | Step::Overwrote {
                source,
                moved: false,
                ..
            } => Some(source),
            _ => None,
        }
    }
}

/// 작업 기록의 기록 하나: 끝난 작업 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    id: RecordId,
    kind: OperationKind,
    steps: Vec<Step>,
    not_undoable: Option<NotUndoable>,
    skipped: u64,
    /// 끝낸 항목 수가 항목별 결과의 수와 다르면 그 수(영구 삭제: 폴더 안의 항목을 따로 담지 않는다).
    completed: Option<u64>,
}

impl Record {
    /// 되돌릴 수 있는 기록. 번호는 [`History::record`]가 붙인다.
    pub fn new(kind: OperationKind, steps: Vec<Step>) -> Record {
        Record {
            id: RecordId(0),
            kind,
            steps,
            not_undoable: None,
            skipped: 0,
            completed: None,
        }
    }

    /// 작업이 건너뛴 항목 수를 더한 기록(작업 기록 보기의 결과에 보인다). 건너뛴 항목은 한 일이 없어 항목별 결과
    /// ([`Step`])에 들지 않는다. 이름바꾸기·새 폴더는 늘 0이다.
    pub fn with_skipped(mut self, skipped: u64) -> Record {
        self.skipped = skipped;
        self
    }

    /// 끝낸 항목 수를 따로 정한 기록: 항목별 결과([`Step`])에 폴더 안의 항목을 담지 않는 작업(영구 삭제)이 지운
    /// 항목 수를 [`Record::items_completed`]로 보이게 한다.
    pub fn with_completed(mut self, completed: u64) -> Record {
        self.completed = Some(completed);
        self
    }

    /// 되돌릴 수 없는 기록(영구 삭제 등). 실행취소는 `reason`을 돌려주고 이 앞 기록으로 넘어가지 않는다.
    pub fn not_undoable(kind: OperationKind, steps: Vec<Step>, reason: NotUndoable) -> Record {
        Record {
            not_undoable: Some(reason),
            ..Record::new(kind, steps)
        }
    }

    /// 끝난 일괄 이름 바꾸기의 기록: 바꾼 항목마다 [`Step::Renamed`](`steps`, 넘긴 차례), 규칙이 바꾸지 않아 건너뛴
    /// 항목 수 `unchanged`. 되돌릴 수 있다.
    pub fn batch_rename(steps: Vec<Step>, unchanged: u64) -> Record {
        Record::new(OperationKind::BatchRename, steps).with_skipped(unchanged)
    }

    /// 끝난 이름바꾸기·새 폴더의 기록.
    pub fn from_edit(edited: &Edited) -> Record {
        let stamp = Stamp {
            is_folder: edited.is_folder,
            size: edited.size,
            modified: edited.modified,
        };
        match (edited.kind, &edited.original) {
            (EditKind::Rename, Some(from)) => Record::new(
                OperationKind::Rename,
                vec![Step::Renamed {
                    from: from.clone(),
                    to: edited.path.clone(),
                    stamp,
                }],
            ),
            _ => Record::new(
                OperationKind::NewFolder,
                vec![Step::CreatedFolder {
                    path: edited.path.clone(),
                    stamp,
                }],
            ),
        }
    }

    /// 번호.
    pub fn id(&self) -> RecordId {
        self.id
    }

    /// 작업 종류.
    pub fn kind(&self) -> OperationKind {
        self.kind
    }

    /// 항목별 결과(작업이 한 차례대로).
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// 되돌릴 수 없으면 그 까닭.
    pub fn not_undoable_reason(&self) -> Option<NotUndoable> {
        self.not_undoable
    }

    /// 끝낸 항목 수: 항목별 결과의 수. 영구 삭제는 지운 항목 수다([`Record::with_completed`]).
    pub fn items_completed(&self) -> u64 {
        self.completed.unwrap_or(self.steps.len() as u64)
    }

    /// 맨 위 항목들: 다른 항목 아래에 있지 않은 항목(작업 이름에 쓸 것). 복사·이동은 폴더 안의 항목도 따로 들므로
    /// 폴더 하나를 복사했으면 그 폴더 하나다. 이름바꾸기·새 폴더는 모든 항목이다.
    pub fn top_steps(&self) -> Vec<&Step> {
        let paths: std::collections::HashSet<String> = self
            .steps
            .iter()
            .map(|step| path_key(step.path()))
            .collect();
        self.steps
            .iter()
            .filter(|step| {
                !step
                    .path()
                    .ancestors()
                    .skip(1)
                    .any(|parent| paths.contains(&path_key(parent)))
            })
            .collect()
    }

    /// 건너뛴 항목 수([`Record::with_skipped`]).
    pub fn items_skipped(&self) -> u64 {
        self.skipped
    }

    /// 되돌릴 수 있는가.
    pub fn is_undoable(&self) -> bool {
        self.not_undoable.is_none()
    }
}

/// 실행취소인가 다시 실행인가.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reversal {
    /// 실행취소: 기록의 마지막 작업을 되돌린다.
    Undo,
    /// 다시 실행: 실행취소한 작업을 다시 한다.
    Redo,
}

/// 실행취소·다시 실행을 시작하지 않은 까닭.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UndoError {
    /// 대상 기록이 없다.
    Nothing,
    /// 다른 실행취소·다시 실행이 아직 돈다.
    Busy,
    /// 마지막 기록이 되돌릴 수 없는 작업이다. 그 앞 기록으로 넘어가지 않는다.
    NotUndoable(NotUndoable),
}

/// 밖의 변경으로 항목 하나를 되돌리지(다시 하지) 않은 까닭. 아무것도 덮어쓰거나 지우지 않고 건너뛴다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RevertSkip {
    /// 다룰 항목이 없다(그 뒤 지워졌거나 옮겨졌다).
    Missing,
    /// 다룰 항목이 기록한 모습(종류·크기·수정 시각)과 다르다(그 뒤 바뀌었다).
    Changed,
    /// 되바꿀(만들) 자리에 같은 이름의 다른 항목이 있다.
    NameTaken,
    /// 지울 폴더가 비어 있지 않다(그 뒤 안에 무언가 생겼거나, 안의 항목을 밖의 변경으로 되돌리지 못했다).
    NotEmpty,
    /// 휴지통에 넣을 수 없어(휴지통보다 큼, 휴지통이 없는·끈 볼륨) 지우지 않았다. 실행취소는 영구 삭제하지 않는다.
    NotRecyclable,
    /// 휴지통 항목이 없다(휴지통을 비웠거나 휴지통에서 지웠다): 덮어쓴 옛 파일이면 되살릴 것이 없어 새 파일도
    /// 그대로 두고, 휴지통 삭제한 항목이면 되살릴 것이 없다.
    RecycledMissing,
    /// 휴지통 삭제가 휴지통에 들어가지 않아 영구 삭제로 진행한 항목이다: 되살릴 것이 남지 않았다. 같은 기록의 휴지통에
    /// 보낸 항목은 되돌아간다.
    PermanentlyDeleted,
}

/// 밖의 변경으로 건너뛴 항목: 그 경로와 까닭.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotReverted {
    /// 다루려던 항목의 경로.
    pub path: PathBuf,
    /// 까닭.
    pub reason: RevertSkip,
}

/// 경로를 같은지 견주는 열쇠(Windows는 대소문자를 가리지 않는다).
pub(crate) fn path_key(path: &Path) -> String {
    let text = path.to_string_lossy();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text.into_owned()
    }
}

/// 항목마다 되돌린(다시 한) 결과를 작업 스레드가 적는 곳. 되돌렸으면 반대 방향에 쓸 새 모습의 항목이 들어간다.
pub(crate) type Outcomes = Arc<Mutex<Vec<Option<Step>>>>;

/// 도는 실행취소·다시 실행.
#[derive(Debug)]
struct Running {
    job: JobId,
    reversal: Reversal,
    record: Record,
    /// 시작할 때의 세대. 그 사이 새 작업이 기록됐으면 다르다.
    generation: u64,
    outcomes: Outcomes,
}

/// 작업 기록과 다시 실행 목록. 앱 전체에 하나다.
#[derive(Debug, Default)]
pub struct History {
    /// 끝난 작업의 기록, 오래된 것부터.
    done: VecDeque<Record>,
    /// 실행취소한 기록, 다음에 다시 실행할 것이 끝.
    undone: Vec<Record>,
    running: Option<Running>,
    next_id: u64,
    /// 새 작업을 기록할 때마다 는다.
    generation: u64,
}

impl History {
    /// 빈 작업 기록.
    pub fn new() -> Self {
        History::default()
    }

    /// 끝난 작업을 기록한다. 다시 실행 목록을 비우고, [`HISTORY_LIMIT`]개를 넘으면 가장 오래된 기록을 뺀다. 붙인
    /// 번호를 돌려준다.
    pub fn record(&mut self, mut record: Record) -> RecordId {
        self.next_id += 1;
        record.id = RecordId(self.next_id);
        let id = record.id;
        self.generation += 1;
        self.undone.clear();
        self.push_done(record);
        id
    }

    fn push_done(&mut self, record: Record) {
        self.done.push_back(record);
        while self.done.len() > HISTORY_LIMIT {
            self.done.pop_front();
        }
    }

    /// 새 폴더 직후의 이름 입력란 확정(이름바꾸기)을 그 새 폴더 기록에 묶는다: 마지막 기록이 `from`을 만든 새
    /// 폴더면 그 기록의 폴더가 `to`가 되어 두 작업이 하나로 남는다. 묶지 못했으면(그 사이 다른 작업이 기록됐거나
    /// 새 폴더를 실행취소함) `false`이고 부르는 쪽이 따로 기록한다.
    pub fn merge_new_folder_rename(&mut self, from: &Path, to: &Path, stamp: Stamp) -> bool {
        let Some(record) = self.done.back_mut() else {
            return false;
        };
        if record.kind != OperationKind::NewFolder || record.steps.len() != 1 {
            return false;
        }
        match &mut record.steps[0] {
            Step::CreatedFolder { path, stamp: old } if path == from => {
                *path = to.to_path_buf();
                *old = stamp;
            }
            _ => return false,
        }
        self.generation += 1;
        self.undone.clear();
        true
    }

    /// 끝난 작업의 기록, 오래된 것부터(화면의 읽기 전용 목록용). 도는 실행취소·다시 실행의 대상은 빠져 있다.
    pub fn records(&self) -> impl DoubleEndedIterator<Item = &Record> + ExactSizeIterator + '_ {
        self.done.iter()
    }

    /// 다시 실행 목록, 실행취소한 차례대로(다음에 다시 실행할 것이 끝).
    pub fn redo_records(
        &self,
    ) -> impl DoubleEndedIterator<Item = &Record> + ExactSizeIterator + '_ {
        self.undone.iter()
    }

    /// 실행취소 대상: 끝난 작업 가운데 마지막 기록. 되돌릴 수 없는 기록이어도 그것이다(실행취소가 까닭을 돌려준다).
    /// 기록이 없거나 실행취소·다시 실행이 도는 중이면 `None`.
    pub fn undo_target(&self) -> Option<&Record> {
        if self.running.is_some() {
            return None;
        }
        self.done.back()
    }

    /// 다시 실행 대상: 다시 실행 목록의 마지막. 없거나 실행취소·다시 실행이 도는 중이면 `None`.
    pub fn redo_target(&self) -> Option<&Record> {
        if self.running.is_some() {
            return None;
        }
        self.undone.last()
    }

    /// 실행취소·다시 실행이 도는 중인가.
    pub fn is_busy(&self) -> bool {
        self.running.is_some()
    }

    /// 실행취소·다시 실행이 도는 작업.
    pub fn running_job(&self) -> Option<JobId> {
        self.running.as_ref().map(|running| running.job)
    }

    /// 기록과 다시 실행 목록이 모두 비었는가(도는 것도 없음).
    pub fn is_empty(&self) -> bool {
        self.done.is_empty() && self.undone.is_empty() && self.running.is_none()
    }

    /// 대상을 확인한다. 시작할 수 있으면 대상 기록.
    pub(crate) fn check(&self, reversal: Reversal) -> Result<&Record, UndoError> {
        if self.running.is_some() {
            return Err(UndoError::Busy);
        }
        let target = match reversal {
            Reversal::Undo => self.done.back(),
            Reversal::Redo => self.undone.last(),
        };
        let record = target.ok_or(UndoError::Nothing)?;
        match (reversal, record.not_undoable) {
            (Reversal::Undo, Some(reason)) => Err(UndoError::NotUndoable(reason)),
            _ => Ok(record),
        }
    }

    /// 대상 기록을 목록에서 꺼내 작업 `job`이 도는 것으로 둔다. `outcomes`는 작업 스레드가 결과를 적는 곳이다.
    pub(crate) fn begin(&mut self, reversal: Reversal, job: JobId, outcomes: Outcomes) {
        let record = match reversal {
            Reversal::Undo => self.done.pop_back(),
            Reversal::Redo => self.undone.pop(),
        };
        if let Some(record) = record {
            self.running = Some(Running {
                job,
                reversal,
                record,
                generation: self.generation,
                outcomes,
            });
        }
    }

    /// 작업이 `record`를 다룰 차례대로의 항목들: 실행취소는 작업의 거꾸로, 다시 실행은 그대로.
    pub(crate) fn steps_for(record: &Record, reversal: Reversal) -> Vec<Step> {
        let mut steps = record.steps.clone();
        if reversal == Reversal::Undo {
            steps.reverse();
        }
        steps
    }

    /// 작업 `job`이 끝났다. 도는 실행취소·다시 실행이면 결과대로 기록을 옮기고 `true`.
    ///
    /// 되돌린 항목은 반대편 목록으로(실행취소면 다시 실행 목록, 다시 실행이면 기록), 되돌리지 못한 항목은 원래
    /// 목록의 끝으로 돌아간다. 실행취소하는 사이 새 작업이 기록됐으면 다시 실행 목록은 비운 채로 둔다.
    #[cfg(test)]
    pub(crate) fn finish(&mut self, job: JobId) -> bool {
        self.finish_reverted(job).is_some()
    }

    /// [`History::finish`]와 같고, 도는 실행취소·다시 실행이었으면 실제로 되돌린 항목이 파일 시스템에 준 변화
    /// ([`crate::search_results::PathChange`], 검색 결과 목록에 반영할 것)를 돌려준다. 아니면 `None`이다.
    pub(crate) fn finish_reverted(
        &mut self,
        job: JobId,
    ) -> Option<Vec<crate::search_results::PathChange>> {
        if self
            .running
            .as_ref()
            .is_none_or(|running| running.job != job)
        {
            return None;
        }
        let running = self.running.take()?;
        let mut outcomes = running
            .outcomes
            .lock()
            .map(|mut o| std::mem::take(&mut *o))
            .unwrap_or_default();
        // 작업 스레드는 시작할 때의 차례(steps_for: 실행취소면 거꾸로)로 적었다. 기록의 차례로 맞춘다. 항목이 많아도
        // (복사한 폴더 안의 1만 항목) 항목을 복제하지 않고 옮기기만 한다.
        if running.reversal == Reversal::Undo {
            outcomes.reverse();
        }
        let ordered: Vec<(Step, Option<Step>)> = running
            .record
            .steps
            .into_iter()
            .zip(outcomes.into_iter().chain(std::iter::repeat(None)))
            .collect();
        let changes = ordered
            .iter()
            .filter(|(_, outcome)| outcome.is_some())
            .filter_map(|(step, _)| {
                crate::search_results::PathChange::of_reverted(step, running.reversal)
            })
            .collect();
        let mut moved = Vec::new();
        let mut stayed = Vec::new();
        for (step, outcome) in ordered {
            match outcome {
                Some(changed) => moved.push(changed),
                None => stayed.push(step),
            }
        }
        let piece = |steps: Vec<Step>| Record {
            id: running.record.id,
            kind: running.record.kind,
            steps,
            not_undoable: running.record.not_undoable,
            skipped: running.record.skipped,
            // 되돌릴 수 있는 기록만 여기 오고, 그것들은 끝낸 항목 수를 따로 갖지 않는다.
            completed: None,
        };
        let intact = running.generation == self.generation;
        match running.reversal {
            Reversal::Undo => {
                if !moved.is_empty() && intact {
                    self.undone.push(piece(moved));
                }
                if !stayed.is_empty() {
                    self.push_done(piece(stayed));
                }
            }
            Reversal::Redo => {
                if !stayed.is_empty() && intact {
                    self.undone.push(piece(stayed));
                }
                if !moved.is_empty() {
                    self.push_done(piece(moved));
                }
            }
        }
        Some(changes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rename(n: usize) -> Record {
        Record::new(
            OperationKind::Rename,
            vec![Step::Renamed {
                from: PathBuf::from(format!("a{n}")),
                to: PathBuf::from(format!("b{n}")),
                stamp: Stamp {
                    is_folder: false,
                    size: 0,
                    modified: SystemTime::UNIX_EPOCH,
                },
            }],
        )
    }

    #[test]
    fn 실행취소는_마지막_기록이고_되돌릴_수_없으면_까닭이다() {
        let mut history = History::new();
        assert_eq!(history.check(Reversal::Undo), Err(UndoError::Nothing));
        history.record(rename(1));
        assert!(history.check(Reversal::Undo).is_ok());
        history.record(Record::not_undoable(
            OperationKind::Rename,
            Vec::new(),
            NotUndoable::PermanentDelete,
        ));
        assert_eq!(
            history.check(Reversal::Undo),
            Err(UndoError::NotUndoable(NotUndoable::PermanentDelete))
        );
        assert_eq!(history.records().len(), 2);
    }

    #[test]
    fn 항목_1만_개의_복사_기록도_곧바로_다룬다() {
        let stamp = Stamp {
            is_folder: false,
            size: 4096,
            modified: SystemTime::UNIX_EPOCH,
        };
        let root = PathBuf::from("대상").join("사진");
        let mut steps = vec![Step::Copied {
            source: PathBuf::from("원본").join("사진"),
            path: root.clone(),
            stamp: Stamp {
                is_folder: true,
                ..stamp
            },
        }];
        steps.extend((0..10_000).map(|n| Step::Copied {
            source: PathBuf::from("원본").join("사진").join(format!("{n}.jpg")),
            path: root.join(format!("{n}.jpg")),
            stamp,
        }));
        let start = std::time::Instant::now();
        let mut history = History::new();
        history.record(Record::new(OperationKind::Copy, steps));
        let record = history.check(Reversal::Undo).unwrap();
        assert_eq!(record.top_steps().len(), 1, "폴더 하나를 복사한 작업이다");
        let outcomes = Outcomes::default();
        let steps = History::steps_for(record, Reversal::Undo);
        *outcomes.lock().unwrap() = steps.into_iter().map(Some).collect();
        history.begin(Reversal::Undo, JobId::for_test(1), Arc::clone(&outcomes));
        assert!(history.finish(JobId::for_test(1)));
        assert_eq!(
            history.redo_target().map(Record::items_completed),
            Some(10_001)
        );
        // 넉넉한 상한: 경로를 한 번씩 훑는 일이라 디버그 빌드에서도 수십 밀리초다.
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn 기록은_끝낸_항목과_건너뛴_항목_수를_갖는다() {
        let record = rename(1);
        assert_eq!((record.items_completed(), record.items_skipped()), (1, 0));
        let record = rename(1).with_skipped(3);
        assert_eq!((record.items_completed(), record.items_skipped()), (1, 3));
    }
}
