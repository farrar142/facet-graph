//! 명령: 사용자가 실행할 수 있는 이름 붙은 동작. 단축키와 명령 팔레트는 명령을 실행하는 통로일 뿐이다.
//!
//! - 명령은 바뀌지 않는 ID([`CommandId`])로 구별한다. 코어는 화면 문장(명령 이름)을 갖지 않는다(ADR-0007).
//!   명령 이름은 앱이 주고, 명령 팔레트 검색([`search`])은 앱이 넘긴 이름에서 찾는다.
//! - 명령마다 기본 단축키가 있다([`CommandId::shortcuts`]). 키 조합으로 명령을 찾는 것은 [`find_by_shortcut`]이다.
//! - 지금 실행할 수 있는지는 탭 상태에서 나온다([`CommandId::is_enabled`]). 탭 명령(n번째 탭 등)은 패널의 탭
//!   수도 본다([`CommandId::is_enabled_in`]). 패널 명령(활성 패널 바꾸기)은 창의 패널 수를 본다
//!   ([`CommandId::is_enabled_in_window`]). 실행은 [`crate::window::App::execute`].
//!
//! 단축키는 입력한 글자가 아니라 키로 맞춘다. 한글 IME가 한글 모드여서 H 키가 「ㅗ」를 내도 Ctrl+H는 Ctrl+H다.
//! 앱은 키 이벤트의 글자를 [`Key::from_char`]로 키로 바꾼다. 두벌식 자모는 그 자모를 내는 영문 키로 바뀐다.
//! 백슬래시 키([`Key::Backslash`])는 배열마다 내는 글자가 달라(한국어 「₩」, 독일어 「#」) 앱이 키 위치를 알면
//! 위치로 정한다. 글자로만 알 때는 「\」·「₩」·「￦」를 그 키로 본다.

use crate::details::{Column, Direction, Sort};
use crate::job::JobKind;
use crate::tab::Tab;
use crate::window::{Panel, Window};

