//! 작업 대기열과 그 안에서 도는 작업(ADR-0005).
//!
//! **작업 대기열**([`JobQueue`])은 앱 전체에 하나다. 앱이 하나만 만들어 들고 있는다. 작업을 넣으면 일꾼 스레드
//! 하나가 넣은 차례대로 하나씩 실행하고, 그 사이 일어난 일을 **사건**([`JobEvent`])으로 보낸다. 화면은 사건을
//! 받아 그리기만 한다. 사건에는 화면 문장이 없다(ADR-0007): 상태는 [`JobState`], 실패는 종류
//! ([`FileOpErrorKind`])와 경로다.
//!
//! 사건은 탭의 읽기 결과와 나란한 통로로 받는다: [`JobQueue::set_waker`]가 사건을 보낼 때마다 부를 깨우기를
//! 정하고, 깨워진 앱이 [`JobQueue::poll`]로 쌓인 사건을 받는다(테스트는 [`JobQueue::wait_event`]). 탭 결과의
//! 통로([`crate::window::App::poll`])에 섞지 않은 까닭: 작업은 탭·창에 속하지 않고(마지막 창을 닫아도 돈다),
//! 대기열을 `App` 없이 따로 테스트·벤치마크할 수 있어야 하기 때문이다. 깨우기 방식은 같으므로 앱은 두 통로를
//! 한 깨우기에서 차례로 받으면 된다.
//!
//! 작업은 기본으로 넣은 차례대로 하나씩 돈다. 사용자는 작업을 일시정지·이어하기([`JobQueue::pause`]·
//! [`JobQueue::resume`])·취소([`JobQueue::cancel`])하고, 대기 중인 작업을 「지금 시작」([`JobQueue::start_now`])해
//! 이미 도는 작업과 나란히 돌릴 수 있다. 상태가 바뀔 때마다 [`JobEvent::StateChanged`]를 보낸다. 끝난 작업에
//! 이것들을 부르면 아무 일도 없다.
//!
//! 작업이 사용자의 답을 기다려야 하면(**이름 충돌**, [`JobEvent::Conflict`]) 그 작업만 [`JobState::AwaitingAnswer`]로
//! 멈추고 질문 사건을 보낸다. 답을 기다리는 작업은 **차례를 놓는다**: 차례 실행의 다음 작업이 곧 시작할 수 있다.
//! 답([`JobQueue::answer_conflict`])을 받은 작업은 차례를 다시 기다리지 않고 그 자리에서 이어 돈다(그동안 차례를
//! 잡은 작업과 나란히). 답을 기다리는 작업도 취소할 수 있다. **항목별 오류**([`JobEvent::ItemError`],
//! [`JobQueue::answer_error`])도 같은 규칙을 따른다.
//!
//! 지금 있는 작업은 **복사**([`JobQueue::copy`])와 **이동**([`JobQueue::move_items`]), 그리고 작업 기록(복사·이동도
//! 남는다, [`crate::window::App::start_transfer`])을 되돌리는
//! **실행취소**·**다시 실행**([`crate::window::App::undo`]·[`crate::window::App::redo`])이다. 진행·이름 충돌·오류
//! 처리의 자세한 규칙은 그 문서에 있다. 삭제 작업은 **휴지통 삭제**([`JobQueue::recycle`])와 **영구 삭제**
//! ([`JobQueue::delete_permanently`])가 있다. 휴지통이 없는 위치인지는 [`has_recycle_bin`]·[`deletes_permanently`]로
//! 미리 안다. 삭제와 실행취소·다시 실행은 대기열의 작업이지만 차례를 기다리지 않고 넣자마자 시작한다(「지금 시작」과
//! 같은 길).

mod batch_rename;
mod copy;
mod delete;
mod moving;
mod recycle;
mod revert;

pub(crate) use revert::RevertJob;

use std::collections::{HashMap, VecDeque};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, SystemTime};

pub use batch_rename::RenameItem;
pub use facet_win::file_ops::FileOpErrorKind;
pub use recycle::RECYCLE_CHUNK;

/// 진행 사건 사이의 가장 짧은 간격. 작업마다 진행 사건은 초당 10번 이하다.
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// 작업의 번호. 대기열 안에서 바뀌지 않고 다시 쓰이지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JobId(u64);

impl JobId {
    /// 번호 값(기록·디버그용).
    pub fn get(self) -> u64 {
        self.0
    }

    /// 시험용 번호.
    #[cfg(test)]
    pub(crate) fn for_test(n: u64) -> Self {
        JobId(n)
    }
}

/// 작업의 종류: 복사([`JobQueue::copy`])나 이동([`JobQueue::move_items`]), 실행취소·다시 실행
/// ([`crate::window::App::undo`]·[`crate::window::App::redo`]), 삭제([`JobQueue::delete_permanently`]).
/// [`JobQueue::submit`]이 이것으로 복사·이동을 고른다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobKind {
    /// 복사.
    Copy,
    /// 이동.
    Move,
    /// 실행취소: 작업 기록의 마지막 작업을 되돌린다.
    Undo,
    /// 다시 실행: 실행취소한 작업을 다시 한다.
    Redo,
    /// **영구 삭제**([`JobQueue::delete_permanently`]). 차례를 기다리지 않고 곧바로 시작한다.
    Delete,
    /// **휴지통 삭제**([`JobQueue::recycle`]). 차례를 기다리지 않고 곧바로 시작한다.
    Recycle,
    /// **일괄 이름 바꾸기**([`JobQueue::batch_rename`]). 차례를 기다리지 않고 곧바로 시작한다.
    BatchRename,
}

/// 작업의 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobState {
    /// 넣었고 차례를 기다린다.
    Queued,
    /// 실행 중이다.
    Running,
    /// 사용자가 멈췄다. 시작 전에 멈춘 작업은 차례가 와도 시작하지 않는다.
    Paused,
    /// 이름 충돌이나 항목별 오류에 대한 사용자의 답을 기다린다. 차례를 잡고 있지 않다.
    AwaitingAnswer,
    /// 끝났다. 항목별 오류로 건너뛴 항목이 없다(이름 충돌로 건너뛴 항목은 있을 수 있다).
    Finished,
    /// 끝났지만 항목별 오류로 건너뛴 항목([`Summary::failures`])이나, 실행취소·다시 실행이 밖의 변경으로 건너뛴
    /// 항목([`Summary::not_reverted`])이 있다.
    Failed,
    /// 사용자가 취소했다. 이미 다 복사한 항목은 남는다.
    Cancelled,
}

impl JobState {
    /// 더 바뀌지 않는 마지막 상태인지.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            JobState::Finished | JobState::Failed | JobState::Cancelled
        )
    }
}

/// 남은 시간.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Remaining {
    /// 아직 다 세지 않았거나 속도를 모른다(「계산 중」).
    Calculating,
    /// 지금 속도로 남은 바이트를 옮기는 시간.
    Known(Duration),
}

/// 작업의 진행. 항목은 파일과 폴더(넘긴 원본 자신 포함)이고, 바이트는 파일 내용의 크기다.
///
/// 「한 것」에는 이름 충돌·오류로 건너뛴 항목도 든다. 그래서 작업이 끝나면 한 것이 센 것과 같다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    /// 처리한 항목 수.
    pub items_done: u64,
    /// 지금까지 센 항목 수. 다 세기 전에는 늘어난다.
    pub items_total: u64,
    /// 처리한 바이트(복사 중인 파일의 복사한 만큼 포함).
    pub bytes_done: u64,
    /// 지금까지 센 바이트.
    pub bytes_total: u64,
    /// 원본을 다 셌는지. 다 세기 전에는 `items_total`·`bytes_total`이 전체가 아니다.
    pub counted: bool,
    /// 작업을 시작한 뒤 평균 속도(초당 바이트).
    pub bytes_per_second: u64,
    /// 남은 시간. 다 세기 전에는 늘 [`Remaining::Calculating`].
    pub remaining: Remaining,
    /// 지금 다루는 원본 항목.
    pub current: Option<PathBuf>,
    /// 작업을 시작한 뒤 흐른 시간(이 진행을 만든 때).
    pub elapsed: Duration,
}

/// 항목 하나에서 난 오류(**항목별 오류**): 그 원본 항목과 오류 종류. 문장은 화면 몫이다(ADR-0007).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemFailure {
    /// 원본 경로.
    pub source: PathBuf,
    /// 오류 종류(잠김·접근 거부·디스크 가득 참·원본 없음·그 밖 등).
    pub kind: FileOpErrorKind,
}

