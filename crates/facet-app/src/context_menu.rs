//! 오른쪽 메뉴의 화면 상태: 코어가 만든 메뉴([`facet_core::menu::Menu`])와 고른 줄, 열린 하위 메뉴, 놓일 자리.
//! 무엇이 들어 있고 켜졌는지는 코어가 정한다. 이 모듈은 ↑↓·→·←·Enter를 줄 고르기로 바꾸고, 창 밖으로 넘치지 않게
//! 자리를 정한다(Slint도 같은 줄 높이·너비로 그린다: `app.slint`의 `ContextMenuView`).

use facet_core::command::CommandId;
use facet_core::menu::{Menu, MenuEntry};

/// 명령 줄의 높이.
pub const ROW_HEIGHT: f32 = 28.0;
/// 구분선 줄의 높이.
pub const SEPARATOR_HEIGHT: f32 = 9.0;
/// 메뉴 위아래 안쪽 여백.
pub const PADDING: f32 = 4.0;
/// 메뉴 너비.
pub const WIDTH: f32 = 300.0;
/// 하위 메뉴 너비.
pub const SUB_WIDTH: f32 = 200.0;

/// 메뉴 안에서 한 일.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// 아무 일도 없다(고른 줄이나 열린 하위 메뉴만 바뀌었을 수 있다).
    None,
    /// 이 명령을 실행하고 메뉴를 닫는다.
    Run(CommandId),
}

/// 열려 있는 오른쪽 메뉴.
#[derive(Debug, Clone)]
pub struct ContextMenu {
    menu: Menu,
    /// 고른 줄(메뉴 항목 번호).
    selected: Option<usize>,
    /// 열린 하위 메뉴가 있으면 그 줄(메뉴 항목 번호)과 고른 하위 줄.
    sub: Option<(usize, Option<usize>)>,
}

/// 줄 하나가 고를 수 있는 줄인가.
fn selectable(entry: &MenuEntry) -> bool {
    match entry {
        MenuEntry::Command(c) => c.enabled,
        MenuEntry::Submenu(s) => s.enabled,
        MenuEntry::Separator => false,
    }
}

impl ContextMenu {
    pub fn new(menu: Menu) -> ContextMenu {
        ContextMenu {
            menu,
            selected: None,
            sub: None,
        }
    }

    pub fn menu(&self) -> &Menu {
        &self.menu
    }

    /// 고른 줄(메뉴 항목 번호).
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// 열린 하위 메뉴의 줄 번호(메뉴 항목 번호).
    pub fn open_submenu(&self) -> Option<usize> {
        self.sub.map(|(row, _)| row)
    }

    /// 고른 하위 줄.
    pub fn sub_selected(&self) -> Option<usize> {
        self.sub.and_then(|(_, selected)| selected)
    }

    /// 하위 메뉴의 명령 번호 목록.
    fn sub_len(&self) -> usize {
        match self.sub.and_then(|(row, _)| self.menu.entries.get(row)) {
            Some(MenuEntry::Submenu(s)) => s.entries.len(),
            _ => 0,
        }
    }

    fn sub_enabled(&self, i: usize) -> bool {
        match self.sub.and_then(|(row, _)| self.menu.entries.get(row)) {
            Some(MenuEntry::Submenu(s)) => s.entries.get(i).is_some_and(|c| c.enabled),
            _ => false,
        }
    }

    /// 한 줄 아래(`delta` 1)나 위(-1)의 고를 수 있는 줄로 간다. 하위 메뉴가 열려 있으면 그 안에서, 끝에서는 반대편
    /// 끝으로 돈다. 꺼진 줄과 구분선은 건너뛴다.
    pub fn step(&mut self, delta: i32) {
        if let Some((row, selected)) = self.sub {
            let len = self.sub_len();
            let next = cycle(len, selected, delta, |i| self.sub_enabled(i));
            self.sub = Some((row, next.or(selected)));
        } else {
            let len = self.menu.entries.len();
            let next = cycle(len, self.selected, delta, |i| {
                selectable(&self.menu.entries[i])
            });
            self.selected = next.or(self.selected);
        }
    }

    /// 하위 메뉴를 연다: 고른 줄이 하위 메뉴면 열고 첫 줄을 고른다. 열었으면 `true`.
    pub fn open_sub(&mut self) -> bool {
        let Some(row) = self.selected else {
            return false;
        };
        if self.sub.is_some() {
            return false;
        }
        if let Some(MenuEntry::Submenu(s)) = self.menu.entries.get(row)
            && s.enabled
        {
            let first = (0..s.entries.len()).find(|i| s.entries[*i].enabled);
            self.sub = Some((row, first));
            return true;
        }
        false
    }

    /// ←: 하위 메뉴가 열려 있으면 닫는다(그 줄이 고른 줄로 남는다). 닫았으면 `true`.
    pub fn close_sub(&mut self) -> bool {
        self.sub.take().is_some()
    }