/// 명령의 ID. 이 이름과 [`CommandId::as_str`]의 글자는 바뀌지 않는다(설정·단축키 바꾸기가 이것으로 명령을 가리킨다).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandId {
    /// 열기: 커서 항목이 폴더면 그 안으로 가고, 파일이면 기본 프로그램으로 연다.
    Open,
    /// 뒤로.
    Back,
    /// 앞으로.
    Forward,
    /// 위로.
    Up,
    /// 새로 고침.
    Refresh,
    /// 모두 선택.
    SelectAll,
    /// 커서 항목 선택 토글.
    ToggleCursorSelection,
    /// 주소창을 글자로 고쳐 쓰는 모드로.
    EditAddress,
    /// 숨김 항목 보이기/감추기.
    ToggleHidden,
    /// 명령 팔레트 열기.
    CommandPalette,
    /// 이름 열로 정렬(같은 열이면 방향을 뒤집는다).
    SortByName,
    /// 수정한 날짜 열로 정렬.
    SortByModified,
    /// 유형 열로 정렬.
    SortByType,
    /// 크기 열로 정렬.
    SortBySize,
    /// 위치 열(항목이 든 폴더)로 정렬. 검색 결과 위치에서만 켜진다.
    SortByLocation,
    /// 일치 수로 정렬(내용 검색 결과 위치에서만 켜짐).
    SortByMatches,
    /// 새 탭: 지금 위치의 탭을 활성 탭 바로 뒤에 열고 활성으로 한다.
    NewTab,
    /// 탭 닫기: 활성 탭을 닫는다. 패널의 마지막 탭이면 패널이, 창의 마지막 패널이면 창이 닫힌다.
    CloseTab,
    /// 다음 탭. 끝에서는 처음으로 돈다.
    NextTab,
    /// 이전 탭. 처음에서는 끝으로 돈다.
    PreviousTab,
    /// n번째 탭(1부터 8까지). 그 탭이 없으면 꺼진다.
    GoToTab(u8),
    /// 마지막 탭.
    LastTab,
    /// 새 탭에서 열기: 커서 항목인 폴더를 새 탭으로 활성 탭 바로 뒤에 연다. 지금 탭은 그대로 활성이다.
    OpenInNewTab,
    /// 닫은 탭 되살리기: 마지막에 닫은 탭을 활성 패널에 활성 탭으로 되살린다. 닫은 탭이 없으면 꺼진다
    /// (닫은 탭 목록은 앱 전체에 하나라 [`crate::window::App::can_execute`]가 본다).
    ReopenClosedTab,
    /// 패널 나누기/합치기: 패널이 하나면 좌우로 나누고(새 오른쪽 패널이 활성 탭과 같은 위치의 탭 하나로 시작해
    /// 활성이 된다), 둘이면 오른쪽 패널의 탭을 왼쪽 패널 탭 끝에 붙여 합친다.
    TogglePanelSplit,
    /// 활성 패널 바꾸기: 반대편 패널을 활성 패널로 한다. 패널이 하나면 꺼진다.
    SwitchPanel,
    /// 새 창: 활성 탭의 위치(창이 없으면 사용자 홈 폴더) 탭 하나로 새 창을 열고 활성 창으로 한다.
    NewWindow,
    /// 반대편 패널로 복사: 활성 탭의 선택(없으면 커서 항목)을 반대편 패널의 활성 탭 위치로 복사한다. 화면이 확인 줄을
    /// 띄우고([`crate::window::Execution::ConfirmTransfer`]) 확인하면 작업을 넣는다. 원본이 없거나 패널이 하나면
    /// 꺼진다.
    CopyToOtherPanel,
    /// 반대편 패널로 이동: 복사와 같고 이동 작업을 넣는다.
    MoveToOtherPanel,
    /// 복사 위치…: 활성 탭의 선택(없으면 커서 항목)을 고른 폴더로 복사한다. 화면이 대상 선택 창을 띄우고
    /// ([`crate::window::Execution::ChooseTransferTarget`]) 거기서 Enter가 확인이다. 기본 단축키가 없다. 패널이 하나뿐일
    /// 때는 「반대편 패널로 복사」(F7)도 이 선택 창을 연다.
    CopyTo,
    /// 이동 위치…: 복사 위치와 같고 이동 작업을 넣는다.
    MoveTo,
    /// 이름 바꾸기: 커서 항목의 이름 입력란을 연다([`crate::window::Execution::Rename`]). 확정하면
    /// [`crate::window::App::rename`]이 바꾼다. 커서 항목이 있고 위치가 실제 폴더일 때 켜진다.
    Rename,
    /// 일괄 이름 바꾸기: 활성 탭의 선택(둘 이상)에 규칙을 써 한꺼번에 이름을 바꾼다
    /// ([`crate::window::Execution::BatchRename`] → [`crate::window::App::start_batch_rename`]). 위치가 실제 폴더이거나
    /// 검색 결과 위치이고 두 항목 이상 골랐을 때만 켜진다(커서는 세지 않는다). 기본 단축키가 없다.
    BatchRename,
    /// 새 폴더: 활성 탭 위치에 폴더를 만든다([`crate::window::Execution::NewFolder`] →
    /// [`crate::window::App::new_folder`]). 위치가 실제 폴더일 때 켜진다.
    NewFolder,
    /// 실행취소: 작업 기록의 마지막 작업을 되돌린다([`crate::window::App::undo`]). 기록이 없거나 다른 실행취소·다시
    /// 실행이 도는 중이면 꺼진다(작업 기록은 앱 상태라 [`crate::window::App::can_execute`]가 본다). 마지막 기록이
    /// 되돌릴 수 없는 작업이면 켜져 있고, 실행하면 까닭을 돌려준다.
    Undo,
    /// 다시 실행: 실행취소한 작업을 다시 한다([`crate::window::App::redo`]). 다시 실행 목록이 비었거나 도는 중이면
    /// 꺼진다.
    Redo,
    /// 작업 기록 보기: 작업 기록을 읽기 전용 목록으로 연다([`crate::window::Execution::ShowHistory`]). 기록이 없으면
    /// 꺼진다.
    ShowHistory,
    /// 필터: 활성 패널에 필터 입력란을 열고 포커스를 준다([`crate::window::Execution::Filter`]). 치는 즉시 현재 폴더의
    /// 목록이 이름으로 좁혀진다([`crate::window::App::set_filter`]). 늘 켜져 있다.
    Filter,
    /// 검색: 활성 패널의 검색창에 포커스를 준다([`crate::window::Execution::Search`]). 검색창에서 Enter를 누르면
    /// [`crate::window::App::start_search`]가 현재 폴더 아래 실시간 탐색을 시작한다. 늘 켜져 있다.
    Search,
    /// 내용 검색: 활성 패널의 검색창을 「내용」으로 바꾸고 포커스를 준다([`crate::window::Execution::ContentSearch`]). Enter를
    /// 누르면 [`crate::window::App::start_content_search`]가 현재 폴더 아래 파일의 내용을 찾는다. 늘 켜져 있다. 기본
    /// 단축키는 없다.
    ContentSearch,
    /// 삭제: 활성 탭의 선택(없으면 커서 항목)을 휴지통으로 보낸다. 묻지 않고 곧바로 휴지통 삭제 작업을 넣는다. 원본이
    /// 모두 휴지통이 없는 위치에 있으면 영구 삭제가 되므로 화면이 먼저 확인받는다
    /// ([`crate::window::Execution::ConfirmDelete`]). 원본이 없거나 위치가 실제 폴더가 아니면 꺼진다.
    Delete,
    /// 영구 삭제: 활성 탭의 선택(없으면 커서 항목)을 휴지통을 거치지 않고 지운다. 늘 화면이 먼저 확인받는다
    /// ([`crate::window::Execution::ConfirmDelete`]). 켜지는 조건은 삭제와 같다.
    DeletePermanently,
    /// 복사: 활성 탭의 선택(없으면 커서 항목)을 클립보드에 복사 형식으로 올린다([`crate::window::App::execute`]가 클립보드
    /// 스레드에 맡긴다). 원본이 없으면 꺼진다.
    Copy,
    /// 잘라내기: 활성 탭의 선택(없으면 커서 항목)을 잘라내기 표시와 함께 클립보드에 올린다. 클립보드 변경 알림이 그 경로를
    /// 잘라내기 표시([`crate::clipboard::CutMarks`])로 앱 상태에 올려 모든 탭·창이 흐리게 그린다. 켜지는 조건은 복사와 같다.
    Cut,
    /// 붙여넣기: 클립보드의 파일을 활성 탭에 붙여넣는다(대상은 선택이 폴더 하나면 그 안, 아니면 현재 폴더,
    /// [`crate::window::App::paste_target`]). 복사면 복사 작업, 잘라내기 표시가 있으면 이동 작업을 넣는다. 클립보드에
    /// 파일이 있다는 것은 앱 상태라 [`crate::window::App::can_execute`]가 본다(클립보드를 부르지 않는다).
    Paste,
    /// 경로 복사: 선택한 항목의 전체 경로를 큰따옴표로 감싸 줄마다 하나씩 글자로 클립보드에 올린다. 선택이 없으면 현재
    /// 폴더 경로다(검색 결과 위치면 검색한 폴더). 위치가 실제 폴더이거나 검색 결과 위치일 때 켜진다.
    CopyPath,
    /// 폴더에서 보기: 커서 결과 항목의 상위 폴더를 같은 탭에서 열고(기록에 남는다) 그 항목을 선택·커서로 둔다. 검색 결과
    /// 위치에서 커서 항목이 있을 때만 켜진다.
    ShowInFolder,
    /// 검색 중지: 활성 탭의 검색을 멈추고 지금까지의 결과는 남긴다. 검색이 도는 중일 때만 켜진다. 기본 단축키는 없다
    /// (Esc 등은 앱이 붙인다).
    StopSearch,
    /// 현재 폴더 아래만: 즉시 검색의 범위를 「모든 볼륨」과 「현재 폴더 아래만」 사이에서 바꾸는 전환(켜짐 표시는
    /// [`CommandId::is_checked`]). 범위는 탭에 남고, 즉시 검색 결과 위치에서 바꾸면 같은 질의를 새 범위로 다시 보낸다. 검색을
    /// 시작한 위치가 실제 폴더가 아니면(이름·내용 검색 결과 위치) 꺼지고 모든 볼륨으로 찾는다. 기본 단축키는 없다.
    InstantUnderFolder,
}