/// 끝난 작업의 결과 요약.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    /// 복사한 파일 수.
    pub files_copied: u64,
    /// 만든 폴더 수.
    pub folders_copied: u64,
    /// 복사한 바이트.
    pub bytes_copied: u64,
    /// 이름 충돌에 건너뛰기(또는 더 새것만 덮어쓰기에서 원본이 새것이 아님)로 답해 건너뛴 항목 수(폴더면 그 폴더
    /// 하나로 센다).
    pub conflicts_skipped: u64,
    /// 이름 충돌에 덮어쓰기로 답해 덮어쓴 파일 수(복사로 덮어썼으면 [`Summary::files_copied`]에도, 파일 링크로
    /// 덮어썼으면 [`Summary::links_copied`]에도, 이동의 이름 바꾸기로 덮어썼으면 [`Summary::items_renamed`]에도
    /// 든다).
    pub files_overwritten: u64,
    /// 이동 작업이 이름 바꾸기 한 번으로 옮긴 항목 수(폴더면 그 폴더 하나로 센다). 복사 작업은 늘 0이다.
    pub items_renamed: u64,
    /// 대상에 같은 이름의 폴더가 있어 묻지 않고 합친 폴더 수([`Summary::folders_copied`]에는 들지 않는다).
    pub folders_merged: u64,
    /// 따라 들어가지 않고 링크 자체를 대상에 만든 심볼릭 링크(파일·폴더)·정션 수. 링크로 덮어썼으면
    /// [`Summary::files_overwritten`]에도 든다. 이동이 이름 바꾸기로 옮긴 링크는 [`Summary::items_renamed`]에 든다.
    pub links_copied: u64,
    /// 영구 삭제 작업이 지운 항목 수(파일·폴더·링크를 하나씩 센다). 휴지통 삭제는 휴지통에 들어가지 않아 영구 삭제로
    /// 진행한 항목에서 지운 수다. 다른 작업은 늘 0이다.
    pub items_deleted: u64,
    /// 휴지통 삭제가 휴지통에 들어가지 않는 항목([`JobEvent::NotRecyclable`])에 건너뛰기로 답해 지우지 않고 남긴
    /// 항목. [`Summary::items_skipped`]에 든다. 다른 작업은 늘 비어 있다.
    pub not_recycled: Vec<PathBuf>,
    /// 항목별 오류에 건너뛰기(또는 「나머지 오류도 건너뛰기」)로 답해 건너뛴 항목과 그 오류(폴더면 그 아래는
    /// 따로 세지 않는다). 그 수는 [`Summary::errors_skipped`]. 하나라도 있으면 작업은 [`JobState::Failed`]로 끝난다.
    pub failures: Vec<ItemFailure>,
    /// 처리한 항목 수([`Progress::items_done`]와 같은 셈: 건너뛴 항목도 든다).
    pub items_done: u64,
    /// 하지 못한 항목 수: 센 항목 가운데 처리하지 못한 것. 끝까지 돈 작업은 0이고, 취소한 작업은 남은 만큼이다.
    /// 다 세기 전에 취소했으면([`Summary::all_counted`]가 `false`) 센 데까지의 수라 실제로는 더 많을 수 있다.
    /// 시작하기 전에 취소한 작업은 넘긴 원본 수다.
    pub items_remaining: u64,
    /// 원본을 다 셌는지. `true`면 `items_done + items_remaining`이 원본 전체 항목 수다.
    pub all_counted: bool,
    /// 실행취소·다시 실행이 밖의 변경(항목이 없음·바뀜, 그 자리에 같은 이름, 새 폴더가 비어 있지 않음)으로
    /// 건너뛴 항목과 그 까닭. 아무것도 덮어쓰거나 지우지 않았다. 하나라도 있으면 작업은 [`JobState::Failed`]로
    /// 끝난다. 복사·이동은 늘 비어 있다.
    pub not_reverted: Vec<crate::history::NotReverted>,
    /// 휴지통으로 보낸 항목(원래 경로·휴지통 안 경로): 복사·이동의 덮어쓰기가 덮어쓰기 전에 보낸 원래 대상 파일,
    /// 복사(덮어쓰기 포함)의 실행취소가 보낸 복사본(폴더를 통째로 보냈으면 그 폴더 하나), 덮어쓰기의 다시 실행이 다시
    /// 보낸 옛 파일, 휴지통 삭제가 보낸 항목(넘긴 원본마다 하나, 폴더면 그 폴더 하나)과 그 다시 실행이 다시 보낸
    /// 항목. 휴지통을 거치지 않은 덮어쓰기·삭제는 들지 않는다.
    pub recycled: Vec<facet_win::recycle::RecycledItem>,
    /// 일괄 이름 바꾸기가 규칙이 이름을 바꾸지 않아 건너뛴 항목 수([`Summary::items_skipped`]에 든다). 다른 작업은
    /// 늘 0이다.
    pub names_unchanged: u64,
}

impl Summary {
    /// 항목별 오류로 건너뛴 항목 수([`Summary::failures`]의 수).
    pub fn errors_skipped(&self) -> u64 {
        self.failures.len() as u64
    }

    /// 건너뛴 항목 수: 이름 충돌로 건너뛴 것, 항목별 오류로 건너뛴 것(폴더면 그 폴더 하나로 센다), 실행취소·다시
    /// 실행이 밖의 변경으로 건너뛴 것, 휴지통 삭제가 휴지통에 들어가지 않아 남긴 것, 일괄 이름 바꾸기가 규칙이 바꾸지
    /// 않아 건너뛴 것.
    pub fn items_skipped(&self) -> u64 {
        self.conflicts_skipped
            + self.errors_skipped()
            + self.not_reverted.len() as u64
            + self.not_recycled.len() as u64
            + self.names_unchanged
    }

    /// 끝낸 항목 수: 처리한 항목([`Summary::items_done`]) 가운데 건너뛰지 않은 것.
    pub fn items_completed(&self) -> u64 {
        self.items_done.saturating_sub(self.items_skipped())
    }
}

/// 대기열이 보내는 사건.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobEvent {
    /// 진행. 작업마다 [`PROGRESS_INTERVAL`]보다 자주 오지 않는다.
    Progress {
        /// 작업.
        job: JobId,
        /// 진행.
        progress: Progress,
    },
    /// 상태가 바뀌었다. 넣으면 [`JobState::Queued`], 시작하면 [`JobState::Running`], 일시정지하면
    /// [`JobState::Paused`], 이어 가면 [`JobState::Running`](시작 전에 멈췄던 작업은 [`JobState::Queued`]),
    /// 끝나면 마지막 상태다.
    StateChanged {
        /// 작업.
        job: JobId,
        /// 새 상태.
        state: JobState,
    },
    /// 이름 충돌: 작업이 답을 기다린다. [`JobState::AwaitingAnswer`]의 [`JobEvent::StateChanged`] 바로 뒤에 온다.
    /// [`JobQueue::answer_conflict`]로 답한다.
    Conflict {
        /// 작업.
        job: JobId,
        /// 부딪힌 두 항목.
        conflict: Conflict,
    },
    /// 항목별 오류: 작업이 답을 기다린다. [`JobState::AwaitingAnswer`]의 [`JobEvent::StateChanged`] 바로 뒤에 온다.
    /// [`JobQueue::answer_error`]로 답한다. 복사하다 만 파일은 이 사건을 보내기 전에 지운다.
    ItemError {
        /// 작업.
        job: JobId,
        /// 오류가 난 원본 항목과 오류 종류.
        error: ItemFailure,
    },
    /// 휴지통 삭제에서 셸이 휴지통 대신 영구 삭제하려 한 항목(휴지통보다 큼, 휴지통이 없는·끈 볼륨): 지우지 않고
    /// 멈춰 영구 삭제로 진행할지 묻는다. [`JobState::AwaitingAnswer`]의 [`JobEvent::StateChanged`] 바로 뒤에 온다.
    /// [`JobQueue::answer_not_recyclable`]로 답한다.
    NotRecyclable {
        /// 작업.
        job: JobId,
        /// 그 항목(경로·종류·크기·수정 시각).
        item: ConflictItem,
    },
    /// 작업이 끝났다. 마지막 상태의 [`JobEvent::StateChanged`] 바로 뒤에 온다. 이 작업의 마지막 사건이다.
    Finished {
        /// 작업.
        job: JobId,
        /// 결과 요약.
        summary: Summary,
    },
}

/// 답을 기다리는 작업이 사용자에게 묻는 질문: [`JobEvent::Conflict`]나 [`JobEvent::ItemError`]의 내용. 작업
/// 영역([`crate::work_area::JobEntry::question`])이 답을 받을 때까지 들고 있어 화면이 대화를 그린다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobQuestion {
    /// 이름 충돌. [`JobQueue::answer_conflict`]로 답한다.
    Conflict(Conflict),
    /// 항목별 오류. [`JobQueue::answer_error`]로 답한다.
    Error(ItemFailure),
    /// 휴지통에 들어가지 않는 항목을 영구 삭제로 진행할지. [`JobQueue::answer_not_recyclable`]로 답한다.
    NotRecyclable(ConflictItem),
}

/// 이름 충돌의 한 쪽 항목 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemKind {
    /// 파일(파일 링크 포함).
    File,
    /// 폴더(폴더 링크·정션 포함).
    Folder,
}

/// 이름 충돌의 한 쪽 항목: 화면이 두 쪽을 비교해 보일 정보.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictItem {
    /// 경로(이름은 마지막 조각).
    pub path: PathBuf,
    /// 종류.
    pub kind: ItemKind,
    /// 파일 크기(바이트). 폴더는 `None`(안을 세지 않는다).
    pub size: Option<u64>,
    /// 수정 시각. 읽지 못하면 `None`.
    pub modified: Option<SystemTime>,
}

/// **이름 충돌**: 복사할 항목과 같은 이름이 대상에 이미 있다. 폴더끼리는 묻지 않고 합치므로 질문이 오지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// 복사할 원본 항목.
    pub source: ConflictItem,
    /// 대상에 이미 있는 항목.
    pub target: ConflictItem,
    /// 덮어쓰기·더 새것만 덮어쓰기를 고를 수 있는지. 파일과 폴더가 부딪히면 `false`이고 그 답은 거부된다.
    pub can_overwrite: bool,
    /// 덮어쓰면 대상 파일을 되살릴 수 없는지(영구 삭제). 덮어쓸 수 있고 대상 위치에 휴지통이 없으면
    /// ([`facet_win::recycle::has_recycle_bin`]) `true`다. 휴지통이 있으면 덮어쓰기 전에 대상 파일을 휴지통으로 보내
    /// 실행취소로 되살릴 수 있으므로 `false`다(그래도 셸이 휴지통에 넣지 않으려 하는 큰 파일은 영구 덮어쓴다). 화면이
    /// 덮어쓰기 선택지에 그 사실을 보이는 데 쓴다(문장은 화면 몫).
    pub overwrite_is_permanent: bool,
}

