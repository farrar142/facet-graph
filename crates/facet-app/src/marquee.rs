//! 사각형 선택: 자세히 보기의 빈 곳(목록 아래, 행의 이름 글자 오른쪽 여백)에서 누른 왼쪽 버튼을 끌기 시작 문턱
//! ([`DRAG_THRESHOLD`], 탭 끌기와 같음)을 넘게 움직이면 사각형 선택이 된다. 문턱을 넘기 전에 놓으면 누르기다.
//! 화면에 기대지 않는 계산만 둔다: 끄는 점과 목록 자리로 그릴 사각형, 사각형이 걸친 행 범위, 목록 위·아래 끝 밖으로
//! 끌 때 자동으로 넘길 거리. 행 범위로 선택을 바꾸는 것은 코어(`Tab::marquee_select`)가 한다.
//!
//! 좌표는 둘이다. 창 기준 점([`Point`], 논리 픽셀)과 목록 내용 기준 점([`Rect`], 목록 왼쪽과 첫 행 위가 0). 사각형의
//! 시작점은 내용 기준으로 기억하므로 목록이 넘어가도 시작한 행에 붙어 있다.

use crate::tab_drag::{DRAG_THRESHOLD, Point};

/// 목록이 놓인 자리: 보이는 영역의 창 기준 왼쪽 위와 크기(스크롤 막대 제외), 넘긴 거리(0 이상), 행 높이, 행 수.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct List {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
    pub scroll: f32,
    pub row_height: f32,
    pub rows: usize,
}

impl List {
    /// 넘길 수 있는 가장 먼 거리. 목록 끝에는 빈 곳 한 행 높이가 늘 있다(`ui/app.slint`의 목록 내용 높이).
    pub fn max_scroll(&self) -> f32 {
        ((self.rows + 1) as f32 * self.row_height - self.height).max(0.0)
    }
}

/// 목록 내용 기준 사각형(논리 픽셀).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// 끌기 하나. 빈 곳을 누르면 생기고 놓으면 없어진다. `T`는 누른 탭의 번호(앱은 코어의 탭 번호)다.
#[derive(Debug, Clone)]
pub struct Marquee<T> {
    /// 누른 패널(0 왼쪽, 1 오른쪽).
    pub panel: usize,
    /// 누른 패널의 보이던 탭. 끄는 동안 바뀌면(다른 곳으로 이동 등) 끌기를 그만둔다.
    pub tab: T,
    /// 누른 행(이름 글자 오른쪽 여백). 목록 아래 빈 곳이면 `None`. 누르기로 끝나면 이 행을 고른다.
    pub row: Option<usize>,
    /// 누를 때 Ctrl·Shift. Ctrl이면 사각형 선택이 끌기 전 선택에 더한다.
    pub ctrl: bool,
    pub shift: bool,
    /// 누른 창 기준 점.
    pressed: Point,
    /// 누른 점의 목록 내용 기준 자리.
    start: (f32, f32),
    /// 마지막 끄는 점(창 기준).
    current: Point,
    dragging: bool,
}

impl<T> Marquee<T> {
    /// 목록 `list`의 빈 곳을 창 기준 점 `at`에서 눌렀다.
    pub fn new(
        panel: usize,
        tab: T,
        row: Option<usize>,
        modifiers: (bool, bool),
        at: Point,
        list: &List,
    ) -> Self {
        let (ctrl, shift) = modifiers;
        Marquee {
            panel,
            tab,
            row,
            ctrl,
            shift,
            pressed: at,
            start: (
                (at.x - list.left).clamp(0.0, list.width.max(0.0)),
                (at.y - list.top).clamp(0.0, list.height.max(0.0)) + list.scroll,
            ),
            current: at,
            dragging: false,
        }
    }

    /// 누른 채 창 기준 점 `at`으로 움직였다. 누른 점에서 끌기 시작 문턱을 넘으면 그때부터 사각형 선택이다(한 번
    /// 넘으면 돌아와도 끌기다). 사각형 선택이면 참이다.
    pub fn moved(&mut self, at: Point) -> bool {
        self.current = at;
        if !self.dragging {
            let (dx, dy) = (at.x - self.pressed.x, at.y - self.pressed.y);
            self.dragging = dx.hypot(dy) > DRAG_THRESHOLD;
        }
        self.dragging
    }