impl CommandId {
    /// 모든 명령. 명령 팔레트는 검색어가 없으면 이 순서로 보인다.
    pub const ALL: [CommandId; 56] = [
        CommandId::Open,
        CommandId::Back,
        CommandId::Forward,
        CommandId::Up,
        CommandId::Refresh,
        CommandId::SelectAll,
        CommandId::ToggleCursorSelection,
        CommandId::EditAddress,
        CommandId::ToggleHidden,
        CommandId::CommandPalette,
        CommandId::SortByName,
        CommandId::SortByModified,
        CommandId::SortByType,
        CommandId::SortBySize,
        CommandId::NewTab,
        CommandId::CloseTab,
        CommandId::NextTab,
        CommandId::PreviousTab,
        CommandId::GoToTab(1),
        CommandId::GoToTab(2),
        CommandId::GoToTab(3),
        CommandId::GoToTab(4),
        CommandId::GoToTab(5),
        CommandId::GoToTab(6),
        CommandId::GoToTab(7),
        CommandId::GoToTab(8),
        CommandId::LastTab,
        CommandId::OpenInNewTab,
        CommandId::ReopenClosedTab,
        CommandId::TogglePanelSplit,
        CommandId::SwitchPanel,
        CommandId::NewWindow,
        CommandId::CopyToOtherPanel,
        CommandId::MoveToOtherPanel,
        CommandId::CopyTo,
        CommandId::MoveTo,
        CommandId::Rename,
        CommandId::BatchRename,
        CommandId::NewFolder,
        CommandId::Undo,
        CommandId::Redo,
        CommandId::ShowHistory,
        CommandId::Filter,
        CommandId::Search,
        CommandId::ContentSearch,
        CommandId::Delete,
        CommandId::DeletePermanently,
        CommandId::Copy,
        CommandId::Cut,
        CommandId::Paste,
        CommandId::CopyPath,
        CommandId::SortByLocation,
        CommandId::SortByMatches,
        CommandId::ShowInFolder,
        CommandId::StopSearch,
        CommandId::InstantUnderFolder,
    ];