    /// 메뉴 줄 위로 마우스가 왔다. 고를 수 없는 줄이면 고른 줄을 지운다. 하위 메뉴 줄이면 열고, 다른 줄이면 연 하위
    /// 메뉴를 닫는다.
    pub fn hover(&mut self, row: usize) {
        let Some(entry) = self.menu.entries.get(row) else {
            return;
        };
        if !selectable(entry) {
            return;
        }
        if self.selected != Some(row) {
            self.sub = None;
        }
        self.selected = Some(row);
        if matches!(entry, MenuEntry::Submenu(_)) && self.sub.is_none() {
            self.open_sub();
        }
    }

    /// 하위 메뉴 줄 위로 마우스가 왔다.
    pub fn hover_sub(&mut self, i: usize) {
        if let Some((row, _)) = self.sub
            && self.sub_enabled(i)
        {
            self.sub = Some((row, Some(i)));
        }
    }

    /// 메뉴 줄을 눌렀다(또는 Enter): 명령이면 실행하고, 하위 메뉴면 연다. 꺼진 줄·구분선은 아무 일도 없다.
    pub fn activate(&mut self, row: usize) -> Outcome {
        match self.menu.entries.get(row) {
            Some(MenuEntry::Command(c)) if c.enabled => Outcome::Run(c.id),
            Some(MenuEntry::Submenu(s)) if s.enabled => {
                self.selected = Some(row);
                if self.sub.is_none() {
                    self.open_sub();
                }
                Outcome::None
            }
            _ => Outcome::None,
        }
    }

    /// 하위 메뉴 줄을 눌렀다(또는 Enter).
    pub fn activate_sub(&self, i: usize) -> Outcome {
        match self.sub.and_then(|(row, _)| self.menu.entries.get(row)) {
            Some(MenuEntry::Submenu(s)) => match s.entries.get(i) {
                Some(c) if c.enabled => Outcome::Run(c.id),
                _ => Outcome::None,
            },
            _ => Outcome::None,
        }
    }

    /// Enter: 고른 줄을 누른 것으로 한다.
    pub fn enter(&mut self) -> Outcome {
        if let Some((_, Some(i))) = self.sub {
            return self.activate_sub(i);
        }
        match self.selected {
            Some(row) => self.activate(row),
            None => Outcome::None,
        }
    }

    /// 메뉴 전체의 높이.
    pub fn height(&self) -> f32 {
        height_of(
            self.menu
                .entries
                .iter()
                .map(|e| matches!(e, MenuEntry::Separator)),
        )
    }

    /// 메뉴 항목 `row`번 줄의 위쪽 y(메뉴 위쪽 기준).
    pub fn row_top(&self, row: usize) -> f32 {
        PADDING
            + self
                .menu
                .entries
                .iter()
                .take(row)
                .map(|e| match e {
                    MenuEntry::Separator => SEPARATOR_HEIGHT,
                    _ => ROW_HEIGHT,
                })
                .sum::<f32>()
    }

    /// 열린 하위 메뉴의 높이. 열려 있지 않으면 0.
    pub fn sub_height(&self) -> f32 {
        height_of(std::iter::repeat_n(false, self.sub_len()))
    }
}

fn height_of(separators: impl Iterator<Item = bool>) -> f32 {
    2.0 * PADDING
        + separators
            .map(|s| if s { SEPARATOR_HEIGHT } else { ROW_HEIGHT })
            .sum::<f32>()
}

/// `len`개 줄에서 `from`의 다음(`delta`가 양수)·이전 줄 가운데 `ok`인 줄. 끝에서 돈다. 없으면 `None`.
fn cycle(len: usize, from: Option<usize>, delta: i32, ok: impl Fn(usize) -> bool) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let forward = delta >= 0;
    let start = match (from, forward) {
        (Some(at), _) => at,
        (None, true) => len - 1,
        (None, false) => 0,
    };
    (1..=len)
        .map(|step| {
            if forward {
                (start + step) % len
            } else {
                (start + len - step % len) % len
            }
        })
        .find(|i| ok(*i))
}

/// 점 `at`에 크기 `size`의 메뉴를 놓는다: 오른쪽·아래로 넘치면 안쪽으로 옮긴다(`bounds`는 창 크기). 창보다 크면
/// 왼쪽·위에 붙는다.
pub fn place(at: (f32, f32), size: (f32, f32), bounds: (f32, f32)) -> (f32, f32) {
    let x = at.0.min(bounds.0 - size.0).max(0.0);
    let y = at.1.min(bounds.1 - size.1).max(0.0);
    (x, y)
}