    /// 사각형 선택인가(문턱을 넘었는가). 아니면 놓았을 때 누르기다.
    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    /// 끄는 점의 목록 내용 기준 자리. 보이는 영역 밖이면 보이는 영역 끝으로 자른다(그 너머는 넘겨서 보인다).
    fn end(&self, list: &List) -> (f32, f32) {
        (
            (self.current.x - list.left).clamp(0.0, list.width.max(0.0)),
            (self.current.y - list.top).clamp(0.0, list.height.max(0.0)) + list.scroll,
        )
    }

    /// 그릴 사각형(목록 내용 기준).
    pub fn rect(&self, list: &List) -> Rect {
        let (x0, y0) = self.start;
        let (x1, y1) = self.end(list);
        Rect {
            x: x0.min(x1),
            y: y0.min(y1),
            width: (x1 - x0).abs(),
            height: (y1 - y0).abs(),
        }
    }

    /// 사각형이 세로로 걸친 행: (시작한 점 쪽 행, 끄는 점 쪽 행). 걸친 행이 없으면(목록 아래 빈 곳만) `None`이다.
    /// 행의 어느 칸에 걸쳐도 그 행이다.
    pub fn rows(&self, list: &List) -> Option<(usize, usize)> {
        if list.rows == 0 || list.row_height <= 0.0 {
            return None;
        }
        let rect = self.rect(list);
        let top = rect.y;
        let bottom = rect.y + rect.height;
        let first = (top / list.row_height).floor().max(0.0) as usize;
        if first >= list.rows {
            return None;
        }
        let last = if bottom > top {
            ((bottom / list.row_height).ceil() as usize).saturating_sub(1)
        } else {
            first
        }
        .clamp(first, list.rows - 1);
        let downward = self.end(list).1 >= self.start.1;
        Some(if downward {
            (first, last)
        } else {
            (last, first)
        })
    }

    /// 자동으로 넘길 거리(아래로 양수). 끄는 점이 목록 위·아래 끝 밖이면 멀수록 빠르게 넘기고, 더 넘길 수 없거나
    /// 끝 안이면 0이다. 앱은 사각형 선택 중 정한 간격마다 이것만큼 넘긴다.
    pub fn scroll_step(&self, list: &List) -> f32 {
        if !self.dragging {
            return 0.0;
        }
        let y = self.current.y - list.top;
        let step = if y < 0.0 {
            -speed(-y, list.row_height)
        } else if y > list.height {
            speed(y - list.height, list.row_height)
        } else {
            0.0
        };
        (list.scroll + step).clamp(0.0, list.max_scroll()) - list.scroll
    }
}