    /// 바뀌지 않는 ID 글자. 화면 문장이 아니다.
    pub const fn as_str(self) -> &'static str {
        match self {
            CommandId::Open => "open",
            CommandId::Back => "back",
            CommandId::Forward => "forward",
            CommandId::Up => "up",
            CommandId::Refresh => "refresh",
            CommandId::SelectAll => "select-all",
            CommandId::ToggleCursorSelection => "toggle-cursor-selection",
            CommandId::EditAddress => "edit-address",
            CommandId::ToggleHidden => "toggle-hidden",
            CommandId::CommandPalette => "command-palette",
            CommandId::SortByName => "sort-by-name",
            CommandId::SortByModified => "sort-by-modified",
            CommandId::SortByType => "sort-by-type",
            CommandId::SortBySize => "sort-by-size",
            CommandId::NewTab => "new-tab",
            CommandId::CloseTab => "close-tab",
            CommandId::NextTab => "next-tab",
            CommandId::PreviousTab => "previous-tab",
            CommandId::GoToTab(1) => "go-to-tab-1",
            CommandId::GoToTab(2) => "go-to-tab-2",
            CommandId::GoToTab(3) => "go-to-tab-3",
            CommandId::GoToTab(4) => "go-to-tab-4",
            CommandId::GoToTab(5) => "go-to-tab-5",
            CommandId::GoToTab(6) => "go-to-tab-6",
            CommandId::GoToTab(7) => "go-to-tab-7",
            CommandId::GoToTab(8) => "go-to-tab-8",
            // 1..=8 밖의 번호는 명령 목록에 없다.
            CommandId::GoToTab(_) => "go-to-tab",
            CommandId::LastTab => "last-tab",
            CommandId::OpenInNewTab => "open-in-new-tab",
            CommandId::ReopenClosedTab => "reopen-closed-tab",
            CommandId::TogglePanelSplit => "toggle-panel-split",
            CommandId::SwitchPanel => "switch-panel",
            CommandId::NewWindow => "new-window",
            CommandId::CopyToOtherPanel => "copy-to-other-panel",
            CommandId::MoveToOtherPanel => "move-to-other-panel",
            CommandId::CopyTo => "copy-to",
            CommandId::MoveTo => "move-to",
            CommandId::Rename => "rename",
            CommandId::BatchRename => "batch-rename",
            CommandId::NewFolder => "new-folder",
            CommandId::Undo => "undo",
            CommandId::Redo => "redo",
            CommandId::ShowHistory => "show-history",
            CommandId::Filter => "filter",
            CommandId::Search => "search",
            CommandId::ContentSearch => "content-search",
            CommandId::Delete => "delete",
            CommandId::DeletePermanently => "delete-permanently",
            CommandId::Copy => "copy",
            CommandId::Cut => "cut",
            CommandId::Paste => "paste",
            CommandId::CopyPath => "copy-path",
            CommandId::SortByLocation => "sort-by-location",
            CommandId::SortByMatches => "sort-by-matches",
            CommandId::ShowInFolder => "show-in-folder",
            CommandId::StopSearch => "stop-search",
            CommandId::InstantUnderFolder => "instant-under-folder",
        }
    }

    /// ID 글자로 명령을 찾는다. [`CommandId::as_str`]의 반대다.
    pub fn from_id(id: &str) -> Option<CommandId> {
        CommandId::ALL.into_iter().find(|c| c.as_str() == id)
    }

    /// 그 열로 정렬하는 명령.
    pub const fn sort_by(column: Column) -> CommandId {
        match column {
            Column::Name => CommandId::SortByName,
            Column::Modified => CommandId::SortByModified,
            Column::Type => CommandId::SortByType,
            Column::Size => CommandId::SortBySize,
            Column::Location => CommandId::SortByLocation,
            Column::Matches => CommandId::SortByMatches,
        }
    }

    /// 정렬 명령이면 그 열.
    pub const fn sort_column(self) -> Option<Column> {
        match self {
            CommandId::SortByName => Some(Column::Name),
            CommandId::SortByModified => Some(Column::Modified),
            CommandId::SortByType => Some(Column::Type),
            CommandId::SortBySize => Some(Column::Size),
            CommandId::SortByLocation => Some(Column::Location),
            CommandId::SortByMatches => Some(Column::Matches),
            _ => None,
        }
    }

    /// 복사·이동 명령(반대편 패널로, 위치를 골라)이면 그 작업 종류.
    pub const fn transfer_kind(self) -> Option<JobKind> {
        match self {
            CommandId::CopyToOtherPanel | CommandId::CopyTo => Some(JobKind::Copy),
            CommandId::MoveToOtherPanel | CommandId::MoveTo => Some(JobKind::Move),
            _ => None,
        }
    }

    /// 기본 단축키. 여럿이면 앞의 것이 대표다(명령 팔레트에 먼저 보인다). 없으면 비어 있다.
    pub const fn shortcuts(self) -> &'static [Shortcut] {
        const fn key(key: Key) -> Shortcut {
            Shortcut::new(key)
        }
        const fn letter(c: char) -> Key {
            Key::Letter(c)
        }
        match self {
            CommandId::Open => const { &[key(Key::Enter)] },
            CommandId::Back => const { &[key(Key::Backspace), key(Key::Left).alt()] },
            CommandId::Forward => const { &[key(Key::Right).alt()] },
            CommandId::Up => const { &[key(Key::Up).alt()] },
            CommandId::Refresh => const { &[key(Key::Function(5))] },
            CommandId::SelectAll => const { &[key(letter('A')).ctrl()] },
            CommandId::ToggleCursorSelection => const { &[key(Key::Space).ctrl()] },
            CommandId::EditAddress => {
                const {
                    &[
                        key(letter('D')).alt(),
                        key(letter('L')).ctrl(),
                        key(Key::Function(4)),
                    ]
                }
            }
            CommandId::ToggleHidden => const { &[key(letter('H')).ctrl()] },
            CommandId::CommandPalette => const { &[key(letter('P')).ctrl().shift()] },
            CommandId::SortByName
            | CommandId::SortByModified
            | CommandId::SortByType
            | CommandId::SortBySize
            | CommandId::SortByLocation
            | CommandId::SortByMatches
            | CommandId::ContentSearch
            | CommandId::ShowInFolder
            | CommandId::StopSearch
            | CommandId::InstantUnderFolder
            | CommandId::OpenInNewTab => &[],
            CommandId::NewTab => const { &[key(letter('T')).ctrl()] },
            CommandId::CloseTab => const { &[key(letter('W')).ctrl()] },
            CommandId::NextTab => const { &[key(Key::Tab).ctrl()] },
            CommandId::PreviousTab => const { &[key(Key::Tab).ctrl().shift()] },
            CommandId::GoToTab(1) => const { &[key(Key::Digit(1)).ctrl()] },
            CommandId::GoToTab(2) => const { &[key(Key::Digit(2)).ctrl()] },
            CommandId::GoToTab(3) => const { &[key(Key::Digit(3)).ctrl()] },
            CommandId::GoToTab(4) => const { &[key(Key::Digit(4)).ctrl()] },
            CommandId::GoToTab(5) => const { &[key(Key::Digit(5)).ctrl()] },
            CommandId::GoToTab(6) => const { &[key(Key::Digit(6)).ctrl()] },
            CommandId::GoToTab(7) => const { &[key(Key::Digit(7)).ctrl()] },
            CommandId::GoToTab(8) => const { &[key(Key::Digit(8)).ctrl()] },
            CommandId::GoToTab(_) => &[],
            CommandId::LastTab => const { &[key(Key::Digit(9)).ctrl()] },
            CommandId::ReopenClosedTab => const { &[key(letter('T')).ctrl().shift()] },
            CommandId::TogglePanelSplit => const { &[key(Key::Backslash).ctrl()] },
            CommandId::SwitchPanel => const { &[key(Key::Tab)] },
            CommandId::NewWindow => const { &[key(letter('N')).ctrl()] },
            CommandId::CopyToOtherPanel => const { &[key(Key::Function(7))] },
            CommandId::MoveToOtherPanel => const { &[key(Key::Function(8))] },
            CommandId::CopyTo | CommandId::MoveTo => &[],
            CommandId::Rename => const { &[key(Key::Function(2))] },
            CommandId::BatchRename => &[],
            CommandId::NewFolder => const { &[key(letter('N')).ctrl().shift()] },
            CommandId::Undo => const { &[key(letter('Z')).ctrl()] },
            CommandId::Redo => const { &[key(letter('Y')).ctrl()] },
            CommandId::ShowHistory => &[],
            CommandId::Filter => const { &[key(letter('F')).ctrl().shift()] },
            CommandId::Search => const { &[key(letter('F')).ctrl(), key(letter('E')).ctrl()] },
            CommandId::Delete => const { &[key(Key::Delete)] },
            CommandId::DeletePermanently => const { &[key(Key::Delete).shift()] },
            CommandId::Copy => const { &[key(letter('C')).ctrl()] },
            CommandId::Cut => const { &[key(letter('X')).ctrl()] },
            CommandId::Paste => const { &[key(letter('V')).ctrl()] },
            CommandId::CopyPath => const { &[key(letter('C')).ctrl().shift()] },
        }
    }

    /// 탭 상태에서 지금 실행할 수 있는가.
    ///
    /// - 열기·커서 항목 선택 토글: 커서 항목이 있을 때.
    /// - 뒤로·앞으로: 그쪽 기록이 있을 때. 위로: 상위 위치가 있을 때(드라이브 루트에서 꺼진다).
    /// - 모두 선택: 보이는 항목이 있을 때.
    /// - 새 탭에서 열기: 커서 항목이 폴더일 때.
    /// - 이름 바꾸기: 커서 항목이 있고 탭 위치가 실제 폴더일 때. 새 폴더: 탭 위치가 실제 폴더일 때.
    /// - 반대편 패널로 복사·이동: 원본(선택, 없으면 커서 항목)이 있고 탭 위치가 실제 폴더일 때. 반대편 패널은
    ///   [`CommandId::is_enabled_in_window`]가 본다.
    /// - 삭제·영구 삭제·복사: 원본(선택, 없으면 커서 항목)이 있고 탭 위치가 실제 폴더일 때.
    /// - 붙여넣기·경로 복사: 탭 위치가 실제 폴더일 때(붙여넣기는 클립보드에 파일이 있을 때도, 앱 상태라
    ///   [`crate::window::App::can_execute`]가 본다).
    /// - 나머지는 늘 켜져 있다. 다음·이전·n번째 탭은 탭 하나만 보아서는 모르므로 켜져 있다고 하고, 패널의 탭
    ///   수는 [`CommandId::is_enabled_in`]이 본다. 닫은 탭 되살리기도 켜져 있다고 하고, 닫은 탭 목록은
    ///   [`crate::window::App::can_execute`]가 본다. 패널 나누기/합치기와 활성 패널 바꾸기도 켜져 있다고 하고, 창의
    ///   패널 수는 [`CommandId::is_enabled_in_window`]가 본다. 실행취소·다시 실행·작업 기록 보기도 켜져 있다고 하고,
    ///   작업 기록은 [`crate::window::App::can_execute`]가 본다.
    pub fn is_enabled(self, tab: &Tab) -> bool {
        match self {
            CommandId::Open | CommandId::ToggleCursorSelection => tab.cursor_item().is_some(),
            CommandId::OpenInNewTab => tab.cursor_item().is_some_and(|item| item.is_folder),
            CommandId::Back => tab.can_go_back(),
            CommandId::Forward => tab.can_go_forward(),
            CommandId::Up => tab.can_go_up(),
            CommandId::SelectAll => !tab.is_empty(),
            CommandId::CopyToOtherPanel
            | CommandId::MoveToOtherPanel
            | CommandId::CopyTo
            | CommandId::MoveTo
            | CommandId::Delete
            | CommandId::DeletePermanently
            | CommandId::Copy
            | CommandId::Cut => has_sources(tab),
            // 붙여넣을 곳(현재 폴더)이 실제 폴더여야 한다. 클립보드에 파일이 있는지는 앱 상태다. 검색 결과 위치에서는
            // 붙여넣을 폴더가 없어 꺼진다.
            CommandId::Paste => tab.location().file_system_path().is_some(),
            CommandId::CopyPath => has_real_items(tab),
            CommandId::Rename => has_real_items(tab) && tab.cursor_item().is_some(),
            CommandId::BatchRename => has_real_items(tab) && tab.selected_count() >= 2,
            // 검색 결과 위치에는 만들 폴더가 없다.
            CommandId::NewFolder => tab.location().file_system_path().is_some(),
            CommandId::SortByLocation => tab.location().is_search_results(),
            CommandId::SortByMatches => tab.location().search_scope().is_some_and(|scope| {
                matches!(scope.kind, crate::location::SearchKind::Content { .. })
            }),
            CommandId::ShowInFolder => {
                tab.location().is_search_results() && tab.cursor_item().is_some()
            }
            CommandId::StopSearch => tab.search().is_some_and(|status| status.running),
            CommandId::InstantUnderFolder => tab.instant_base().is_some(),
            CommandId::Refresh
            | CommandId::EditAddress
            | CommandId::ToggleHidden
            | CommandId::CommandPalette
            | CommandId::SortByName
            | CommandId::SortByModified
            | CommandId::SortByType
            | CommandId::SortBySize
            | CommandId::NewTab
            | CommandId::CloseTab
            | CommandId::NextTab
            | CommandId::PreviousTab
            | CommandId::GoToTab(_)
            | CommandId::LastTab
            | CommandId::ReopenClosedTab
            | CommandId::TogglePanelSplit
            | CommandId::SwitchPanel
            | CommandId::NewWindow
            | CommandId::Undo
            | CommandId::Redo
            | CommandId::ShowHistory
            | CommandId::Filter
            | CommandId::Search
            | CommandId::ContentSearch => true,
        }
    }

    /// 켜짐 표시가 있는 전환 명령이 지금 켜져 있는가(활성 탭 기준). 전환이 아닌 명령은 `false`다. 「숨김 항목 보이기」의 켜짐은
    /// 앱 상태라 오른쪽 메뉴가 따로 본다([`crate::menu`]).
    pub fn is_checked(self, tab: &Tab) -> bool {
        match self {
            CommandId::InstantUnderFolder => tab.instant_under(),
            _ => false,
        }
    }

    /// 패널 상태에서 지금 실행할 수 있는가. 탭 명령은 패널의 탭 수를 보고, 나머지는 활성 탭을 따른다
    /// ([`CommandId::is_enabled`]).
    ///
    /// - 다음·이전 탭: 다른 탭이 있을 때(탭이 둘 이상).
    /// - n번째 탭: 그 탭이 있을 때.
    pub fn is_enabled_in(self, panel: &Panel) -> bool {
        match self {
            CommandId::NextTab | CommandId::PreviousTab => panel.len() > 1,
            CommandId::GoToTab(n) => n >= 1 && usize::from(n) <= panel.len(),
            command => command.is_enabled(panel.active_tab()),
        }
    }

    /// 창 상태에서 지금 실행할 수 있는가. 패널 명령은 창의 패널 수를 보고, 나머지는 활성 패널을 따른다
    /// ([`CommandId::is_enabled_in`]).
    ///
    /// - 활성 패널 바꾸기: 패널이 둘일 때.
    /// - 패널 나누기/합치기: 늘 켜져 있다(하나면 나누고 둘이면 합친다).
    /// - 반대편 패널로 복사·이동: 활성 탭에 원본이 있을 때, 패널이 둘이면 반대편 패널의 활성 탭 위치가 실제 폴더일
    ///   때도. 패널이 하나여도 켜지고 대상 선택 창을 연다(「복사 위치…」와 같다).
    /// - 복사 위치…·이동 위치…: 활성 탭에 원본이 있을 때(패널 수와 상관없다).
    pub fn is_enabled_in_window(self, window: &Window) -> bool {
        match self {
            CommandId::SwitchPanel => window.is_split(),
            CommandId::CopyToOtherPanel | CommandId::MoveToOtherPanel => {
                window
                    .opposite_panel()
                    .is_none_or(|other| other.active_tab().location().file_system_path().is_some())
                    && self.is_enabled_in(window.active_panel())
            }
            command => command.is_enabled_in(window.active_panel()),
        }
    }
}