/// 이름 충돌에 대한 답의 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Resolution {
    /// 대상 파일을 원본으로 덮어쓴다(대상 폴더 안 임시 이름에 다 쓴 뒤, 대상 파일을 휴지통으로 보내고 제 이름으로
    /// 놓기. 휴지통이 없는 위치·휴지통에 들어가지 않는 파일은 바꿔치기로 영구 덮어쓰기. 읽기 전용 대상도).
    Overwrite,
    /// 이 항목(폴더면 그 아래 전부)을 하지 않는다.
    Skip,
    /// 둘 다 남긴다: 원본을 「이름 (2)」(있으면 (3) …, 파일은 확장자를 뒤에)로 복사한다.
    KeepBoth,
    /// 원본의 수정 시각이 대상보다 늦을 때만 덮어쓰고, 아니면 건너뛴다.
    OverwriteIfNewer,
}

impl Resolution {
    /// 대상을 덮어쓸 수 있는 답인지(파일과 폴더가 부딪히면 고를 수 없다).
    pub fn overwrites(self) -> bool {
        matches!(self, Resolution::Overwrite | Resolution::OverwriteIfNewer)
    }
}

/// 이름 충돌에 대한 답.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConflictAnswer {
    /// 답.
    pub resolution: Resolution,
    /// 「나머지에도 적용」: 그 작업의 남은 이름 충돌에 같은 답을 쓰고 더 묻지 않는다. 다만 그 답을 쓸 수 없는
    /// 충돌(덮어쓰기 답인데 파일과 폴더가 부딪힘)은 다시 묻는다.
    pub apply_to_rest: bool,
}

impl ConflictAnswer {
    /// 이 충돌에만 쓰는 답.
    pub fn once(resolution: Resolution) -> Self {
        ConflictAnswer {
            resolution,
            apply_to_rest: false,
        }
    }

    /// 남은 충돌에도 쓰는 답(「나머지에도 적용」).
    pub fn for_rest(resolution: Resolution) -> Self {
        ConflictAnswer {
            resolution,
            apply_to_rest: true,
        }
    }
}

/// 항목별 오류에 대한 답.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorAnswer {
    /// 그 항목을 처음부터 다시 한다(복사하던 파일은 질문 전에 지웠으므로 처음부터 다시 쓴다). 디스크 가득 참도
    /// 공간을 비운 뒤 다시 시도하면 그 항목부터 이어 가고, 이미 복사한 항목은 다시 복사하지 않는다.
    Retry,
    /// 그 항목(폴더면 그 아래 전부)을 하지 않는다. [`Summary::failures`]에 담는다.
    Skip,
    /// 「나머지 오류도 건너뛰기」: 이 항목을 건너뛰고 그 작업의 남은 항목별 오류는 묻지 않고 건너뛴다.
    SkipAll,
    /// 작업을 취소한다([`JobQueue::cancel`]과 같다: 만들다 만 파일은 지우고 이미 끝낸 항목은 남긴다).
    CancelJob,
}

/// 휴지통에 들어가지 않는 항목([`JobEvent::NotRecyclable`])에 대한 답.
///
/// 「나머지에도 적용」은 답마다 따로 둔다([`ErrorAnswer::SkipAll`]과 같은 꼴): 휴지통을 끈 볼륨에서 여러 항목을
/// 지우면 모든 항목이 같은 질문을 하므로 한 번에 답할 수 있어야 한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotRecyclableAnswer {
    /// 그 항목을 영구 삭제한다(영구 삭제 작업과 같은 엔진, [`JobQueue::delete_permanently`]). 그 항목은 되돌릴 수 없다.
    DeletePermanently,
    /// 그 항목과 그 작업의 남은 「휴지통에 들어가지 않는 항목」을 묻지 않고 영구 삭제한다.
    DeleteAllPermanently,
    /// 그 항목을 지우지 않고 남긴다([`Summary::not_recycled`]).
    Skip,
    /// 그 항목과 그 작업의 남은 「휴지통에 들어가지 않는 항목」을 묻지 않고 남긴다.
    SkipAll,
    /// 작업을 취소한다([`JobQueue::cancel`]과 같다: 이미 휴지통에 보낸 항목은 그대로 휴지통에 있다).
    CancelJob,
}

/// 답을 받지 않은 까닭.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnswerError {
    /// 그 작업은 그 종류의 질문(이름 충돌, 항목별 오류, 휴지통에 들어가지 않는 항목)에 답을 기다리지 않는다(없는·끝난·취소 중인 작업, 다른
    /// 종류의 질문을 기다리는 작업 포함).
    NotAsking,
    /// 파일과 폴더가 부딪힌 충돌에 덮어쓰기 답을 했다. 작업은 계속 답을 기다린다.
    OverwriteNotAllowed,
}

/// 복사를 시작하기 전에 막은 까닭. 이때는 작업을 만들지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyRejection {
    /// 원본이 하나도 없다.
    NoSources,
    /// 대상 위치가 없다.
    TargetMissing,
    /// 대상 위치가 폴더가 아니다.
    TargetNotFolder,
    /// 폴더를 그 폴더 자신으로 복사하려 했다(값은 그 원본).
    IntoItself(PathBuf),
    /// 대상 위치가 원본 폴더 안에 있다(값은 그 원본).
    TargetInsideSource(PathBuf),
}

/// 영구 삭제·휴지통 삭제를 시작하기 전에 막은 까닭. 이때는 작업을 만들지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeleteRejection {
    /// 원본이 하나도 없다.
    NoSources,
}

/// 일꾼에게 넘기는 작업.
enum Job {
    Copy(copy::CopyJob),
    Move(moving::MoveJob),
    Revert(revert::RevertJob),
    Delete(delete::DeleteJob),
    Recycle(recycle::RecycleJob),
    BatchRename(batch_rename::BatchRenameJob),
}

impl Job {
    /// 시작하지 않은 채 취소할 때 남은 항목 수로 알릴 값: 넘긴 원본 수(그 아래는 세지 않았다).
    fn root_count(&self) -> u64 {
        match self {
            Job::Copy(job) => job.root_count(),
            Job::Move(job) => job.root_count(),
            Job::Revert(job) => job.root_count(),
            Job::Delete(job) => job.root_count(),
            Job::Recycle(job) => job.root_count(),
            Job::BatchRename(job) => job.root_count(),
        }
    }

    /// 작업 스레드에서 끝까지(또는 취소될 때까지) 실행한다. 취소로 멈췄으면 `true`도 돌려준다.
    fn run(self, context: &Context<'_>) -> (Summary, bool) {
        match self {
            Job::Copy(job) => job.run(context),
            Job::Move(job) => job.run(context),
            Job::Revert(job) => job.run(context),
            Job::Delete(job) => job.run(context),
            Job::Recycle(job) => job.run(context),
            Job::BatchRename(job) => job.run(context),
        }
    }
}

/// 복사·이동이 실제로 한 일: 작업 기록에 남길 항목별 결과(한 차례대로)와 덮어쓴 것이 있는지. 작업 스레드가
/// 항목을 하나 끝낼 때마다 적고, 작업이 끝나면 앱이 꺼내 기록([`crate::history::Record`])으로 바꾼다.
///
/// 항목 하나가 경로 둘과 모습(크기·수정 시각) 하나라 1만 항목이면 수 MB다(경로 100자 기준 항목당 약 300바이트).
/// 실행취소가 「그 사본만」 남기려면 항목마다 그때의 모습이 있어야 해서 줄이지 않는다. 작업 기록이 100개까지라 전체도
/// 그 배수로 묶이고, 적는 비용은 이미 한 파일 연산 하나에 메타데이터 읽기 하나를 더하는 것이다.
#[derive(Debug, Default)]
pub(crate) struct Journal {
    /// 항목별 결과.
    pub(crate) steps: Vec<crate::history::Step>,
    /// 이름 충돌에서 대상을 휴지통을 거치지 않고 영구 덮어썼다(그 작업은 되돌릴 수 없다). 휴지통으로 보내고
    /// 덮어쓴 것은 항목별 결과의 [`crate::history::Step::Overwrote`]로 남고 되돌릴 수 있다.
    pub(crate) overwrote: bool,
}

/// 작업 스레드가 [`Journal`]을 적는 곳.
pub(crate) type JournalSink = Arc<Mutex<Journal>>;

/// 깨우기.
type Waker = Arc<dyn Fn() + Send + Sync>;

/// 일꾼 스레드가 사건을 보내는 곳. 보낸 뒤 깨우기가 있으면 부른다.
#[derive(Clone)]
struct Outbox {
    sender: Sender<JobEvent>,
    waker: Arc<Mutex<Option<Waker>>>,
}

impl Outbox {
    fn send(&self, event: JobEvent) {
        // 대기열이 이미 없어졌으면 사건을 받을 곳이 없다(작업은 끝까지 돈다).
        if self.sender.send(event).is_err() {
            return;
        }
        let waker = self.waker.lock().ok().and_then(|w| w.clone());
        if let Some(waker) = waker {
            waker();
        }
    }
}