/// 끝 밖으로 `distance`만큼 나간 점에서 한 번에 넘길 거리: 거리의 절반을 행 높이의 1/4 ~ 4배로 자른다.
fn speed(distance: f32, row_height: f32) -> f32 {
    let row_height = row_height.max(1.0);
    (distance / 2.0).clamp(row_height / 4.0, row_height * 4.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: List = List {
        left: 100.0,
        top: 50.0,
        width: 400.0,
        height: 240.0,
        scroll: 0.0,
        row_height: 24.0,
        rows: 100,
    };

    #[test]
    fn 끝까지_넘기면_빈_곳_한_행이_보인다() {
        // 100행 × 24 = 2400, 빈 곳 한 행을 더해 2424, 보이는 높이 240.
        assert_eq!(LIST.max_scroll(), 2424.0 - 240.0);
    }

    fn at(x: f32, y: f32) -> Point {
        Point { x, y }
    }

    fn pressed(x: f32, y: f32, list: &List) -> Marquee<()> {
        Marquee::new(0, (), None, (false, false), at(x, y), list)
    }

    #[test]
    fn 문턱을_넘기_전에는_누르기다() {
        let mut marquee = pressed(300.0, 60.0, &LIST);
        assert!(!marquee.moved(at(304.0, 63.0)));
        assert!(!marquee.is_dragging());
        assert!(marquee.moved(at(300.0, 70.0)));
        // 한 번 넘으면 누른 자리로 돌아와도 끌기다.
        assert!(marquee.moved(at(300.0, 60.0)));
    }

    #[test]
    fn 사각형과_걸친_행() {
        let mut marquee = pressed(300.0, 60.0, &LIST);
        marquee.moved(at(250.0, 130.0));
        assert_eq!(
            marquee.rect(&LIST),
            Rect {
                x: 150.0,
                y: 10.0,
                width: 50.0,
                height: 70.0
            }
        );
        // 내용 y 10..80: 0·1·2·3행.
        assert_eq!(marquee.rows(&LIST), Some((0, 3)));
        // 위로 끌면 끄는 점 쪽이 앞이다.
        let mut marquee = pressed(300.0, 130.0, &LIST);
        marquee.moved(at(300.0, 60.0));
        assert_eq!(marquee.rows(&LIST), Some((3, 0)));
    }

    #[test]
    fn 행_경계에_딱_닿으면_다음_행은_걸치지_않는다() {
        // 내용 y 24..48은 1행 하나다.
        let mut marquee = pressed(300.0, 74.0, &LIST);
        marquee.moved(at(300.0, 98.0));
        assert_eq!(marquee.rows(&LIST), Some((1, 1)));
    }

    #[test]
    fn 목록_아래_빈_곳만_감싸면_걸친_행이_없다() {
        let list = List { rows: 3, ..LIST };
        let mut marquee = pressed(300.0, 200.0, &list);
        marquee.moved(at(350.0, 250.0));
        assert_eq!(marquee.rows(&list), None);
        // 위로 끌어 마지막 행에 닿으면 그 행부터다.
        marquee.moved(at(350.0, 115.0));
        assert_eq!(marquee.rows(&list), Some((2, 2)));
    }

    #[test]
    fn 시작점은_넘긴_목록에_붙어_있다() {
        let list = List {
            scroll: 240.0,
            ..LIST
        };
        // 보이는 첫 행이 10행이다.
        let mut marquee = pressed(300.0, 60.0, &list);
        marquee.moved(at(300.0, 90.0));
        assert_eq!(marquee.rows(&list), Some((10, 11)));
        // 더 넘겨도 시작한 10행에서 사각형이 이어진다.
        let scrolled = List {
            scroll: 480.0,
            ..LIST
        };
        assert_eq!(marquee.rows(&scrolled), Some((10, 21)));
    }

    #[test]
    fn 끝_밖의_점은_보이는_영역_끝으로_자른다() {
        let mut marquee = pressed(300.0, 60.0, &LIST);
        marquee.moved(at(900.0, 1000.0));
        let rect = marquee.rect(&LIST);
        assert_eq!(rect.x + rect.width, 400.0);
        assert_eq!(rect.y + rect.height, 240.0);
        assert_eq!(marquee.rows(&LIST), Some((0, 9)));
    }

    #[test]
    fn 끝_밖으로_끌면_멀수록_빠르게_넘긴다() {
        let list = List {
            scroll: 480.0,
            ..LIST
        };
        let mut marquee = pressed(300.0, 100.0, &list);
        marquee.moved(at(300.0, 200.0));
        assert_eq!(marquee.scroll_step(&list), 0.0);
        marquee.moved(at(300.0, 295.0));
        let near = marquee.scroll_step(&list);
        marquee.moved(at(300.0, 400.0));
        let far = marquee.scroll_step(&list);
        assert!(near > 0.0 && far > near, "{near} {far}");
        marquee.moved(at(300.0, 0.0));
        assert!(marquee.scroll_step(&list) < 0.0);
    }

    #[test]
    fn 더_넘길_수_없으면_넘기지_않는다() {
        let mut marquee = pressed(300.0, 100.0, &LIST);
        marquee.moved(at(300.0, 0.0));
        assert_eq!(marquee.scroll_step(&LIST), 0.0);
        let bottom = List {
            scroll: LIST.max_scroll(),
            ..LIST
        };
        marquee.moved(at(300.0, 1000.0));
        assert_eq!(marquee.scroll_step(&bottom), 0.0);
        // 끝 가까이에서는 남은 만큼만 넘긴다.
        let near_bottom = List {
            scroll: LIST.max_scroll() - 2.0,
            ..LIST
        };
        assert_eq!(marquee.scroll_step(&near_bottom), 2.0);
    }

    #[test]
    fn 누르기만_하면_넘기지_않는다() {
        let mut marquee = pressed(300.0, 52.0, &LIST);
        marquee.moved(at(300.0, 49.0));
        assert!(!marquee.is_dragging());
        assert_eq!(
            marquee.scroll_step(&List {
                scroll: 100.0,
                ..LIST
            }),
            0.0
        );
    }
}