/// 명령의 실행 가능 여부를 정하는 상태: 탭 하나([`CommandId::is_enabled`]), 패널([`CommandId::is_enabled_in`]),
/// 창([`CommandId::is_enabled_in_window`]),
/// 앱 전체([`crate::window::App::can_execute`], 닫은 탭 목록까지 본다).
pub trait CommandState {
    /// `command`를 지금 실행할 수 있는가.
    fn is_enabled(&self, command: CommandId) -> bool;
}

impl CommandState for Tab {
    fn is_enabled(&self, command: CommandId) -> bool {
        command.is_enabled(self)
    }
}

impl CommandState for Panel {
    fn is_enabled(&self, command: CommandId) -> bool {
        command.is_enabled_in(self)
    }
}

impl CommandState for Window {
    fn is_enabled(&self, command: CommandId) -> bool {
        command.is_enabled_in_window(self)
    }
}

/// 복사·이동·삭제할 원본이 있는가: 선택이나 커서 항목이 있고, 탭의 항목이 실제 파일 시스템 항목이다(실제 폴더나
/// 검색 결과 위치).
fn has_sources(tab: &Tab) -> bool {
    has_real_items(tab) && (tab.selected_items().next().is_some() || tab.cursor_item().is_some())
}

/// 탭의 항목이 실제 파일 시스템 항목인가: 위치가 실제 폴더이거나 검색 결과 위치다(결과 항목은 제 폴더 경로가 있다).
fn has_real_items(tab: &Tab) -> bool {
    tab.location().file_system_path().is_some() || tab.location().is_search_results()
}