/// 하위 메뉴를 놓는다: 메뉴 오른쪽에 `row_y`(그 줄의 위쪽, 창 기준)부터. 오른쪽에 자리가 없으면 메뉴 왼쪽에 놓고
/// 아래로 넘치면 위로 올린다.
pub fn place_sub(
    menu_x: f32,
    menu_width: f32,
    row_y: f32,
    size: (f32, f32),
    bounds: (f32, f32),
) -> (f32, f32) {
    let right = menu_x + menu_width;
    let x = if right + size.0 <= bounds.0 {
        right
    } else {
        (menu_x - size.0).max(0.0)
    };
    let y = row_y.min(bounds.1 - size.1).max(0.0);
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use facet_core::details::{Column, Direction, Sort};
    use facet_core::menu::{MenuContext, MenuKind, build};

    fn context(kind: MenuKind) -> MenuContext {
        MenuContext {
            kind,
            selected_count: 1,
            has_cursor: true,
            cursor_is_folder: false,
            single_folder_selected: false,
            location_is_folder: true,
            location_is_results: false,
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
            can_go_up: false,
            can_undo: false,
        }
    }

    fn item_menu() -> ContextMenu {
        ContextMenu::new(build(&context(MenuKind::Item)))
    }

    #[test]
    fn arrow_keys_skip_disabled_rows_and_separators() {
        let mut menu = item_menu();
        // 파일 하나: 열기(켜짐), 새 탭에서 열기(꺼짐), 구분선, 잘라내기(켜짐)…
        menu.step(1);
        assert_eq!(menu.selected(), Some(0));
        menu.step(1);
        assert_eq!(menu.selected(), Some(3), "꺼진 줄과 구분선을 건너뛴다");
        menu.step(-1);
        assert_eq!(menu.selected(), Some(0));
        menu.step(-1);
        assert_eq!(menu.selected(), Some(14), "처음에서 위로 가면 끝으로 돈다");
    }

    #[test]
    fn enter_runs_the_selected_command_but_not_disabled_ones() {
        let mut menu = item_menu();
        menu.step(1);
        assert_eq!(menu.enter(), Outcome::Run(CommandId::Open));
        // 꺼진 줄(새 탭에서 열기)은 눌러도 아무 일이 없다.
        assert_eq!(menu.activate(1), Outcome::None);
        menu.hover(1);
        assert_eq!(menu.selected(), Some(0), "꺼진 줄 위로는 고르지 않는다");
    }

    #[test]
    fn submenu_opens_with_right_and_closes_with_left() {
        let mut menu = ContextMenu::new(build(&context(MenuKind::Background)));
        // 꺼진 뒤로·앞으로·위로·붙여넣기는 건너뛰고 새 폴더 → (꺼진 실행취소 건너뜀) 정렬 기준(하위 메뉴).
        menu.step(1);
        assert_eq!(menu.selected(), Some(5));
        menu.step(1);
        assert_eq!(menu.selected(), Some(8));
        assert!(menu.open_sub());
        assert_eq!(menu.open_submenu(), Some(8));
        assert_eq!(menu.sub_selected(), Some(0));
        menu.step(1);
        assert_eq!(menu.sub_selected(), Some(1));
        assert_eq!(menu.enter(), Outcome::Run(CommandId::SortByModified));
        assert!(menu.close_sub());
        assert_eq!(menu.selected(), Some(8));
        assert!(!menu.close_sub());
    }

    #[test]
    fn enter_on_a_submenu_row_opens_it() {
        let mut menu = ContextMenu::new(build(&context(MenuKind::Background)));
        menu.hover(8);
        assert_eq!(menu.open_submenu(), Some(8), "마우스를 올리면 열린다");
        menu.hover(9);
        assert_eq!(menu.open_submenu(), None, "다른 줄로 가면 닫힌다");
        menu.hover(8);
        menu.close_sub();
        assert_eq!(menu.enter(), Outcome::None);
        assert_eq!(menu.open_submenu(), Some(8));
    }

    #[test]
    fn placement_keeps_the_menu_inside_the_window() {
        let size = (300.0, 400.0);
        let bounds = (1000.0, 700.0);
        assert_eq!(place((100.0, 100.0), size, bounds), (100.0, 100.0));
        assert_eq!(place((900.0, 100.0), size, bounds), (700.0, 100.0));
        assert_eq!(place((100.0, 600.0), size, bounds), (100.0, 300.0));
        assert_eq!(place((50.0, 50.0), (2000.0, 2000.0), bounds), (0.0, 0.0));
        // 하위 메뉴: 오른쪽에 자리가 없으면 왼쪽.
        assert_eq!(
            place_sub(100.0, 300.0, 50.0, (200.0, 120.0), bounds),
            (400.0, 50.0)
        );
        assert_eq!(
            place_sub(700.0, 300.0, 50.0, (200.0, 120.0), bounds),
            (500.0, 50.0)
        );
        assert_eq!(
            place_sub(100.0, 300.0, 650.0, (200.0, 120.0), bounds),
            (400.0, 580.0)
        );
    }

    #[test]
    fn heights_follow_the_rows() {
        let menu = item_menu();
        // 열두 줄과 구분선 셋.
        assert_eq!(
            menu.height(),
            8.0 + 12.0 * ROW_HEIGHT + 3.0 * SEPARATOR_HEIGHT
        );
        assert_eq!(menu.row_top(0), PADDING);
        assert_eq!(
            menu.row_top(3),
            PADDING + 2.0 * ROW_HEIGHT + SEPARATOR_HEIGHT
        );
    }
}
