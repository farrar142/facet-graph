//! 오른쪽 메뉴: 자체 항목으로 된 메뉴의 구성(ADR-0006). 셸 항목은 나중에 이 메뉴 아래 구분선 뒤에 붙는다.
//!
//! 구성은 순수 함수다([`build`]). 이미 가진 상태([`MenuContext`])만 받고 파일 시스템·셸·클립보드를 부르지 않으므로
//! 선택이 1만 개여도 시간이 같다. 항목은 모두 명령이고([`MenuCommand::id`]) 실행은 기존 명령 실행 길
//! ([`crate::window::App::execute`])로 한다. 코어는 화면 문장을 갖지 않는다(ADR-0007): 이름은 앱의 명령 이름
//! 모듈이 주고, 하위 메뉴 이름은 [`SubmenuId`]를 앱이 문장으로 바꾼다.
//!
//! 할 수 없는 항목은 숨기지 않고 꺼서 자리를 고정한다: 같은 메뉴 종류([`MenuKind`])는 맥락이 달라도 항목 순서가
//! 같다. 검색 결과 위치의 항목 메뉴만 첫머리에 「폴더에서 보기」와 구분선이 더 붙고 그 아래는 같은 순서다.

use crate::command::CommandId;
use crate::details::{Direction, Sort};

/// 메뉴 종류. 항목 위에서 열면 [`MenuKind::Item`], 빈 곳이면 [`MenuKind::Background`]다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuKind {
    /// 선택한 항목(오른쪽 클릭한 항목이 선택에 없으면 그 항목 하나)에 적용하는 메뉴.
    Item,
    /// 빈 곳 메뉴: 현재 폴더에 적용한다.
    Background,
}

/// 메뉴 구성에 쓰는 맥락. 모두 이미 가진 상태이고 만드는 데 목록을 훑지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuContext {
    pub kind: MenuKind,
    /// 선택한 항목 수.
    pub selected_count: usize,
    /// 커서 항목이 있는가.
    pub has_cursor: bool,
    /// 커서 항목이 폴더인가.
    pub cursor_is_folder: bool,
    /// 선택이 폴더 하나뿐인가(붙여넣을 곳이 그 폴더 안이 된다).
    pub single_folder_selected: bool,
    /// 탭 위치가 실제 폴더인가.
    pub location_is_folder: bool,
    /// 탭 위치가 검색 결과 위치인가. 결과 항목은 실제 파일 시스템 항목이라 파일 작업은 되고, 붙여넣기·새 폴더는
    /// 꺼지며, 항목 메뉴 첫머리에 「폴더에서 보기」가 붙는다.
    pub location_is_results: bool,
    /// 창의 패널 수(1 또는 2).
    pub panel_count: usize,
    /// 패널이 둘일 때 반대편 패널 위치가 실제 폴더인가. 패널이 하나면 쓰지 않는다.
    pub opposite_is_folder: bool,
    /// 클립보드에 파일이 있는가.
    pub clipboard_has_files: bool,
    /// 지금 정렬.
    pub sort: Sort,
    /// 숨김 항목을 보이는가.
    pub show_hidden: bool,
    /// 활성 탭에 뒤로 갈 기록이 있는가.
    pub can_go_back: bool,
    /// 활성 탭에 앞으로 갈 기록이 있는가.
    pub can_go_forward: bool,
    /// 활성 탭에 상위 위치가 있는가.
    pub can_go_up: bool,
    /// 실행취소 대상이 있는가.
    pub can_undo: bool,
}

/// 메뉴 항목에 붙는 표시.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// 표시 없음.
    None,
    /// 켜진 상태(숨김 항목 보이기가 켜짐).
    Checked,
    /// 지금 정렬 기준이고 방향이 이렇다.
    Sorted(Direction),
}

/// 명령 항목.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuCommand {
    pub id: CommandId,
    /// 지금 고를 수 있는가. 꺼진 항목도 자리는 그대로 있다.
    pub enabled: bool,
    pub mark: Mark,
}

/// 하위 메뉴의 이름.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmenuId {
    /// 정렬 기준.
    SortBy,
}

/// 하위 메뉴.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submenu {
    pub id: SubmenuId,
    pub enabled: bool,
    pub entries: Vec<MenuCommand>,
}

/// 메뉴의 한 줄.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuEntry {
    Command(MenuCommand),
    Submenu(Submenu),
    Separator,
}

/// 오른쪽 메뉴의 자체 항목.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu {
    pub kind: MenuKind,
    pub entries: Vec<MenuEntry>,
}