/// 열 머리글을 누른 것처럼 정렬을 바꾼다: 같은 열이면 방향을 뒤집고, 다른 열이면 그 열 오름차순.
pub fn sort_toggled(sort: Sort, column: Column) -> Sort {
    let direction = match (sort.column == column, sort.direction) {
        (true, Direction::Ascending) => Direction::Descending,
        (true, Direction::Descending) | (false, _) => Direction::Ascending,
    };
    Sort { column, direction }
}

/// 키보드의 키. 글자가 아니라 키다: 「a」와 「A」, 두벌식의 「ㅁ」은 모두 [`Key::Letter`]`('A')`다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// 영문 키. 대문자 `'A'..='Z'`로 쓴다. 소문자로 만들어도 [`Shortcut::new`]가 대문자로 맞춘다.
    Letter(char),
    /// 숫자 키 `0..=9`.
    Digit(u8),
    /// 기능 키 F1..F24.
    Function(u8),
    Enter,
    Backspace,
    Space,
    Tab,
    Escape,
    Delete,
    Insert,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    /// 백슬래시 키: US 배열에서 「\」를 내는 자리(한국어 배열의 「₩」 키, 독일어 배열의 「#」 키).
    Backslash,
}

impl Key {
    /// 키 이벤트가 낸 글자 하나에서 키를 찾는다. 영문자(대소문자), 숫자, 공백, 백슬래시(「₩」 포함), 그리고 한글
    /// IME가 한글 모드일 때 영문 키가 내는 두벌식 자모(Shift 쪽 쌍자음·ㅒ·ㅖ 포함)를 키로 바꾼다. 그 밖의 글자는
    /// `None`이다.
    ///
    /// Enter·화살표·기능 키처럼 글자가 없는 키는 앱이 자기 키 이벤트에서 직접 [`Key`]로 바꾼다.
    pub fn from_char(c: char) -> Option<Key> {
        match c {
            'a'..='z' | 'A'..='Z' => Some(Key::Letter(c.to_ascii_uppercase())),
            '0'..='9' => Some(Key::Digit(c as u8 - b'0')),
            ' ' => Some(Key::Space),
            // 한국어 글꼴·IME에서는 백슬래시가 「₩」로 보이거나 그 글자(U+20A9, 전각 U+FFE6)로 온다. U+001C는 Ctrl과
            // 함께 누른 백슬래시 키의 제어 글자다.
            '\\' | '\u{20a9}' | '\u{ffe6}' | '\u{1c}' => Some(Key::Backslash),
            _ => dubeolsik_key(c).map(Key::Letter),
        }
    }