/// 도는 작업 하나의 일시정지·취소 표지. 작업 스레드(복사의 진행 콜백, 항목 사이)와 훑기 스레드가 본다.
#[derive(Default)]
struct Control {
    flags: Mutex<Flags>,
    changed: Condvar,
}

#[derive(Default)]
struct Flags {
    paused: bool,
    cancelled: bool,
    /// 받은 답(작업 스레드가 가져간다).
    answer: Option<Answer>,
}

/// 작업이 묻는 질문의 종류. 답을 기다리는 동안 판에 둔다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Question {
    /// 이름 충돌. 덮어쓰기 답을 받을 수 있는지.
    Conflict { can_overwrite: bool },
    /// 항목별 오류.
    Error,
    /// 휴지통에 들어가지 않는 항목.
    NotRecyclable,
}

/// 질문에 대한 답.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    Conflict(ConflictAnswer),
    Error(ErrorAnswer),
    NotRecyclable(NotRecyclableAnswer),
}

impl Control {
    /// 일시정지면 풀리거나 취소될 때까지 기다린다. 계속해도 되면 `true`, 취소됐으면 `false`.
    fn proceed(&self) -> bool {
        let mut flags = lock(&self.flags);
        while flags.paused && !flags.cancelled {
            flags = self
                .changed
                .wait(flags)
                .unwrap_or_else(PoisonError::into_inner);
        }
        !flags.cancelled
    }

    /// 일시정지가 아니면 `f`를 부른다. 표지 잠금 안에서 부르므로, 일시정지를 세우고 보내는 일시정지 사건보다
    /// `f`가 보낸 사건이 늘 먼저다(진행 사건이 일시정지 사건 뒤에 오지 않게).
    fn unless_paused(&self, f: impl FnOnce()) {
        let flags = lock(&self.flags);
        if !flags.paused {
            f();
        }
    }

    fn set_paused(&self, paused: bool) {
        lock(&self.flags).paused = paused;
        self.changed.notify_all();
    }

    fn cancel(&self) {
        lock(&self.flags).cancelled = true;
        self.changed.notify_all();
    }

    /// 답이 오거나 취소될 때까지 기다린다. 취소됐으면 `None`.
    fn wait_answer(&self) -> Option<Answer> {
        let mut flags = lock(&self.flags);
        loop {
            if flags.cancelled {
                return None;
            }
            if let Some(answer) = flags.answer.take() {
                return Some(answer);
            }
            flags = self
                .changed
                .wait(flags)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn set_answer(&self, answer: Answer) {
        lock(&self.flags).answer = Some(answer);
        self.changed.notify_all();
    }
}

/// 도는 작업이 대기열과 주고받는 곳: 번호, 사건을 보낼 곳, 일시정지·취소 표지, 판(질문할 때 상태를 바꾼다).
struct Context<'a> {
    id: JobId,
    outbox: &'a Outbox,
    control: &'a Control,
    shared: &'a Shared,
}

impl Context<'_> {
    /// 이름 충돌을 묻고 답을 기다린다. 취소되면 `None`.
    fn ask_conflict(&self, conflict: Conflict) -> Option<ConflictAnswer> {
        let question = Question::Conflict {
            can_overwrite: conflict.can_overwrite,
        };
        let event = JobEvent::Conflict {
            job: self.id,
            conflict,
        };
        match self.ask(question, event)? {
            Answer::Conflict(answer) => Some(answer),
            Answer::Error(_) | Answer::NotRecyclable(_) => None,
        }
    }

    /// 항목별 오류를 묻고 답을 기다린다. 취소되면(작업 취소 답 포함) `None`.
    fn ask_error(&self, error: ItemFailure) -> Option<ErrorAnswer> {
        let event = JobEvent::ItemError {
            job: self.id,
            error,
        };
        match self.ask(Question::Error, event)? {
            Answer::Error(ErrorAnswer::CancelJob)
            | Answer::Conflict(_)
            | Answer::NotRecyclable(_) => None,
            Answer::Error(answer) => Some(answer),
        }
    }

    /// 휴지통에 들어가지 않는 항목을 영구 삭제로 진행할지 묻고 답을 기다린다. 취소되면(작업 취소 답 포함) `None`.
    fn ask_not_recyclable(&self, item: ConflictItem) -> Option<NotRecyclableAnswer> {
        let event = JobEvent::NotRecyclable { job: self.id, item };
        match self.ask(Question::NotRecyclable, event)? {
            Answer::NotRecyclable(NotRecyclableAnswer::CancelJob)
            | Answer::Conflict(_)
            | Answer::Error(_) => None,
            Answer::NotRecyclable(answer) => Some(answer),
        }
    }

    /// 묻고 답을 기다린다. 기다리는 동안 작업은 [`JobState::AwaitingAnswer`]이고 차례를 놓는다. 답이 오면
    /// [`JobState::Running`]으로 이어 가고(상태는 답하는 쪽이 바꾼다), 취소되면 `None`.
    ///
    /// 일시정지 중이면 이어 갈 때까지 기다린 뒤 묻는다(일시정지와 답 기다림이 겹치지 않게).
    fn ask(&self, question: Question, event: JobEvent) -> Option<Answer> {
        loop {
            let mut board = self.shared.board();
            let slot = board.jobs.get_mut(&self.id)?;
            if slot.cancelling {
                return None;
            }
            if slot.state == JobState::Paused {
                drop(board);
                if !self.control.proceed() {
                    return None;
                }
                continue;
            }
            slot.state = JobState::AwaitingAnswer;
            slot.question = Some(question);
            self.outbox.send(JobEvent::StateChanged {
                job: self.id,
                state: JobState::AwaitingAnswer,
            });
            self.outbox.send(event);
            if board.turn == Some(self.id) {
                board.turn = None;
            }
            drop(board);
            self.shared.changed.notify_all();
            return self.control.wait_answer();
        }
    }
}

/// 잠금이 독(다른 스레드의 패닉)에 걸려도 값을 쓴다. 값은 한 번에 바뀌는 단위로만 고치므로 그대로 쓸 수 있다.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 대기열의 판: 끝나지 않은 작업과 차례. 작업 상태는 이 판에서만 바꾸고, 바꾼 그 잠금 안에서 상태 사건을 보내
/// 상태 사건의 차례가 판의 차례와 같다.
#[derive(Default)]
struct Board {
    /// 아직 시작하지 않은 작업의 차례(대기·시작 전 일시정지).
    pending: VecDeque<JobId>,
    /// 끝나지 않은 작업.
    jobs: HashMap<JobId, Slot>,
    /// 차례를 잡고 도는 작업. 차례 일꾼은 이것이 비어야 다음 작업을 시작한다. 답을 기다리는 작업은 차례를 놓는다.
    turn: Option<JobId>,
    /// 대기열을 버렸다. 차례 일꾼은 더 시작할 작업이 없으면 끝난다.
    closed: bool,
}

struct Slot {
    state: JobState,
    /// 시작하기 전이면 작업, 시작했으면 `None`.
    job: Option<Job>,
    control: Arc<Control>,
    /// 취소를 청했다(작업 스레드가 멈추면 취소로 끝난다).
    cancelling: bool,
    /// 답을 기다리는 질문. 답을 받으면 비운다.
    question: Option<Question>,
}

/// 시작한 작업: 번호, 작업, 그 표지.
type Started = (JobId, Job, Arc<Control>);

impl Board {
    /// 차례에서 다음에 시작할 작업(맨 앞의 대기 상태, 일시정지는 건너뜀)을 꺼내 실행 중으로 한다.
    fn take_next(&mut self, outbox: &Outbox) -> Option<Started> {
        let index = self.pending.iter().position(|id| {
            self.jobs
                .get(id)
                .is_some_and(|slot| slot.state == JobState::Queued)
        })?;
        let id = self.pending.remove(index)?;
        self.start(id, outbox)
    }

    /// 시작하지 않은 작업을 실행 중으로 하고 꺼낸다(차례에서는 부르는 쪽이 뺀다).
    fn start(&mut self, id: JobId, outbox: &Outbox) -> Option<Started> {
        let slot = self.jobs.get_mut(&id)?;
        let job = slot.job.take()?;
        slot.state = JobState::Running;
        outbox.send(JobEvent::StateChanged {
            job: id,
            state: JobState::Running,
        });
        Some((id, job, Arc::clone(&slot.control)))
    }
}

/// 판과 그 바뀜 알림. 차례 일꾼이 시작할 작업이 생기기를 기다린다.
#[derive(Default)]
struct Shared {
    board: Mutex<Board>,
    changed: Condvar,
}

impl Shared {
    fn board(&self) -> MutexGuard<'_, Board> {
        lock(&self.board)
    }
}

/// 작업 대기열. 앱 전체에 하나다.
///
/// 작업은 넣은 차례대로 하나씩 실행한다(차례 일꾼 스레드 하나, 처음 작업을 넣을 때 띄움). 사용자가 대기 중인
/// 작업을 [`JobQueue::start_now`]하면 그 작업만 따로 스레드를 띄워 차례를 기다리지 않고 나란히 돈다.
///
/// 대기열을 버려도 이미 시작한 작업과 대기 중인 작업은 끝까지 돈다(일시정지한 작업은 이을 쪽이 없으니 남는다).
pub struct JobQueue {
    next_id: u64,
    shared: Arc<Shared>,
    runner: bool,
    outbox: Outbox,
    receiver: Receiver<JobEvent>,
}