impl Menu {
    /// 명령 항목들(하위 메뉴 안은 빼고)을 순서대로.
    pub fn commands(&self) -> impl Iterator<Item = &MenuCommand> {
        self.entries.iter().filter_map(|entry| match entry {
            MenuEntry::Command(command) => Some(command),
            _ => None,
        })
    }
}

/// 항목 메뉴의 고정 순서. 구분선은 `None`이다.
const ITEM_LAYOUT: [Option<CommandId>; 15] = [
    Some(CommandId::Open),
    Some(CommandId::OpenInNewTab),
    None,
    Some(CommandId::Cut),
    Some(CommandId::Copy),
    Some(CommandId::Paste),
    Some(CommandId::CopyPath),
    None,
    Some(CommandId::CopyToOtherPanel),
    Some(CommandId::MoveToOtherPanel),
    Some(CommandId::CopyTo),
    Some(CommandId::MoveTo),
    None,
    Some(CommandId::Rename),
    // 셸 항목은 이 아래에 구분선을 두고 붙인다(S9).
    Some(CommandId::Delete),
];

/// 정렬 하위 메뉴의 고정 순서.
const SORT_LAYOUT: [CommandId; 4] = [
    CommandId::SortByName,
    CommandId::SortByModified,
    CommandId::SortByType,
    CommandId::SortBySize,
];

/// 맥락에서 메뉴를 만든다. 항목 순서는 맥락과 상관없이 [`MenuKind`]마다 같다.
pub fn build(context: &MenuContext) -> Menu {
    let entries = match context.kind {
        MenuKind::Item => {
            let results = context
                .location_is_results
                .then(|| {
                    [
                        MenuEntry::Command(command(CommandId::ShowInFolder, context)),
                        MenuEntry::Separator,
                    ]
                })
                .into_iter()
                .flatten();
            results
                .chain(ITEM_LAYOUT.iter().map(|slot| match slot {
                    Some(id) => MenuEntry::Command(command(*id, context)),
                    None => MenuEntry::Separator,
                }))
                .collect()
        }
        MenuKind::Background => {
            let c = |id| MenuEntry::Command(command(id, context));
            vec![
                c(CommandId::Back),
                c(CommandId::Forward),
                c(CommandId::Up),
                MenuEntry::Separator,
                c(CommandId::Paste),
                c(CommandId::NewFolder),
                c(CommandId::Undo),
                MenuEntry::Separator,
                MenuEntry::Submenu(Submenu {
                    id: SubmenuId::SortBy,
                    enabled: true,
                    entries: SORT_LAYOUT.iter().map(|id| command(*id, context)).collect(),
                }),
                c(CommandId::ToggleHidden),
                c(CommandId::Refresh),
                MenuEntry::Separator,
                c(CommandId::Search),
                c(CommandId::TogglePanelSplit),
                MenuEntry::Separator,
                c(CommandId::CopyPath),
            ]
        }
    };
    Menu {
        kind: context.kind,
        entries,
    }
}

fn command(id: CommandId, c: &MenuContext) -> MenuCommand {
    // 결과 항목도 실제 파일 시스템 항목이다(제 폴더 경로가 있다).
    let real_items = c.location_is_folder || c.location_is_results;
    let has_sources = real_items && (c.selected_count > 0 || c.has_cursor);
    let enabled = match id {
        CommandId::Open => c.has_cursor,
        CommandId::OpenInNewTab => c.cursor_is_folder,
        CommandId::Cut | CommandId::Copy | CommandId::CopyTo | CommandId::MoveTo => has_sources,
        CommandId::CopyToOtherPanel | CommandId::MoveToOtherPanel => {
            has_sources && (c.panel_count < 2 || c.opposite_is_folder)
        }
        // 항목 메뉴에서는 선택한 폴더 하나 안에 붙여넣는다. 빈 곳 메뉴는 현재 폴더에 붙여넣는다.
        CommandId::Paste => {
            c.location_is_folder
                && c.clipboard_has_files
                && (c.kind == MenuKind::Background || c.single_folder_selected)
        }
        CommandId::NewFolder => c.location_is_folder,
        CommandId::CopyPath => real_items,
        CommandId::Rename => real_items && c.has_cursor,
        CommandId::ShowInFolder => c.location_is_results && c.has_cursor,
        CommandId::Delete => has_sources,
        CommandId::Back => c.can_go_back,
        CommandId::Forward => c.can_go_forward,
        CommandId::Up => c.can_go_up,
        CommandId::Undo => c.can_undo,
        _ => true,
    };
    let mark = match id {
        CommandId::ToggleHidden if c.show_hidden => Mark::Checked,
        id => match id.sort_column() {
            Some(column) if column == c.sort.column => Mark::Sorted(c.sort.direction),
            _ => Mark::None,
        },
    };
    MenuCommand { id, enabled, mark }
}