    /// 영문자는 대문자로 맞춘다.
    const fn normalized(self) -> Key {
        match self {
            Key::Letter(c) => Key::Letter(c.to_ascii_uppercase()),
            key => key,
        }
    }
}

/// 두벌식 자판에서 자모 `c`를 내는 영문 키(대문자).
fn dubeolsik_key(c: char) -> Option<char> {
    const LAYOUT: [(char, char); 33] = [
        ('ㅂ', 'Q'),
        ('ㅈ', 'W'),
        ('ㄷ', 'E'),
        ('ㄱ', 'R'),
        ('ㅅ', 'T'),
        ('ㅛ', 'Y'),
        ('ㅕ', 'U'),
        ('ㅑ', 'I'),
        ('ㅐ', 'O'),
        ('ㅔ', 'P'),
        ('ㅁ', 'A'),
        ('ㄴ', 'S'),
        ('ㅇ', 'D'),
        ('ㄹ', 'F'),
        ('ㅎ', 'G'),
        ('ㅗ', 'H'),
        ('ㅓ', 'J'),
        ('ㅏ', 'K'),
        ('ㅣ', 'L'),
        ('ㅋ', 'Z'),
        ('ㅌ', 'X'),
        ('ㅊ', 'C'),
        ('ㅍ', 'V'),
        ('ㅠ', 'B'),
        ('ㅜ', 'N'),
        ('ㅡ', 'M'),
        // Shift 쪽.
        ('ㅃ', 'Q'),
        ('ㅉ', 'W'),
        ('ㄸ', 'E'),
        ('ㄲ', 'R'),
        ('ㅆ', 'T'),
        ('ㅒ', 'O'),
        ('ㅖ', 'P'),
    ];
    LAYOUT
        .iter()
        .find(|(jamo, _)| *jamo == c)
        .map(|(_, key)| *key)
}