impl Default for JobQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for JobQueue {
    fn drop(&mut self) {
        self.shared.board().closed = true;
        self.shared.changed.notify_all();
    }
}

impl JobQueue {
    /// 빈 대기열.
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        JobQueue {
            next_id: 1,
            shared: Arc::default(),
            runner: false,
            outbox: Outbox {
                sender,
                waker: Arc::default(),
            },
            receiver,
        }
    }

    /// 사건을 보낼 때마다 부를 깨우기를 정한다. 일꾼 스레드에서, 또 상태를 바꾸는 이 대기열의 메서드 안에서
    /// 대기열의 잠금을 쥔 채 불린다. 그러니 깨우기 안에서 대기열을 다시 부르지 않는다(앱은 이벤트 루프만 깨운다).
    pub fn set_waker(&mut self, waker: impl Fn() + Send + Sync + 'static) {
        if let Ok(mut slot) = self.outbox.waker.lock() {
            *slot = Some(Arc::new(waker));
        }
    }

    /// 쌓인 사건을 모두 받는다. 기다리지 않는다.
    pub fn poll(&mut self) -> Vec<JobEvent> {
        self.receiver.try_iter().collect()
    }

    /// 사건 하나를 `timeout`까지 기다려 받는다.
    pub fn wait_event(&mut self, timeout: Duration) -> Option<JobEvent> {
        self.receiver.recv_timeout(timeout).ok()
    }

    /// 끝나지 않은(기다리거나 도는) 작업이 있는지. 마지막 작업의 [`JobEvent::Finished`]를 보낸 뒤 `false`가 된다.
    pub fn has_unfinished(&self) -> bool {
        !self.shared.board().jobs.is_empty()
    }

    /// 끝나지 않은 작업의 지금 상태. 끝난(또는 없는) 작업이면 `None`.
    pub fn state(&self, job: JobId) -> Option<JobState> {
        self.shared.board().jobs.get(&job).map(|slot| slot.state)
    }

    /// 원본 항목들(파일·폴더)을 `target` 폴더 안으로 복사하는 작업을 넣는다.
    ///
    /// 막을 경우([`CopyRejection`])에는 작업을 만들지 않고 까닭을 돌려준다: 원본이 없음, 대상 위치가 없거나
    /// 폴더가 아님, 폴더를 자기 자신이나 그 하위로 복사.
    ///
    /// 작업이 시작되면 원본을 훑어 전체 항목 수와 바이트를 세는 동안 이미 센 항목부터 복사한다. 파일 내용과
    /// 수정 시각·만든 시각·속성·대체 데이터 스트림을 보존하고, 폴더 시각·속성은 안의 항목을 다 쓴 뒤 맞춘다.
    /// 심볼릭 링크(폴더·파일)·정션은 따라 들어가지 않고 링크 자체를 대상에 만든다([`Summary::links_copied`]:
    /// 같은 종류, 같은 곳을 가리킴, 상대 경로 링크는 상대 경로 그대로). 진행에서는 항목 하나(바이트 0)다. 링크를
    /// 만들 권한이 없으면(개발자 모드가 아닌 일반 사용자의 심볼릭 링크 등) 접근 거부 항목별 오류로 묻는다.
    ///
    /// 원본이 대상과 같은 폴더에 있으면 묻지 않고 「이름 - 복사본」(있으면 「이름 - 복사본 (2)」, 「(3)」 …)으로
    /// 만든다. 파일은 확장자를 이름 뒤에 그대로 둔다.
    ///
    /// **이름 충돌**(대상에 같은 이름이 이미 있음)은 사용자에게 묻는다: 작업이 [`JobState::AwaitingAnswer`]가 되고
    /// [`JobEvent::Conflict`]를 보낸 뒤 [`JobQueue::answer_conflict`]의 답([`Resolution`])대로 푼다. 「나머지에도
    /// 적용」이면 그 작업의 남은 충돌에 같은 답을 쓴다. 같은 이름의 폴더끼리는 묻지 않고 합친다(안의 충돌은 다시
    /// 묻는다, 합친 폴더의 시각·속성은 대상 것을 그대로 둔다). 파일과 폴더가 부딪히면 덮어쓰기를 고를 수 없다.
    /// 덮어쓰기는 대상 폴더 안 임시 이름에 다 쓴 뒤 바꿔치기하므로, 복사가 실패하거나 취소되면 원래 파일이 그대로
    /// 남고 임시 파일은 남지 않는다. 덮어쓴 파일은 되살릴 수 없다(영구 삭제).
    ///
    /// **항목별 오류**(잠김·접근 거부·디스크 가득 참·원본 없음 등)도 묻는다: 복사하다 만 파일을 지운 뒤 작업이
    /// [`JobState::AwaitingAnswer`]가 되고 [`JobEvent::ItemError`]를 보낸 뒤 [`JobQueue::answer_error`]의 답
    /// ([`ErrorAnswer`])대로 한다: 다시 시도(그 항목을 처음부터), 건너뛰기, 작업 취소, 「나머지 오류도 건너뛰기」.
    /// 훑은 뒤 복사 전에 원본이 사라져도 원본 없음으로 묻는다. 건너뛴 항목은 [`Summary::failures`]에 담고, 하나라도
    /// 있으면 작업은 [`JobState::Failed`]로 끝난다.
    ///
    /// 일시정지하면 파일 경계를 기다리지 않고 파일 복사의 진행 콜백(덩어리 사이)에서 멈추고 훑기도 멈춘다. 취소하면
    /// 복사하다 만 파일은 지우고, 이미 다 복사한 항목은 남긴다.
    pub fn copy(&mut self, sources: Vec<PathBuf>, target: PathBuf) -> Result<JobId, CopyRejection> {
        let job = copy::CopyJob::prepare(sources, target)?;
        Ok(self.enqueue(Job::Copy(job)))
    }

    /// 원본 항목들(파일·폴더)을 `target` 폴더 안으로 옮기는(**이동**) 작업을 넣는다. 사건·상태·일시정지·취소·이름
    /// 충돌·항목별 오류 질문은 복사([`JobQueue::copy`])와 같고, 시작 전에 막는 경우([`CopyRejection`])도 같다.
    ///
    /// 항목마다 먼저 이름 바꾸기를 한다. **같은 볼륨 안 이동은 이름 바꾸기 한 번**이라 큰 폴더도 곧바로 끝나고
    /// 파일 ID·보안 설명자가 그대로 남는다([`Summary::items_renamed`]). 다른 볼륨이라 이름을 바꿀 수 없으면 그
    /// 항목을 **복사한 뒤 원본을 지운다**: 파일은 복사를 마친 뒤 지우고, 폴더는 안의 항목이 모두 옮겨진 뒤 빈
    /// 폴더일 때만 지운다. 그래서 실패·건너뛰기·취소한 항목의 원본과 그 상위 폴더는 남는다. 복사를 마친 원본을 지우지
    /// 못하면 항목별 오류로 묻는다(건너뛰면 원본과 복사본이 둘 다 남는다).
    ///
    /// 진행은 이름 바꾸기도 항목 수로 세고, 바이트는 복사로 바뀐 항목만 센다. 항목 수는 합치거나 복사로 바꾼 폴더를
    /// 훑으며 늘어나므로 작업이 끝날 때까지 다 셌다고 하지 않는다(남은 시간은 「계산 중」).
    ///
    /// 같은 볼륨이어도 대상에 같은 이름이 있으면 **이름 충돌**로 묻는다. 같은 이름의 폴더끼리는 묻지 않고 합친다:
    /// 안의 항목을 하나씩 옮기고(안의 충돌은 다시 묻는다) 빈 원본 폴더를 지운다. 덮어쓰기는 원본을 대상 폴더 안
    /// 임시 이름으로 옮긴(다른 볼륨이면 복사한) 뒤 바꿔치기한다.
    ///
    /// **같은 폴더로 이동**(원본이 이미 대상 폴더 바로 안에 있음)은 아무것도 하지 않는다: 그 항목은 처리한 것으로만
    /// 세고, 원본이 모두 그렇다면 작업은 곧바로 [`JobState::Finished`]로 끝난다.
    pub fn move_items(
        &mut self,
        sources: Vec<PathBuf>,
        target: PathBuf,
    ) -> Result<JobId, CopyRejection> {
        let job = moving::MoveJob::prepare(sources, target)?;
        Ok(self.enqueue(Job::Move(job)))
    }

    /// 종류에 따라 복사([`JobQueue::copy`])나 이동([`JobQueue::move_items`]) 작업을 넣는다. 실행취소·다시 실행과
    /// 삭제는 원본·대상으로 넣는 작업이 아니라([`crate::window::App::undo`], [`JobQueue::delete_permanently`]) 작업을
    /// 만들지 않고 [`CopyRejection::NoSources`]다.
    pub fn submit(
        &mut self,
        kind: JobKind,
        sources: Vec<PathBuf>,
        target: PathBuf,
    ) -> Result<JobId, CopyRejection> {
        match kind {
            JobKind::Copy => self.copy(sources, target),
            JobKind::Move => self.move_items(sources, target),
            JobKind::Undo
            | JobKind::Redo
            | JobKind::Delete
            | JobKind::Recycle
            | JobKind::BatchRename => {
                Err(CopyRejection::NoSources)
            }
        }
    }

    /// 복사·이동 작업을 넣고([`JobQueue::submit`]과 같다) 그 작업이 한 일을 적을 곳을 함께 돌려준다(작업 기록에
    /// 남길 작업). 작업이 끝난 뒤([`JobEvent::Finished`]) 꺼내 쓴다.
    pub(crate) fn submit_recorded(
        &mut self,
        kind: JobKind,
        sources: Vec<PathBuf>,
        target: PathBuf,
    ) -> Result<(JobId, JournalSink), CopyRejection> {
        let journal = JournalSink::default();
        let job = match kind {
            JobKind::Copy => {
                let mut job = copy::CopyJob::prepare(sources, target)?;
                job.journal = Some(Arc::clone(&journal));
                Job::Copy(job)
            }
            JobKind::Move => {
                let mut job = moving::MoveJob::prepare(sources, target)?;
                job.plan.journal = Some(Arc::clone(&journal));
                Job::Move(job)
            }
            JobKind::Undo
            | JobKind::Redo
            | JobKind::Delete
            | JobKind::Recycle
            | JobKind::BatchRename => {
                return Err(CopyRejection::NoSources);
            }
        };
        Ok((self.enqueue(job), journal))
    }

    /// 실행취소·다시 실행 작업을 넣고 차례를 기다리지 않고 곧바로 시작한다([`JobQueue::start_now`]와 같다).
    ///
    /// 곧바로 시작하는 까닭: 실행취소는 사용자가 방금 한 일을 바로잡으려는 것이라, 앞에 큰 복사가 돌고 있다고
    /// 그 뒤로 미루면 무엇이 되돌아갔는지 알 수 없는 채 기다리게 된다. 이름바꾸기·새 폴더·같은 볼륨 이동의 되돌리기는
    /// 순간에 끝나고 복사의 되돌리기는 휴지통으로 보내기(같은 볼륨 안 이름 바꾸기)라 다른 작업의 디스크를 거의 쓰지
    /// 않는다. 볼륨을 넘는 이동의 되돌리기는 다시 복사하지만, 그것도 사용자가 곧바로 되돌리기를 바란 것이다. 사건·일시정지·취소·항목별 오류 질문은 다른 작업과 같다.
    pub(crate) fn revert(&mut self, job: RevertJob) -> JobId {
        self.start_at_once(Job::Revert(job))
    }

    /// 원본 항목들(파일·폴더)을 **영구 삭제**하는 작업을 넣고 차례를 기다리지 않고 곧바로 시작한다
    /// ([`JobQueue::start_now`]와 같은 길: 이미 도는 작업과 나란히 돌고 차례 실행은 그대로 이어진다). 휴지통을 거치지
    /// 않으므로 되돌릴 수 없다. 원본이 없으면 작업을 만들지 않는다([`DeleteRejection::NoSources`]).
    ///
    /// 곧바로 시작하는 까닭: 삭제는 사용자가 지금 눈앞의 항목을 치우려는 것이라, 앞에 큰 복사가 돌고 있다고 그 뒤로
    /// 미루면 지운 항목이 목록에 남은 채 기다리게 된다. 지우기는 디스크를 거의 쓰지 않아 다른 작업을 늦추지 않는다.
    ///
    /// 사건·상태·일시정지·취소·항목별 오류 질문은 복사([`JobQueue::copy`])와 같다. 진행은 항목 수(파일·폴더·링크,
    /// 넘긴 원본 자신 포함)와 파일 크기의 합으로 세고, 원본을 훑어 세는 동안 이미 센 항목부터 지운다.
    ///
    /// - 폴더는 안을 깊이 우선으로 지운 뒤 빈 폴더를 지운다. 읽기 전용 파일·폴더는 읽기 전용을 풀고 지운다.
    /// - 심볼릭 링크(파일·폴더)·정션은 따라 들어가지 않고 링크만 지운다. 가리키던 곳의 내용은 그대로다.
    /// - 잠긴 파일·접근 거부 등은 **항목별 오류**로 묻는다([`JobEvent::ItemError`], [`JobQueue::answer_error`]).
    ///   건너뛴 항목([`Summary::failures`])과 그 상위 폴더는 남고(상위 폴더는 오류로 다시 묻지 않는다) 작업은
    ///   [`JobState::Failed`]로 끝난다.
    /// - 없는 항목(넘긴 원본이 이미 없음, 훑은 뒤 밖에서 지워짐)은 이미 지워진 것으로 보고 묻지 않는다.
    /// - 취소하면 이미 지운 것만 지워져 있다.
    /// - 260자를 넘는 경로도 지운다.
    ///
    /// 지운 항목 수는 [`Summary::items_deleted`]다.
    pub fn delete_permanently(&mut self, sources: Vec<PathBuf>) -> Result<JobId, DeleteRejection> {
        let job = delete::DeleteJob::prepare(sources)?;
        Ok(self.start_at_once(Job::Delete(job)))
    }

    /// 영구 삭제 작업을 넣어 곧바로 시작하고([`JobQueue::delete_permanently`]와 같다) 그 작업이 한 일을 적을 곳을
    /// 함께 돌려준다(작업 기록에 남길 작업). 작업이 끝난 뒤([`JobEvent::Finished`]) 꺼내 쓴다.
    pub(crate) fn delete_permanently_recorded(
        &mut self,
        sources: Vec<PathBuf>,
    ) -> Result<(JobId, JournalSink), DeleteRejection> {
        let journal = JournalSink::default();
        let mut job = delete::DeleteJob::prepare(sources)?;
        job.journal = Some(Arc::clone(&journal));
        Ok((self.start_at_once(Job::Delete(job)), journal))
    }

    /// 원본 항목들(파일·폴더)을 **휴지통 삭제**하는(휴지통으로 보내는) 작업을 넣고 차례를 기다리지 않고 곧바로
    /// 시작한다(영구 삭제와 같은 까닭과 길). 원본이 없으면 작업을 만들지 않는다([`DeleteRejection::NoSources`]).
    ///
    /// 휴지통으로 보내기는 셸의 휴지통 보내기([`facet_win::recycle::recycle`])이고 작업 스레드에서 부른다(UI 스레드가
    /// 아니다). 넘긴 원본을 [`RECYCLE_CHUNK`]개씩 셸에 넘긴다. 사건·상태·취소·항목별 오류 질문(잠김·접근 거부 등:
    /// 다시 시도·건너뛰기·나머지 오류도 건너뛰기·작업 취소)은 다른 작업과 같다.
    ///
    /// - **일시정지·취소는 셸 호출 사이(항목 묶음 경계)에서만** 듣는다: 셸의 파일 작업은 한 번 부르면 멈출 수 없고,
    ///   휴지통으로 보내기는 같은 볼륨 안 이름 바꾸기라 묶음 하나가 금방 끝나기 때문이다. 영구 삭제로 진행한 항목을
    ///   지우는 동안은 영구 삭제 작업처럼 항목마다 듣는다.
    /// - 진행은 넘긴 원본 수로 센다(폴더는 하나, 바이트는 영구 삭제로 진행한 항목의 파일 크기만). 처음부터 다 센
    ///   것이다.
    /// - **휴지통에 들어가지 않는 항목**(셸이 영구 삭제로 바꾸려 한 항목)은 지우지 않고 멈춰
    ///   [`JobEvent::NotRecyclable`]로 묻는다([`JobQueue::answer_not_recyclable`]): 영구 삭제로 진행하면 영구 삭제
    ///   작업의 엔진으로 지우고(그 항목은 되돌릴 수 없다), 건너뛰면 남긴다([`Summary::not_recycled`]).
    /// - 없는 항목(넘긴 원본이 이미 없음)은 이미 지워진 것으로 보고 묻지 않는다.
    /// - 취소하면 이미 휴지통에 보낸 항목은 휴지통에 있다(실행취소로 되살린다).
    ///
    /// 휴지통에 보낸 항목은 [`Summary::recycled`]에 든다.
    pub fn recycle(&mut self, sources: Vec<PathBuf>) -> Result<JobId, DeleteRejection> {
        let job = recycle::RecycleJob::prepare(sources)?;
        Ok(self.start_at_once(Job::Recycle(job)))
    }

    /// 휴지통 삭제 작업을 넣어 곧바로 시작하고([`JobQueue::recycle`]와 같다) 그 작업이 한 일을 적을 곳을 함께
    /// 돌려준다(작업 기록에 남길 작업).
    pub(crate) fn recycle_recorded(
        &mut self,
        sources: Vec<PathBuf>,
    ) -> Result<(JobId, JournalSink), DeleteRejection> {
        let journal = JournalSink::default();
        let mut job = recycle::RecycleJob::prepare(sources)?;
        job.journal = Some(Arc::clone(&journal));
        Ok((self.start_at_once(Job::Recycle(job)), journal))
    }

    /// 항목들의 이름을 같은 폴더 안에서 한꺼번에 바꾸는 **일괄 이름 바꾸기** 작업을 넣고 차례를 기다리지 않고 곧바로
    /// 시작한다(삭제와 같은 까닭과 길: 이름 바꾸기는 디스크를 거의 쓰지 않는다). `items`는 바뀌는 항목만(넘긴 차례가
    /// 작업 기록의 차례), `unchanged`는 규칙이 바꾸지 않아 건너뛴 항목 수다([`Summary::names_unchanged`]).
    ///
    /// **전부 아니면 전무**다. 아무것도 바꾸기 전에 디스크에서 원본이 있는지, 새 이름 자리에 이 작업이 비우지 않을
    /// 다른 항목이 없는지(대소문자 무시, 대소문자만 바꾸는 이름은 같은 항목) 보고, 걸리면 아무것도 바꾸지 않고 그
    /// 항목들을 [`Summary::failures`](`SourceMissing`·`TargetExists`)에 담아 [`JobState::Failed`]로 끝난다. 사슬·
    /// 맞바꿈·대소문자만 바꾸기는 같은 폴더 안 임시 이름을 거친다. 일시정지·취소는 이름 바꾸기 사이에서 듣고, 항목별
    /// 오류 질문은 하지 않는다. 바꾸는 중 실패하거나 취소하면 바꾼 항목을 모두 되돌리고 [`JobState::Failed`]
    /// (실패한 항목은 [`Summary::failures`])·[`JobState::Cancelled`]로 끝난다. 진행은 항목 수(`unchanged` 포함)다.
    ///
    /// 끝나면([`JobState::Finished`]) [`Summary::items_renamed`]가 바꾼 수, [`Summary::items_done`]이 바꾼 수와
    /// `unchanged`의 합이다.
    pub fn batch_rename(&mut self, items: Vec<RenameItem>, unchanged: u64) -> JobId {
        self.start_at_once(Job::BatchRename(batch_rename::BatchRenameJob::new(
            items, unchanged,
        )))
    }

    /// 일괄 이름 바꾸기 작업을 넣어 곧바로 시작하고([`JobQueue::batch_rename`]와 같다) 그 작업이 한 일을 적을 곳을
    /// 함께 돌려준다(작업 기록에 남길 작업). 끝나면 바뀐 채 있는 항목마다 [`crate::history::Step::Renamed`]가 넘긴
    /// 차례대로 든다(모두 되돌렸으면 비어 있다).
    pub(crate) fn batch_rename_recorded(
        &mut self,
        items: Vec<RenameItem>,
        unchanged: u64,
    ) -> (JobId, JournalSink) {
        let journal = JournalSink::default();
        let mut job = batch_rename::BatchRenameJob::new(items, unchanged);
        job.journal = Some(Arc::clone(&journal));
        (self.start_at_once(Job::BatchRename(job)), journal)
    }

    /// 작업을 넣고 차례를 기다리지 않고 곧바로 시작한다(실행취소·다시 실행, 삭제, 일괄 이름 바꾸기).
    fn start_at_once(&mut self, job: Job) -> JobId {
        let id = self.enqueue(job);
        self.start_now(id);
        id
    }

    fn enqueue(&mut self, job: Job) -> JobId {
        let id = JobId(self.next_id);
        self.next_id += 1;
        {
            let mut board = self.shared.board();
            board.jobs.insert(
                id,
                Slot {
                    state: JobState::Queued,
                    job: Some(job),
                    control: Arc::default(),
                    cancelling: false,
                    question: None,
                },
            );
            board.pending.push_back(id);
            // 대기 상태를 판에 올린 잠금 안에서 보내 이 작업의 다른 사건보다 먼저 온다.
            self.outbox.send(JobEvent::StateChanged {
                job: id,
                state: JobState::Queued,
            });
        }
        if !self.runner {
            self.runner = true;
            let shared = Arc::clone(&self.shared);
            let outbox = self.outbox.clone();
            thread::Builder::new()
                .name("facet-jobs".into())
                .spawn(move || run_in_turn(&shared, &outbox))
                .expect("작업 일꾼 스레드를 띄우지 못했다");
        }
        self.shared.changed.notify_all();
        id
    }

    /// 작업을 일시정지한다. 실행 중이면 파일 경계를 기다리지 않고 멈추고(훑기도 멈춘다), 대기 중이면 차례가 와도
    /// 시작하지 않고 뒤 작업이 먼저 돈다. 상태는 [`JobState::Paused`]가 되고 그 사건을 보낸다.
    ///
    /// 실행 중에 멈춘 작업은 차례를 쥔 채 멈춘다: 뒤에서 기다리는 작업은 시작하지 않는다(디스크를 잠시 다른 일에
    /// 쓰려고 멈추는 것이므로). 뒤 작업을 돌리려면 그 작업을 [`JobQueue::start_now`]한다.
    ///
    /// 대기·실행 중이 아니면(이미 일시정지, 취소 중, 끝남, 없는 작업) 아무 일도 하지 않고 `false`.
    pub fn pause(&mut self, job: JobId) -> bool {
        let mut board = self.shared.board();
        let Some(slot) = board.jobs.get_mut(&job) else {
            return false;
        };
        if slot.cancelling {
            return false;
        }
        match slot.state {
            JobState::Queued => {}
            JobState::Running => slot.control.set_paused(true),
            _ => return false,
        }
        slot.state = JobState::Paused;
        self.outbox.send(JobEvent::StateChanged {
            job,
            state: JobState::Paused,
        });
        true
    }

    /// 일시정지한 작업을 잇는다. 실행 중에 멈췄으면 멈춘 자리에서 이어 [`JobState::Running`], 시작 전에 멈췄으면
    /// 차례의 제자리로 돌아가 [`JobState::Queued`]가 된다(앞에 도는 작업이 없으면 곧 시작한다).
    ///
    /// 일시정지가 아니면 아무 일도 하지 않고 `false`.
    pub fn resume(&mut self, job: JobId) -> bool {
        let mut board = self.shared.board();
        let Some(slot) = board.jobs.get_mut(&job) else {
            return false;
        };
        if slot.cancelling || slot.state != JobState::Paused {
            return false;
        }
        let state = if slot.job.is_some() {
            JobState::Queued
        } else {
            JobState::Running
        };
        slot.state = state;
        self.outbox.send(JobEvent::StateChanged { job, state });
        // 상태 사건을 보낸 뒤에 푼다: 풀린 작업의 진행 사건이 이어 감 사건보다 먼저 오지 않게.
        if state == JobState::Running {
            slot.control.set_paused(false);
        }
        drop(board);
        self.shared.changed.notify_all();
        true
    }

    /// 작업을 취소한다(대기·실행 중·일시정지).
    ///
    /// 시작하지 않은 작업은 곧바로 [`JobState::Cancelled`]와 끝남 사건을 보낸다(남은 항목 수는 넘긴 원본 수,
    /// [`Summary::all_counted`]는 `false`). 시작한 작업은 복사하던 파일을 그 자리에서 그만두고 만들다 만 파일을
    /// 지운 뒤 취소로 끝난다. 이미 다 복사한 항목은 되돌리지 않는다. 끝남 사건의 결과에 끝낸 항목 수와 남은 항목
    /// 수가 있다. 차례는 다음 작업으로 넘어간다.
    ///
    /// 거의 끝난 작업은 취소가 닿기 전에 마저 끝나 [`JobState::Finished`]로 끝날 수 있다. 끝났거나 이미 취소 중이면
    /// 아무 일도 하지 않고 `false`.
    pub fn cancel(&mut self, job: JobId) -> bool {
        let mut board = self.shared.board();
        let Some(slot) = board.jobs.get_mut(&job) else {
            return false;
        };
        if slot.cancelling {
            return false;
        }
        if slot.job.is_none() {
            slot.cancelling = true;
            slot.control.cancel();
            return true;
        }
        let slot = board.jobs.remove(&job).expect("방금 본 작업");
        board.pending.retain(|id| *id != job);
        let summary = Summary {
            items_remaining: slot.job.as_ref().map_or(0, Job::root_count),
            ..Summary::default()
        };
        self.outbox.send(JobEvent::StateChanged {
            job,
            state: JobState::Cancelled,
        });
        self.outbox.send(JobEvent::Finished { job, summary });
        true
    }

    /// 이름 충돌의 답을 기다리는 작업에 답한다. 작업은 [`JobState::Running`]이 되어(그 상태 사건을 보낸다) 차례를
    /// 다시 기다리지 않고 그 자리에서 이어 돈다.
    ///
    /// 답을 기다리지 않는 작업이면 [`AnswerError::NotAsking`]. 덮어쓸 수 없는 충돌(파일과 폴더)에 덮어쓰기·더
    /// 새것만 덮어쓰기로 답하면 [`AnswerError::OverwriteNotAllowed`]이고 작업은 계속 답을 기다린다.
    pub fn answer_conflict(
        &mut self,
        job: JobId,
        answer: ConflictAnswer,
    ) -> Result<(), AnswerError> {
        let mut board = self.shared.board();
        let Some(slot) = board.jobs.get_mut(&job) else {
            return Err(AnswerError::NotAsking);
        };
        let Some(Question::Conflict { can_overwrite }) = slot.question else {
            return Err(AnswerError::NotAsking);
        };
        if slot.cancelling || slot.state != JobState::AwaitingAnswer {
            return Err(AnswerError::NotAsking);
        }
        if answer.resolution.overwrites() && !can_overwrite {
            return Err(AnswerError::OverwriteNotAllowed);
        }
        slot.question = None;
        slot.state = JobState::Running;
        self.outbox.send(JobEvent::StateChanged {
            job,
            state: JobState::Running,
        });
        slot.control.set_answer(Answer::Conflict(answer));
        Ok(())
    }

    /// 항목별 오류의 답을 기다리는 작업에 답한다. 다시 시도·건너뛰기·나머지 오류도 건너뛰기면 작업은
    /// [`JobState::Running`]이 되어(그 상태 사건을 보낸다) 차례를 다시 기다리지 않고 그 자리에서 이어 돈다. 작업
    /// 취소면 [`JobQueue::cancel`]과 같다.
    ///
    /// 항목별 오류의 답을 기다리지 않는 작업이면 [`AnswerError::NotAsking`].
    pub fn answer_error(&mut self, job: JobId, answer: ErrorAnswer) -> Result<(), AnswerError> {
        let mut board = self.shared.board();
        let Some(slot) = board.jobs.get_mut(&job) else {
            return Err(AnswerError::NotAsking);
        };
        if slot.cancelling
            || slot.state != JobState::AwaitingAnswer
            || slot.question != Some(Question::Error)
        {
            return Err(AnswerError::NotAsking);
        }
        if answer == ErrorAnswer::CancelJob {
            drop(board);
            self.cancel(job);
            return Ok(());
        }
        slot.question = None;
        slot.state = JobState::Running;
        self.outbox.send(JobEvent::StateChanged {
            job,
            state: JobState::Running,
        });
        slot.control.set_answer(Answer::Error(answer));
        Ok(())
    }

    /// 휴지통에 들어가지 않는 항목의 답을 기다리는 휴지통 삭제 작업에 답한다. 작업 취소가 아니면 작업은
    /// [`JobState::Running`]이 되어(그 상태 사건을 보낸다) 그 자리에서 이어 돈다. 작업 취소면 [`JobQueue::cancel`]과
    /// 같다.
    ///
    /// 그 질문의 답을 기다리지 않는 작업이면 [`AnswerError::NotAsking`].
    pub fn answer_not_recyclable(
        &mut self,
        job: JobId,
        answer: NotRecyclableAnswer,
    ) -> Result<(), AnswerError> {
        let mut board = self.shared.board();
        let Some(slot) = board.jobs.get_mut(&job) else {
            return Err(AnswerError::NotAsking);
        };
        if slot.cancelling
            || slot.state != JobState::AwaitingAnswer
            || slot.question != Some(Question::NotRecyclable)
        {
            return Err(AnswerError::NotAsking);
        }
        if answer == NotRecyclableAnswer::CancelJob {
            drop(board);
            self.cancel(job);
            return Ok(());
        }
        slot.question = None;
        slot.state = JobState::Running;
        self.outbox.send(JobEvent::StateChanged {
            job,
            state: JobState::Running,
        });
        slot.control.set_answer(Answer::NotRecyclable(answer));
        Ok(())
    }

    /// 대기 중인 작업을 차례를 기다리지 않고 바로 시작한다. 이미 도는 작업과 나란히 돌고, 차례 실행은 그대로
    /// 이어진다(이 작업이 끝나도 차례의 다음 작업은 앞 작업이 끝나야 시작한다).
    ///
    /// 시작 전에 일시정지한 작업도 바로 시작한다(이어하기를 겸한다). 시작한 작업·끝난 작업이면 아무 일도 하지
    /// 않고 `false`.
    pub fn start_now(&mut self, job: JobId) -> bool {
        let mut board = self.shared.board();
        if !board.jobs.get(&job).is_some_and(|slot| slot.job.is_some()) {
            return false;
        }
        board.pending.retain(|id| *id != job);
        let Some((id, job, control)) = board.start(job, &self.outbox) else {
            return false;
        };
        drop(board);
        let shared = Arc::clone(&self.shared);
        let outbox = self.outbox.clone();
        thread::Builder::new()
            .name("facet-job-now".into())
            .spawn(move || run_job(id, job, &control, &shared, &outbox))
            .expect("작업 스레드를 띄우지 못했다");
        true
    }
}

/// 차례 일꾼: 차례를 잡은 작업이 없으면 차례의 맨 앞 대기 작업을 시작해 차례를 준다. 작업은 따로 스레드에서
/// 돌고, 끝나거나 답을 기다리느라 차례를 놓으면 다음 작업을 시작한다. 대기열을 버렸는데 시작할 작업이 없으면
/// 끝난다.
fn run_in_turn(shared: &Arc<Shared>, outbox: &Outbox) {
    loop {
        let (id, job, control) = {
            let mut board = shared.board();
            loop {
                if board.turn.is_none()
                    && let Some(next) = board.take_next(outbox)
                {
                    board.turn = Some(next.0);
                    break next;
                }
                let startable = board.pending.iter().any(|id| {
                    board
                        .jobs
                        .get(id)
                        .is_some_and(|slot| slot.state == JobState::Queued)
                });
                if board.closed && !startable {
                    return;
                }
                board = shared
                    .changed
                    .wait(board)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        };
        let shared = Arc::clone(shared);
        let outbox = outbox.clone();
        thread::Builder::new()
            .name("facet-job".into())
            .spawn(move || run_job(id, job, &control, &shared, &outbox))
            .expect("작업 스레드를 띄우지 못했다");
    }
}

/// 작업 하나를 이 스레드에서 끝까지 실행하고 마지막 상태와 끝남 사건을 보낸다. 차례를 잡고 있었으면 놓는다.
fn run_job(id: JobId, job: Job, control: &Control, shared: &Shared, outbox: &Outbox) {
    let context = Context {
        id,
        outbox,
        control,
        shared,
    };
    let result = panic::catch_unwind(AssertUnwindSafe(|| job.run(&context)));
    let (summary, state) = match result {
        Ok((summary, true)) => (summary, JobState::Cancelled),
        Ok((summary, false)) if summary.failures.is_empty() && summary.not_reverted.is_empty() => {
            (summary, JobState::Finished)
        }
        Ok((summary, false)) => (summary, JobState::Failed),
        // 작업 코드의 버그: 대기열은 계속 돌게 하고 그 작업만 실패로 끝낸다.
        Err(_) => (Summary::default(), JobState::Failed),
    };
    let mut board = shared.board();
    // 끝남을 보내기 전에 판에서 빼, 끝남을 받은 쪽이 보면 이미 끝난 것으로 보인다.
    board.jobs.remove(&id);
    if board.turn == Some(id) {
        board.turn = None;
    }
    outbox.send(JobEvent::StateChanged { job: id, state });
    outbox.send(JobEvent::Finished { job: id, summary });
    drop(board);
    shared.changed.notify_all();
}

/// `path`가 있는 위치(볼륨)에 휴지통이 있는가. 경로가 아직 없어도 된다. 실행 전에 알 수 있는 위치 정보다: 로컬
/// 고정 디스크는 있고, 이동식 디스크·네트워크 위치·볼륨을 알 수 없는 경로는 없다
/// ([`facet_win::recycle::has_recycle_bin`]). 사용자가 휴지통을 꺼 두었거나 휴지통보다 큰 항목은 미리 알 수 없고,
/// 휴지통 삭제가 그 항목에서 묻는다([`JobEvent::NotRecyclable`]).
pub fn has_recycle_bin(path: &Path) -> bool {
    facet_win::recycle::has_recycle_bin(path)
}

/// 원본들을 삭제하면 **영구 삭제**가 되는가: 원본이 있고 그 모두가 휴지통이 없는 위치에 있으면 `true`다. 앱은 삭제
/// 전에 이것으로 「영구 삭제됩니다」를 알리고, [`crate::window::App::delete`]는 이것으로 영구 삭제 작업과 휴지통
/// 삭제 작업을 고른다. 휴지통이 있는 위치와 없는 위치가 섞였으면 `false`다(휴지통 삭제가 휴지통에 들어가지 않는
/// 항목마다 묻는다).
pub fn deletes_permanently(sources: &[PathBuf]) -> bool {
    !sources.is_empty() && sources.iter().all(|source| !has_recycle_bin(source))
}

/// 두 경로가 같은지(Windows는 대소문자를 가리지 않는다). 둘 다 정규화한 경로라야 한다.
pub(crate) fn same_path(a: &Path, b: &Path) -> bool {
    let a: Vec<_> = a.components().collect();
    let b: Vec<_> = b.components().collect();
    a.len() == b.len() && a.iter().zip(&b).all(|(x, y)| same_component(x, y))
}

/// `path`가 `base`이거나 그 아래인지. 둘 다 정규화한 경로라야 한다.
pub(crate) fn is_within(path: &Path, base: &Path) -> bool {
    let path: Vec<_> = path.components().collect();
    let base: Vec<_> = base.components().collect();
    path.len() >= base.len() && base.iter().zip(&path).all(|(x, y)| same_component(x, y))
}

fn same_component(a: &std::path::Component<'_>, b: &std::path::Component<'_>) -> bool {
    if cfg!(windows) {
        a.as_os_str().to_string_lossy().to_lowercase()
            == b.as_os_str().to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 하위_경로를_가린다() {
        let base = Path::new("/a/b");
        assert!(is_within(Path::new("/a/b"), base));
        assert!(is_within(Path::new("/a/b/c"), base));
        assert!(!is_within(Path::new("/a/bc"), base));
        assert!(!is_within(Path::new("/a"), base));
        assert!(same_path(Path::new("/a/b"), base));
        assert!(!same_path(Path::new("/a/b/c"), base));
    }

    #[cfg(windows)]
    #[test]
    fn windows는_대소문자를_가리지_않는다() {
        assert!(is_within(Path::new(r"C:\Foo\Bar"), Path::new(r"c:\foo")));
        assert!(same_path(Path::new(r"C:\Foo"), Path::new(r"c:\FOO")));
    }

    #[test]
    fn 마지막_상태() {
        assert!(JobState::Finished.is_terminal());
        assert!(JobState::Failed.is_terminal());
        assert!(JobState::Cancelled.is_terminal());
        assert!(!JobState::Queued.is_terminal());
        assert!(!JobState::Running.is_terminal());
        assert!(!JobState::Paused.is_terminal());
        assert!(!JobState::AwaitingAnswer.is_terminal());
    }
}