/// 키 조합: 키 하나와 Ctrl·Alt·Shift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Shortcut {
    pub key: Key,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Shortcut {
    /// 수식 키 없는 키 하나.
    pub const fn new(key: Key) -> Shortcut {
        Shortcut {
            key: key.normalized(),
            ctrl: false,
            alt: false,
            shift: false,
        }
    }

    /// Ctrl을 더한다.
    pub const fn ctrl(mut self) -> Shortcut {
        self.ctrl = true;
        self
    }

    /// Alt를 더한다.
    pub const fn alt(mut self) -> Shortcut {
        self.alt = true;
        self
    }

    /// Shift를 더한다.
    pub const fn shift(mut self) -> Shortcut {
        self.shift = true;
        self
    }
}

/// 키 조합에 묶인 명령. 없으면 `None`이다.
pub fn find_by_shortcut(shortcut: Shortcut) -> Option<CommandId> {
    let shortcut = Shortcut {
        key: shortcut.key.normalized(),
        ..shortcut
    };
    CommandId::ALL
        .into_iter()
        .find(|command| command.shortcuts().contains(&shortcut))
}

/// 명령 팔레트 검색 결과 한 줄.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaletteEntry {
    pub command: CommandId,
    /// 기본 단축키. 비어 있을 수 있다.
    pub shortcuts: &'static [Shortcut],
    /// 지금 실행할 수 있는가. 아니면 팔레트에 꺼진 채로 보인다.
    pub enabled: bool,
}

/// 명령 팔레트 검색: 앱이 주는 명령 이름 `name`에 `query`가 부분 일치하는 명령을 찾는다.
///
/// - 대소문자를 가리지 않는다. 앞뒤 공백은 무시하고, 비면 모든 명령을 [`CommandId::ALL`] 순서로 준다.
/// - 이름이 검색어로 시작하는 명령이 먼저다. 그 안에서, 그리고 나머지는 [`CommandId::ALL`] 순서다.
/// - 실행 가능 여부는 `state`(탭·패널·앱)로 정한다([`CommandState`]). 꺼진 명령도 결과에 든다.
pub fn search<S: AsRef<str>>(
    query: &str,
    name: impl Fn(CommandId) -> S,
    state: &(impl CommandState + ?Sized),
) -> Vec<PaletteEntry> {
    let query = query.trim().to_lowercase();
    let mut found: Vec<(bool, PaletteEntry)> = CommandId::ALL
        .into_iter()
        .filter_map(|command| {
            let at = name(command).as_ref().to_lowercase().find(&query)?;
            Some((
                at != 0,
                PaletteEntry {
                    command,
                    shortcuts: command.shortcuts(),
                    enabled: state.is_enabled(command),
                },
            ))
        })
        .collect();
    // 안정 정렬이라 무리 안에서는 ALL 순서가 남는다.
    found.sort_by_key(|(at, _)| *at);
    found.into_iter().map(|(_, entry)| entry).collect()
}
