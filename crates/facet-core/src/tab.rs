//! 탭: 패널 안에서 위치 하나를 보여 주는 단위.
//!
//! 탭은 위치, 뒤로·앞으로 기록, 정렬, 목록을 갖는다. 이동은 두 단계다.
//!
//! 1. [`Tab::navigate`]가 갈 곳을 정하고 읽기 요청([`ReadRequest`])을 돌려준다. 이때 탭의 위치와
//!    기록은 아직 바뀌지 않는다.
//! 2. 요청을 (보통 일꾼 스레드에서) [`ReadRequest::run`]으로 읽고, 그 결과를 [`Tab::finish`]에 넘긴다.
//!    읽기가 성공했을 때만 이동을 확정한다. 실패하면 탭은 원래 위치에 남고 오류를 갖는다.
//!    실패한 위치는 기록에 남지 않는다.
//!
//! 앱은 읽을 때 위치의 변경 감시도 함께 건다([`ReadRequest::run_watched`]). 감시는 탭이 갖고, 다른 위치로
//! 옮기면 버려져 풀린다. 묶은 변경([`ChangeBatch`])은 [`Tab::apply_changes`]가 바뀐 항목만 목록에 반영한다.
//! 알림을 잃었거나 위치가 사라졌으면 [`Tab::reload`]로 다시 읽고, 위치가 사라졌으면 가장 가까운 남은 상위
//! 위치(없으면 사용자 홈 폴더)로 옮긴다([`Tab::relocation`]).
//!
//! 탭은 보이거나 숨는다. 패널에서 활성이 아닌 탭은 숨어서 감시를 놓는다([`Tab::hide`]). 목록은 그대로 갖고
//! 있다가, 다시 보이면([`Tab::show`]) 그 목록을 먼저 보이며 다시 읽고 감시를 다시 건다. 숨은 동안 끝난 읽기의
//! 감시는 버린다.
//!
//! 요청마다 번호를 붙인다. 탭이 기다리는 것은 마지막 요청 하나뿐이고, 그보다 앞선 요청의 결과는
//! 늦게 오면 버린다([`ReadOutcome::Stale`]).
//!
//! 이 모듈은 스레드를 만들지 않는다. 일꾼 스레드로 읽고 결과를 탭에 넘기는 일은
//! [`crate::window::App`]이 한다. 그래서 탭만 따로 화면 없이, 결과가 오는 순서를 정해 가며 시험할 수 있다.

use std::borrow::Cow;
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::details::{DetailsList, Sort};
use crate::selection::{CursorMove, Pick, StatusSummary, TypeAhead, ViewState, starts_with_folded};
use crate::watch::{self, ChangeBatch, ChangeSink, Watch, WatchError};
use crate::window::home_location;
use facet_win::OpenError;

use crate::content_search::{
    ContentEnd, ContentEvent, ContentFound, ContentRequest, ContentSearch,
};
use crate::details::Hit;
use crate::indexer_client::{Connection, Reply, RequestError, RequestId as IndexerRequestId};
use crate::instant_search::{
    self, DEMOTED_SEARCH_DELAY, Demotion, DemotionReason, FOLLOW_INTERVAL, Flight, Follow,
    InstantState, InstantStatus, PAGE_SIZE, Unavailable, found_from_hit,
};
use crate::launch::LaunchTarget;
use crate::live_search::{
    Found, SearchEnd, SearchEvent, SearchProgress, SearchRequest, SearchSummary,
};
use crate::location::SearchKind;
use crate::name_match::{NameMatcher, has_wildcard};
use crate::query_syntax::{self, ParseError, Parsed, Today};
use crate::search_results::{
    ContentStatus, PathChange, SearchFeed, SearchStatus, is_within, rebase,
};
use crate::{Candidate, Item, ListError, Location, ResolveError, provider_for};

/// 읽기가 이만큼 넘게 걸리면 탭이 「읽는 중」을 드러낸다([`Tab::is_reading`]).
pub const READING_DELAY: Duration = Duration::from_millis(100);

/// 탭에서 위치를 옮기는 이동.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Navigation {
    /// 주어진 위치로 간다(주소창, 경로 조각 등). 새 위치로 가는 것이라 앞으로 기록을 버린다.
    Go(Location),
    /// 지금 위치 안의 이름이 `name`인 폴더로 들어간다. 새 위치로 가는 것이다.
    Open(OsString),
    /// 뒤로 기록의 가장 가까운 위치로 간다.
    Back,
    /// 앞으로 기록의 가장 가까운 위치로 간다.
    Forward,
    /// 상위 위치로 간다. 새 위치로 가는 것이다. 드라이브 루트처럼 상위가 없으면 꺼진다.
    Up,
    /// 지금 위치를 다시 읽는다. 기록은 바뀌지 않는다.
    Refresh,
}

/// 읽기 요청의 번호. 한 탭 안에서 요청할 때마다 커진다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestId(u64);

/// 탭이 낸 읽기 요청 하나: 어느 위치를 어떤 정렬로 읽을지.
///
/// `Send`이므로 일꾼 스레드에 넘겨 [`ReadRequest::run`]한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadRequest {
    id: RequestId,
    location: Location,
    sort: Sort,
    /// 위치가 사라졌으면 가장 가까운 남은 상위(없으면 홈)를 읽는가([`Tab::reload`]).
    climb: bool,
}

impl ReadRequest {
    /// 요청 번호.
    pub fn id(&self) -> RequestId {
        self.id
    }

    /// 읽을 위치.
    pub fn location(&self) -> &Location {
        &self.location
    }

    /// 위치의 항목 목록을 공급자로 읽고 요청의 정렬로 정렬한다. 끝날 때까지 막으므로 UI 스레드에서
    /// 부르지 않는다. 변경 감시는 걸지 않는다(탭의 [`Tab::auto_refresh`]는 [`AutoRefresh::NotWatched`]).
    pub fn run(&self) -> ReadResult {
        self.read(None)
    }

    /// [`ReadRequest::run`]처럼 읽되, 읽기 전에 위치의 변경 감시를 건다. 묶은 변경은 묶기 스레드에서
    /// `sink`로 넘어온다. 그것을 탭의 [`Tab::apply_changes`]에 넘긴다.
    ///
    /// 감시를 걸 수 없어도 읽기는 한다. 그러면 탭은 「자동 새로 고침 안 됨」([`AutoRefresh::Off`])이다.
    pub fn run_watched(&self, sink: impl Fn(ChangeBatch) + Send + Sync + 'static) -> ReadResult {
        self.read(Some(Arc::new(sink)))
    }

    fn read(&self, sink: Option<ChangeSink>) -> ReadResult {
        // 감시를 먼저 건다: 읽는 사이의 변경도 놓치지 않는다.
        let attempt = |location: &Location| {
            // 검색 결과 위치는 탐색 시점의 것이라 감시하지 않는다.
            let watch = sink
                .as_ref()
                .filter(|_| !location.is_search_results())
                .map(|sink| watch::start(location, self.id, Arc::clone(sink)));
            let list = provider_for(location)
                .list(location)
                .map(|items| DetailsList::new(items, self.sort));
            (list, watch)
        };
        let result = |location: Location, (list, watch), relocation| ReadResult {
            id: self.id,
            location,
            list,
            watch,
            relocation,
        };
        let first = attempt(&self.location);
        // 권한이 없을 뿐이면 위치는 남아 있다.
        let vanished =
            matches!(&first.0, Err(e) if !matches!(e, ListError::PermissionDenied { .. }));
        if !(self.climb && vanished) {
            return result(self.location.clone(), first, None);
        }
        let provider = provider_for(&self.location);
        let mut candidate = provider.parent(&self.location);
        while let Some(location) = candidate {
            let read = attempt(&location);
            if read.0.is_ok() {
                let relocation = Relocation::ToParent {
                    vanished: self.location.clone(),
                };
                return result(location, read, Some(relocation));
            }
            candidate = provider.parent(&location);
        }
        let home = home_location();
        if home != self.location {
            let read = attempt(&home);
            if read.0.is_ok() {
                let relocation = Relocation::ToHome {
                    vanished: self.location.clone(),
                };
                return result(home, read, Some(relocation));
            }
        }
        result(self.location.clone(), first, None)
    }
}

/// 보던 위치가 사라져 다른 위치로 옮긴 까닭. [`Tab::relocation`]이 갖는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Relocation {
    /// 보던 위치 `vanished`가 사라져 가장 가까운 남은 상위 위치로 옮겼다.
    ToParent { vanished: Location },
    /// 보던 위치 `vanished`와 그 상위가 모두 사라져(드라이브가 빠짐 등) 사용자 홈 폴더로 옮겼다.
    ToHome { vanished: Location },
}

impl Relocation {
    /// 사라진 위치.
    pub fn vanished(&self) -> &Location {
        match self {
            Self::ToParent { vanished } | Self::ToHome { vanished } => vanished,
        }
    }

    fn with_vanished(self, vanished: Location) -> Self {
        match self {
            Self::ToParent { .. } => Self::ToParent { vanished },
            Self::ToHome { .. } => Self::ToHome { vanished },
        }
    }
}

/// 탭의 자동 새로 고침(변경 감시) 상태.
#[derive(Debug, Clone, Copy)]
pub enum AutoRefresh<'a> {
    /// 지금 위치를 감시하고 있다. 밖에서 바뀌면 목록이 스스로 갱신된다.
    On,
    /// 「자동 새로 고침 안 됨」: 지금 위치에 감시를 걸 수 없었다. 새로 고침(F5)은 된다.
    Off(&'a WatchError),
    /// 감시를 걸지 않고 읽었다([`ReadRequest::run`]), 또는 아직 읽지 않았다.
    NotWatched,
}

/// 묶은 변경을 탭에 넘긴 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeOutcome {
    /// 바뀐 항목을 목록에 반영했다.
    Applied,
    /// 알림을 잃었거나 위치가 사라졌다. [`Tab::reload`]로 다시 읽어야 한다.
    Reload,
    /// 탭이 지금 갖고 있지 않은 감시의 변경이라 버렸다(이미 다른 위치로 옮겼다).
    Stale,
}

/// 읽기 요청의 결과. [`ReadRequest::run`]만 만든다.
#[derive(Debug)]
pub struct ReadResult {
    id: RequestId,
    /// 실제로 읽은 위치. 사라진 위치에서 옮겼으면 요청한 위치와 다르다.
    location: Location,
    list: Result<DetailsList, ListError>,
    /// 건 감시. 감시를 걸지 않고 읽었으면 `None`이다.
    watch: Option<Result<Watch, WatchError>>,
    relocation: Option<Relocation>,
}

impl ReadResult {
    /// 어느 요청의 결과인가.
    pub fn id(&self) -> RequestId {
        self.id
    }
}

/// 자동 완성 요청의 번호. 한 탭 안에서 요청할 때마다 커진다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CompletionId(u64);

/// 주소창 글자의 자동 완성 요청. `Send`이므로 일꾼 스레드에 넘겨 [`CompletionRequest::run`]한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionRequest {
    id: CompletionId,
    location: Location,
    text: String,
}

impl CompletionRequest {
    /// 요청 번호.
    pub fn id(&self) -> CompletionId {
        self.id
    }

    /// 후보를 읽는다. 폴더를 읽느라 막히므로 UI 스레드에서 부르지 않는다.
    pub fn run(&self) -> CompletionResult {
        CompletionResult {
            id: self.id,
            candidates: provider_for(&self.location).complete(&self.text),
        }
    }
}

/// 자동 완성 요청의 결과. [`CompletionRequest::run`]만 만든다.
#[derive(Debug)]
pub struct CompletionResult {
    id: CompletionId,
    candidates: Vec<Candidate>,
}

impl CompletionResult {
    /// 어느 요청의 결과인가.
    pub fn id(&self) -> CompletionId {
        self.id
    }
}

/// 필터 요청의 번호. 한 탭 안에서 요청할 때마다 커지며, 앞선 요청의 결과를 버리는 데 쓴다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FilterId(u64);

/// 필터 거르기 요청([`Tab::request_filter`]). `Send`이므로 일꾼 스레드에 넘겨 [`FilterRequest::run`]한다.
///
/// 글자를 덧붙인 것이면 앞 결과 안의 이름만 담아(앞 결과가 부분 일치일 때) 앞 결과 안에서만 다시 거른다. 결과는
/// 처음부터 거른 것과 같다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterRequest {
    id: FilterId,
    text: String,
    /// 요청을 만들 때의 목록 항목 이름(목록 안 번호 순서). 목록이 바뀌기 전까지 탭이 한 벌을 나눠 쓴다.
    names: Arc<Vec<OsString>>,
    /// 거를 후보의 목록 안 번호. `None`이면 모든 항목이다.
    candidates: Option<Vec<usize>>,
    list_gen: u64,
}

/// 후보가 이만큼 넘으면 거르기를 여러 스레드로 나눈다(이름 하나는 마이크로초 단위라 적을 때는 나누는 비용이 더 크다).
const PARALLEL_FILTER_MIN: usize = 20_000;

impl FilterRequest {
    /// 요청 번호.
    pub fn id(&self) -> FilterId {
        self.id
    }

    /// 후보 이름을 이름 일치 규칙으로 거른다. 후보가 많으면 오래 걸리므로 UI 스레드에서 부르지 않는다. 후보가
    /// [`PARALLEL_FILTER_MIN`]개를 넘으면 사용할 수 있는 코어만큼 나눠 거른다(결과는 번호 오름차순 그대로).
    pub fn run(&self) -> FilterResult {
        let matcher = NameMatcher::new(&self.text);
        let all: Vec<usize>;
        let candidates: &[usize] = match &self.candidates {
            Some(candidates) => candidates,
            None => {
                all = (0..self.names.len()).collect();
                &all
            }
        };
        let filter_chunk = |chunk: &[usize]| -> Vec<usize> {
            chunk
                .iter()
                .copied()
                .filter(|&i| matcher.is_match(&self.names[i]))
                .collect()
        };
        let threads = std::thread::available_parallelism().map_or(1, usize::from);
        let matched = if candidates.len() < PARALLEL_FILTER_MIN || threads < 2 {
            filter_chunk(candidates)
        } else {
            let size = candidates.len().div_ceil(threads);
            std::thread::scope(|scope| {
                let handles: Vec<_> = candidates
                    .chunks(size)
                    .map(|chunk| scope.spawn(|| filter_chunk(chunk)))
                    .collect();
                handles
                    .into_iter()
                    .flat_map(|handle| handle.join().expect("거르기 스레드가 멈췄다"))
                    .collect()
            })
        };
        FilterResult {
            id: self.id,
            text: self.text.clone(),
            list_gen: self.list_gen,
            matched,
        }
    }
}

/// 필터 거르기의 결과. [`FilterRequest::run`]만 만든다.
#[derive(Debug)]
pub struct FilterResult {
    id: FilterId,
    text: String,
    list_gen: u64,
    /// 맞은 항목의 목록 안 번호(오름차순).
    matched: Vec<usize>,
}

impl FilterResult {
    /// 어느 요청의 결과인가.
    pub fn id(&self) -> FilterId {
        self.id
    }
}

/// 탭이 지금 적용한 필터: 글자와 맞은 이름들.
#[derive(Debug)]
struct Filter {
    text: String,
    matcher: NameMatcher,
    /// 필터에 맞는 항목의 목록 안 번호(숨김 항목 포함, 오름차순)와 그것이 맞는 `list_gen`. 목록이 바뀌어 번호가 어긋나면
    /// 버리고 이름으로 다시 맞춘다([`Tab::update_visible`]).
    matched: Option<(Vec<usize>, u64)>,
}

/// 검색 결과 위치의 상태: 결과를 넣는 피드와 진행.
struct Results {
    /// 결과를 흘려 넣는 것. 아직 붙이지 않았거나 끝났으면 `None`이다. 버리면 검색이 멈춘다.
    feed: Option<Box<dyn SearchFeed>>,
    /// 내용 검색이 결과를 흘려 넣는 것(내용 검색 결과 위치만). 버리면 검색이 멈춘다.
    content: Option<ContentSearch>,
    status: SearchStatus,
    /// 즉시 검색 결과 위치면 그 상태(보낸 요청·이어 받기 표지·전체 개수).
    instant: Option<InstantState>,
}

impl std::fmt::Debug for Results {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Results")
            .field("feed", &self.feed.is_some())
            .field("content", &self.content.is_some())
            .field("status", &self.status)
            .finish()
    }
}

/// 읽기 결과를 탭에 넘긴 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOutcome {
    /// 읽기가 성공해 이동(또는 새로 고침)을 확정했다.
    Committed,
    /// 읽기가 실패했다. 탭은 원래 위치에 남고 [`Tab::error`]에 까닭이 있다.
    Failed,
    /// 탭이 기다리는 요청의 결과가 아니라 버렸다(사용자가 이미 다른 곳으로 갔다).
    Stale,
}

/// 확정되면 기록을 어떻게 바꾸는가.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// 새 위치: 지금 위치를 뒤로 기록에 넣고 앞으로 기록을 버린다.
    New,
    Back,
    Forward,
    Refresh,
    /// 자동 다시 읽기: 위치가 사라졌으면 남은 상위로 옮긴다. 기록은 바뀌지 않는다.
    Reload,
}

/// 기다리는 읽기 요청.
#[derive(Debug, Clone)]
struct Pending {
    id: RequestId,
    location: Location,
    kind: Kind,
    started: Instant,
    /// 위로 갈 때 방금 있던 폴더의 이름. 확정되면 그 항목을 선택하고 커서를 둔다.
    reselect: Option<OsString>,
}

/// 닫은 탭에서 되살릴 것: 위치, 뒤로·앞으로 기록, 정렬, 닫을 때의 선택·커서·스크롤. 목록은 담지 않는다
/// (되살리면 위치를 다시 읽는다). [`Tab::close`]가 만들고 [`Tab::reopen`]이 탭으로 되돌린다.
#[derive(Debug)]
pub struct ClosedTab {
    location: Location,
    back: Vec<Location>,
    back_views: Vec<ViewState>,
    forward: Vec<Location>,
    forward_views: Vec<ViewState>,
    sort: Sort,
    view: ViewState,
}

impl ClosedTab {
    /// 닫을 때의 위치.
    pub fn location(&self) -> &Location {
        &self.location
    }

    /// 뒤로 기록. 가장 가까운 위치가 끝이다.
    pub fn back_history(&self) -> &[Location] {
        &self.back
    }

    /// 앞으로 기록. 가장 가까운 위치가 끝이다.
    pub fn forward_history(&self) -> &[Location] {
        &self.forward
    }

    /// 정렬.
    pub fn sort(&self) -> Sort {
        self.sort
    }
}

/// 패널 안에서 위치 하나를 보여 주는 단위.
///
/// 목록은 탭의 정렬로 정렬해 둔다. 숨김 항목을 감출 때([`Tab::show_hidden`]가 `false`) 숨김 속성이
/// 있는 항목은 [`Tab::len`]·[`Tab::get`]·[`Tab::items`]에 나오지 않는다. 감춘 항목도 탭은 갖고 있어서
/// 보이기로 바꿀 때 다시 읽지 않는다.
#[derive(Debug)]
pub struct Tab {
    location: Location,
    /// 뒤로 기록. 가장 가까운 것이 끝이다.
    back: Vec<Location>,
    /// `back`과 짝을 이루는, 그 위치를 떠날 때의 보기 상태(선택·커서·스크롤).
    back_views: Vec<ViewState>,
    /// 앞으로 기록. 가장 가까운 것이 끝이다.
    forward: Vec<Location>,
    /// `forward`와 짝을 이루는 보기 상태.
    forward_views: Vec<ViewState>,
    sort: Sort,
    show_hidden: bool,
    /// 숨김 항목까지 모두 담은 목록.
    list: DetailsList,
    /// 숨김 항목을 감출 때 보이는 항목의 `list` 안 번호. `None`이면 모두 보인다.
    visible: Option<Vec<usize>>,
    pending: Option<Pending>,
    next_id: u64,
    error: Option<ListError>,
    /// 마지막 주소창 글자 해석이 실패한 까닭.
    address_error: Option<ResolveError>,
    /// 마지막 자동 완성 요청 번호. 이보다 앞선 요청의 결과는 버린다.
    next_completion: u64,
    /// 선택한 항목의 이름. 보이는 항목만 든다.
    selection: HashSet<OsString>,
    /// 커서 항목의 이름과 보이는 항목 안 번호(정렬·새로 고침 뒤 `reindex`로 맞춘다).
    cursor: Option<(OsString, usize)>,
    /// Shift 범위의 기준 항목 이름.
    anchor: Option<OsString>,
    /// 사각형 선택 중이면 사각형과 상관없이 남는 선택(Ctrl로 더하면 끌기 전 선택, 아니면 비어 있음).
    marquee_base: Option<HashSet<OsString>>,
    /// 화면이 맨 위에 보이는 행 번호. 화면이 알려 주는 값을 그대로 보관한다.
    scroll: usize,
    type_ahead: TypeAhead,
    /// 지금 위치의 감시와 그것을 건 읽기 요청 번호. 버리면 풀린다.
    watching: Option<(RequestId, Watch)>,
    /// 지금 위치에 감시를 걸지 못한 까닭.
    watch_error: Option<WatchError>,
    /// 보던 위치가 사라져 옮긴 까닭. 다음 이동을 시작하면 지운다.
    relocation: Option<Relocation>,
    /// 마지막으로 파일을 기본 프로그램으로 열지 못한 까닭.
    open_error: Option<OpenError>,
    /// 보이는 탭인가. 숨은 탭은 감시를 갖지 않는다.
    shown: bool,
    /// 지금 적용한 필터. 위치를 바꾸면 풀린다.
    filter: Option<Filter>,
    /// 요청이 쓰는 항목 이름 한 벌과 그것이 맞는 `list_gen`. 목록이 바뀌면 다음 요청 때 다시 만든다.
    names: Option<(u64, Arc<Vec<OsString>>)>,
    /// 다음 필터 요청 번호. 이보다 앞선 요청의 결과는 버린다.
    next_filter: u64,
    /// 목록의 항목이나 순서가 바뀔 때(읽기 결과·변경·정렬)마다 커진다. 거르는 동안 목록이 바뀌었는지 안다.
    list_gen: u64,
    /// 검색 결과 위치면 그 상태(피드·진행). 다른 위치로 가면 버려 검색이 멈춘다.
    results: Option<Results>,
    /// 즉시 검색의 범위: 「현재 폴더 아래만」인가(아니면 모든 볼륨). 위치를 옮겨도 탭에 남는다.
    instant_under: bool,
    /// 마지막으로 넘긴 즉시 검색 글자가 틀린 까닭(결과·위치는 그대로다). 맞는 글자·검색 끝내기·다른 곳으로 이동이 지운다.
    instant_error: Option<ParseError>,
    /// 마지막으로 결과에서 열려던 항목이 디스크에 없었으면 그 경로(그 항목은 결과에서 뺐다). 다음 열기가 지운다.
    missing_item: Option<PathBuf>,
}

impl Tab {
    /// `location`을 보여 줄 탭. 목록은 비어 있고 기록도 없다.
    ///
    /// 목록을 채우려면 [`Navigation::Refresh`]로 읽는다.
    pub fn new(location: Location, sort: Sort, show_hidden: bool) -> Self {
        Tab {
            location,
            back: Vec::new(),
            back_views: Vec::new(),
            forward: Vec::new(),
            forward_views: Vec::new(),
            sort,
            show_hidden,
            list: DetailsList::new(Vec::new(), sort),
            visible: None,
            pending: None,
            next_id: 0,
            error: None,
            address_error: None,
            next_completion: 0,
            selection: HashSet::new(),
            cursor: None,
            anchor: None,
            marquee_base: None,
            scroll: 0,
            type_ahead: TypeAhead::default(),
            watching: None,
            watch_error: None,
            relocation: None,
            open_error: None,
            shown: true,
            filter: None,
            names: None,
            next_filter: 0,
            list_gen: 0,
            results: None,
            instant_under: false,
            instant_error: None,
            missing_item: None,
        }
    }

    /// 탭을 닫아 되살릴 것만 남긴다([`ClosedTab`]): 위치, 뒤로·앞으로 기록(과 그 보기 상태), 정렬, 지금 보기
    /// 상태(선택·커서·스크롤). 목록과 감시는 버린다.
    pub fn close(mut self) -> ClosedTab {
        let view = self.take_view();
        ClosedTab {
            location: self.location,
            back: self.back,
            back_views: self.back_views,
            forward: self.forward,
            forward_views: self.forward_views,
            sort: self.sort,
            view,
        }
    }

    /// 닫은 탭을 되살린다. 위치·기록·정렬·보기 상태는 닫을 때 그대로이고 목록은 비어 있다. 보이는 탭이며, 목록을
    /// 채우려면 [`Navigation::Refresh`]로 다시 읽는다(선택·커서는 이름으로 남는다).
    pub fn reopen(closed: ClosedTab, show_hidden: bool) -> Tab {
        let ClosedTab {
            location,
            back,
            back_views,
            forward,
            forward_views,
            sort,
            view,
        } = closed;
        let mut tab = Tab::new(location, sort, show_hidden);
        tab.back = back;
        tab.back_views = back_views;
        tab.forward = forward;
        tab.forward_views = forward_views;
        tab.selection = view.selection;
        tab.cursor = view.cursor.map(|name| (name, 0));
        tab.anchor = view.anchor;
        tab.scroll = view.scroll;
        tab
    }

    /// 실행 인자 하나([`LaunchTarget`])를 보일 탭. 목록은 비어 있고 기록도 없다. 파일 경로였으면 그 파일을 고른
    /// 채로(선택·커서·범위 기준이 그 이름) 시작해, 읽으면 그 항목이 골라져 있다. 해석하지 못한 인자면
    /// [`Tab::address_error`]에 까닭을 갖는다(이때는 읽지 않는다).
    pub fn launched(target: LaunchTarget, sort: Sort, show_hidden: bool) -> Tab {
        let LaunchTarget {
            location,
            select,
            error,
        } = target;
        let mut tab = Tab::new(location, sort, show_hidden);
        tab.address_error = error;
        if let Some(name) = select {
            tab.selection.insert(name.clone());
            tab.cursor = Some((name.clone(), 0));
            tab.anchor = Some(name);
        }
        tab
    }

    /// 탭 제목: 위치의 폴더 이름. 드라이브 루트는 드라이브 이름(`C:`)이다. 화면 문장이 아니라 이름이다.
    pub fn title(&self) -> String {
        provider_for(&self.location)
            .segments(&self.location)
            .pop()
            .map(|segment| segment.label)
            .unwrap_or_else(|| self.location.to_string())
    }

    /// 보이는 탭인가. 새로 만든 탭은 보인다.
    pub fn is_shown(&self) -> bool {
        self.shown
    }

    /// 탭을 숨긴다: 감시를 놓는다(버려서 푼다). 목록·선택·커서·스크롤·기록·정렬은 그대로 갖는다. 숨은 동안 끝난
    /// 읽기도 감시를 남기지 않는다.
    pub fn hide(&mut self) {
        self.shown = false;
        self.watching = None;
        self.watch_error = None;
    }

    /// 숨은 탭을 다시 보인다. 갖고 있던 목록은 그대로 두고, 지금 위치를 다시 읽을 요청을 돌려준다. 그 요청을
    /// 감시와 함께 읽어([`ReadRequest::run_watched`]) [`Tab::finish`]에 넘기면 새 목록으로 바뀌고 감시가 다시
    /// 걸린다. 선택·커서·스크롤은 이름으로 남는다. 숨은 동안 위치가 사라졌으면 [`Tab::reload`]처럼 가장 가까운
    /// 남은 상위로 옮긴다.
    ///
    /// 이미 보이면 아무것도 하지 않고 `None`이다. 기다리는 읽기가 있어도 `None`이다(그 읽기가 감시를 건다).
    pub fn show(&mut self) -> Option<ReadRequest> {
        if self.shown {
            return None;
        }
        self.shown = true;
        // 검색 결과 위치는 다시 읽을 것이 없다: 결과는 숨은 동안에도 그대로이고 검색도 이어진다.
        if self.location.is_search_results() && self.results.is_some() {
            return None;
        }
        self.reload()
    }

    /// 지금 위치. 이동은 읽기가 성공해야 바뀐다.
    pub fn location(&self) -> &Location {
        &self.location
    }

    /// 뒤로 기록. 가장 가까운 위치(뒤로 가면 갈 곳)가 끝이다.
    pub fn back_history(&self) -> &[Location] {
        &self.back
    }

    /// 앞으로 기록. 가장 가까운 위치(앞으로 가면 갈 곳)가 끝이다.
    pub fn forward_history(&self) -> &[Location] {
        &self.forward
    }

    /// 뒤로 갈 수 있는가.
    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    /// 앞으로 갈 수 있는가.
    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// 위로 갈 수 있는가. 드라이브 루트처럼 상위 위치가 없으면 꺼진다.
    pub fn can_go_up(&self) -> bool {
        provider_for(&self.location)
            .parent(&self.location)
            .is_some()
    }

    /// 지금 정렬.
    pub fn sort(&self) -> Sort {
        self.sort
    }

    /// 숨김 항목을 보이는가.
    pub fn show_hidden(&self) -> bool {
        self.show_hidden
    }

    /// 마지막 이동(또는 새로 고침)이 실패한 까닭. 다음 이동을 시작하면 지운다.
    pub fn error(&self) -> Option<&ListError> {
        self.error.as_ref()
    }

    /// 마지막 [`Tab::go_to_text`]가 글자를 해석하지 못한 까닭. 다음 이동을 시작하면 지운다.
    pub fn address_error(&self) -> Option<&ResolveError> {
        self.address_error.as_ref()
    }

    /// 보던 위치가 사라져 다른 위치로 옮겼으면 그 까닭. 다음 이동을 시작하면 지운다.
    pub fn relocation(&self) -> Option<&Relocation> {
        self.relocation.as_ref()
    }

    /// 마지막으로 파일을 기본 프로그램으로 열지 못한 까닭(연결된 프로그램 없음 등). 다음 이동이나 다음 열기를
    /// 시작하면 지운다. 여는 일은 [`crate::window::App::execute`]가 한다.
    pub fn open_error(&self) -> Option<&OpenError> {
        self.open_error.as_ref()
    }

    pub(crate) fn set_open_error(&mut self, error: Option<OpenError>) {
        self.open_error = error;
    }

    /// 자동 새로 고침 상태. 감시를 걸 수 없는 위치면 [`AutoRefresh::Off`]다.
    pub fn auto_refresh(&self) -> AutoRefresh<'_> {
        match (&self.watching, &self.watch_error) {
            (Some(_), _) => AutoRefresh::On,
            (None, Some(error)) => AutoRefresh::Off(error),
            (None, None) => AutoRefresh::NotWatched,
        }
    }

    /// 감시하고 있는 위치. 감시가 없으면 `None`이다.
    pub fn watched_location(&self) -> Option<&Location> {
        self.watching.as_ref().map(|_| &self.location)
    }

    /// 읽기를 기다리는 위치. 없으면 `None`이다.
    pub fn pending_location(&self) -> Option<&Location> {
        self.pending.as_ref().map(|p| &p.location)
    }

    /// 「읽는 중」을 드러낼 시각: 기다리는 읽기를 시작하고 [`READING_DELAY`]가 지난 때. 기다리는 읽기가
    /// 없으면 `None`이다. 화면은 이 시각에 다시 그리도록 타이머를 건다.
    pub fn reading_shown_at(&self) -> Option<Instant> {
        self.pending.as_ref().map(|p| p.started + READING_DELAY)
    }

    /// `now`에 「읽는 중」을 드러내는가: 읽기를 기다린 지 [`READING_DELAY`]가 지났는가.
    pub fn is_reading(&self, now: Instant) -> bool {
        self.reading_shown_at().is_some_and(|at| now >= at)
    }

    /// 보이는 항목 수.
    pub fn len(&self) -> usize {
        match &self.visible {
            Some(visible) => visible.len(),
            None => self.list.len(),
        }
    }

    /// 보이는 항목이 없는가.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 보이는 항목 중 정렬된 순서로 `index`번째.
    pub fn get(&self, index: usize) -> Option<&Item> {
        match &self.visible {
            Some(visible) => self.list.get(*visible.get(index)?),
            None => self.list.get(index),
        }
    }

    /// 보이는 항목을 정렬된 순서로.
    pub fn items(&self) -> impl Iterator<Item = &Item> + '_ {
        (0..self.len()).filter_map(|i| self.get(i))
    }

    /// 이동을 시작한다. 읽어야 할 요청을 돌려주고, 이동할 수 없으면(기록이 없음, 상위가 없음) `None`이다.
    ///
    /// 위치와 기록은 [`Tab::finish`]가 성공한 결과를 받을 때 바뀐다. 앞서 기다리던 요청이 있으면 이 요청이
    /// 그것을 대신하고, 앞선 요청의 결과는 버린다. 시작하면 지난 오류를 지운다.
    pub fn navigate(&mut self, navigation: Navigation) -> Option<ReadRequest> {
        let provider = provider_for(&self.location);
        let mut reselect = None;
        let (location, kind) = match navigation {
            Navigation::Go(location) => (location, Kind::New),
            Navigation::Open(name) => (provider.child(&self.location, &name), Kind::New),
            Navigation::Back => (self.back.last()?.clone(), Kind::Back),
            Navigation::Forward => (self.forward.last()?.clone(), Kind::Forward),
            Navigation::Up => {
                let parent = provider.parent(&self.location)?;
                reselect = self
                    .location
                    .file_system_path()
                    .and_then(|path| path.file_name())
                    .map(OsStr::to_os_string);
                (parent, Kind::New)
            }
            Navigation::Refresh => (self.location.clone(), Kind::Refresh),
        };
        self.address_error = None;
        self.relocation = None;
        self.open_error = None;
        Some(self.request(location, kind, reselect))
    }

    fn request(
        &mut self,
        location: Location,
        kind: Kind,
        reselect: Option<OsString>,
    ) -> ReadRequest {
        let id = RequestId(self.next_id);
        self.next_id += 1;
        self.error = None;
        self.pending = Some(Pending {
            id,
            location: location.clone(),
            kind,
            started: Instant::now(),
            reselect,
        });
        ReadRequest {
            id,
            location,
            sort: self.sort,
            climb: kind == Kind::Reload,
        }
    }

    /// 자동 다시 읽기: 지금 위치를 다시 읽되, 위치가 사라졌으면 가장 가까운 남은 상위 위치로, 상위가 하나도
    /// 없으면 사용자 홈 폴더로 옮긴다. 옮기면 [`Tab::relocation`]에 까닭을 두고, 사라진 위치는 기록에 넣지 않는다.
    ///
    /// [`Tab::apply_changes`]가 [`ChangeOutcome::Reload`]를 돌려줄 때 부른다. 이미 기다리는 읽기가 있으면
    /// 그것을 가로채지 않고 `None`을 돌려준다(그 읽기가 목록을 새로 맞춘다).
    pub fn reload(&mut self) -> Option<ReadRequest> {
        if self.pending.is_some() {
            return None;
        }
        let location = self.location.clone();
        Some(self.request(location, Kind::Reload, None))
    }

    /// 묶은 변경을 받는다. 지금 위치의 감시에서 온 것이면 바뀐 항목만 목록에 반영해 다시 정렬한다. 선택과
    /// 스크롤 위치는 그대로이고, 사라진 항목만 선택에서 빠진다.
    ///
    /// 알림을 잃었거나 위치가 사라졌으면 반영하지 않고 [`ChangeOutcome::Reload`]를 돌려준다.
    pub fn apply_changes(&mut self, batch: ChangeBatch) -> ChangeOutcome {
        if self
            .watching
            .as_ref()
            .is_none_or(|(id, _)| *id != batch.watch)
        {
            return ChangeOutcome::Stale;
        }
        if batch.reload {
            return ChangeOutcome::Reload;
        }
        if !batch.changes.is_empty() {
            self.list_gen += 1;
            self.list.apply_changes(batch.changes);
            self.update_visible();
            self.prune_selection();
            self.reindex();
        }
        ChangeOutcome::Applied
    }

    /// 숨김 항목까지 모든 항목의 이름(같은 이름 확인용).
    pub(crate) fn all_names(&self) -> impl Iterator<Item = &OsStr> + '_ {
        self.list.iter().map(|item| item.name.as_os_str())
    }

    /// 끝난 이름바꾸기·새 폴더([`crate::edit::Edited`])를 목록에 곧바로 반영한다. 변경 감시의 묶음을 기다리지 않는
    /// 것은 이름 확정부터 새 이름이 보이기까지 한 프레임(16ms) 안이어야 해서다(감시는 [`crate::watch::BATCH_INTERVAL`]
    /// 100ms 단위로 묶는다). 뒤에 감시가 같은 변경을 알려도 이름으로 맞추므로 목록·선택이 그대로다.
    ///
    /// - 이름바꾸기: 원래 이름 항목을 새 이름 항목으로 바꿔 다시 정렬하고, 선택·커서·범위 기준이 갖던 원래 이름을 새
    ///   이름으로 옮긴다. 바꾼 항목은 선택·커서다.
    /// - 새 폴더: 만든 폴더를 넣고 그것만 선택해 커서를 둔다.
    ///
    /// 이 탭이 그 폴더를 보고 있지 않으면 아무것도 하지 않고 `false`다.
    pub(crate) fn apply_edit(&mut self, edited: &crate::edit::Edited) -> bool {
        let Some(name) = edited.name() else {
            return false;
        };
        let here = self.location.file_system_path();
        if here.is_none() || here != edited.path.parent() {
            return false;
        }
        let name = name.to_os_string();
        let mut changes = Vec::with_capacity(2);
        let original = edited.original_name().map(OsStr::to_os_string);
        if let Some(original) = &original {
            changes.push((original.clone(), None));
        }
        changes.push((name.clone(), edited.item.clone()));
        self.list_gen += 1;
        self.list.apply_changes(changes);
        self.update_visible();
        if let Some(original) = &original {
            if self.selection.remove(original) {
                self.selection.insert(name.clone());
            }
            if self.anchor.as_ref() == Some(original) {
                self.anchor = Some(name.clone());
            }
        }
        match self.position_of(&name) {
            Some(index) if edited.kind == crate::edit::EditKind::NewFolder => {
                self.select_only(index);
            }
            Some(index) => {
                self.selection.insert(name.clone());
                self.set_cursor(index);
            }
            None => {}
        }
        self.prune_selection();
        self.reindex();
        true
    }

    /// 주소창에 친 글자로 간다: 해석 → 이동 시작. 파일 경로를 쳤으면 그 폴더로 가고 도착하면 그 파일을 선택한다.
    ///
    /// 해석에 실패하면 `None`을 돌려주고 [`Tab::address_error`]에 까닭을 둔다. 위치·기록·기다리는 읽기는
    /// 그대로다. 성공하면 [`Tab::navigate`]처럼 읽기 요청을 돌려주고 [`Tab::finish`]로 확정한다.
    pub fn go_to_text(&mut self, text: &str) -> Option<ReadRequest> {
        let resolved = match provider_for(&self.location).resolve(text) {
            Ok(resolved) => resolved,
            Err(error) => {
                self.address_error = Some(error);
                return None;
            }
        };
        let request = self.navigate(Navigation::Go(resolved.location))?;
        if let Some(pending) = &mut self.pending {
            pending.reselect = resolved.select;
        }
        Some(request)
    }

    /// 고쳐 쓰는 글자의 자동 완성 후보를 읽을 요청. 앞선 요청의 결과는 [`Tab::finish_completion`]이 버린다.
    pub fn complete(&mut self, text: &str) -> CompletionRequest {
        let id = CompletionId(self.next_completion);
        self.next_completion += 1;
        CompletionRequest {
            id,
            location: self.location.clone(),
            text: text.to_string(),
        }
    }

    /// 자동 완성 결과를 받는다. 마지막 요청의 결과면 후보를, 늦게 온 앞선 요청의 결과면 `None`을 돌려준다.
    pub fn finish_completion(&self, result: CompletionResult) -> Option<Vec<Candidate>> {
        (result.id.0 + 1 == self.next_completion).then_some(result.candidates)
    }

    /// 읽기 결과를 받는다. 기다리는 요청의 결과일 때만 반영한다.
    ///
    /// - 성공: 위치를 옮기고 기록을 고친다. 새 위치로 가면 지금 위치를 뒤로 기록에 넣고 앞으로 기록을 버린다
    ///   (지금 위치와 같은 곳이면 기록을 건드리지 않는다). 뒤로·앞으로 가면 지금 위치를 반대쪽 기록에 넣는다.
    /// - 실패: 위치·목록·기록을 그대로 두고 오류를 갖는다.
    pub fn finish(&mut self, result: ReadResult) -> ReadOutcome {
        let Some(pending) = self.pending.take_if(|p| p.id == result.id) else {
            return ReadOutcome::Stale;
        };
        let ReadResult {
            id,
            location: read_location,
            list,
            watch,
            relocation,
        } = result;
        let mut list = match list {
            Ok(list) => list,
            Err(error) => {
                self.error = Some(error);
                return ReadOutcome::Failed;
            }
        };
        let mut restored = None;
        let reselect = pending.reselect.filter(|_| pending.kind == Kind::New);
        let moved = read_location != self.location;
        match pending.kind {
            Kind::New if pending.location != self.location => {
                let previous = std::mem::replace(&mut self.location, pending.location);
                self.back.push(previous);
                let view = self.take_view();
                self.back_views.push(view);
                self.forward.clear();
                self.forward_views.clear();
                restored = Some(ViewState::default());
            }
            Kind::Back => {
                self.back.pop();
                let previous = std::mem::replace(&mut self.location, pending.location);
                self.forward.push(previous);
                let view = self.take_view();
                self.forward_views.push(view);
                restored = self.back_views.pop();
            }
            Kind::Forward => {
                self.forward.pop();
                let previous = std::mem::replace(&mut self.location, pending.location);
                self.back.push(previous);
                let view = self.take_view();
                self.back_views.push(view);
                restored = self.forward_views.pop();
            }
            Kind::Reload if moved => {
                // 사라진 위치는 기록에 넣지 않고, 그 위치의 보기 상태도 버린다.
                self.location = read_location;
                self.take_view();
                restored = Some(ViewState::default());
                // 옮긴 곳도 이어서 사라지면 다시 옮긴다. 사용자가 보던 위치는 처음 사라진 곳이다.
                self.relocation = match (self.relocation.take(), relocation) {
                    (Some(first), Some(again)) => {
                        Some(again.with_vanished(first.vanished().clone()))
                    }
                    (_, relocation) => relocation,
                };
            }
            Kind::New | Kind::Refresh | Kind::Reload => {}
        }
        match watch {
            // 숨은 탭은 감시를 갖지 않는다: 숨은 동안 끝난 읽기의 감시는 버린다.
            Some(_) if !self.shown => {
                self.watching = None;
                self.watch_error = None;
            }
            Some(Ok(watch)) => {
                self.watching = Some((id, watch));
                self.watch_error = None;
            }
            Some(Err(error)) => {
                self.watching = None;
                self.watch_error = Some(error);
            }
            // 감시 없이 읽었다: 다른 위치로 옮겼으면 앞 위치의 감시를 푼다.
            None if moved => {
                self.watching = None;
                self.watch_error = None;
            }
            None => {}
        }
        // 읽는 사이에 정렬을 바꿨으면 지금 정렬로 맞춘다.
        if list.sort() != self.sort {
            list.set_sort(self.sort);
        }
        self.list = list;
        self.list_gen += 1;
        // 검색 결과 위치로 갔거나 다시 읽었으면 결과를 처음부터 받는다(앞 피드를 버려 앞 검색을 멈춘다). 다른 위치로
        // 갔으면 검색이 멈춘다.
        let is_instant = is_instant_location(&self.location);
        if !is_instant {
            self.instant_error = None;
        }
        self.results = self.location.is_search_results().then(|| Results {
            feed: None,
            content: None,
            instant: is_instant.then(InstantState::default),
            status: SearchStatus {
                show_hidden: self.show_hidden,
                ..SearchStatus::default()
            },
        });
        if restored.is_some() {
            // 다른 위치로 갔다: 필터가 풀린다.
            self.filter = None;
            self.next_filter += 1;
        }
        self.update_visible();
        if let Some(view) = restored {
            self.selection = view.selection;
            self.cursor = view.cursor.map(|name| (name, 0));
            self.anchor = view.anchor;
            self.scroll = view.scroll;
        }
        self.prune_selection();
        self.reindex();
        if let Some(name) = reselect
            && let Some(index) = self.position_of(&name)
        {
            self.select_only(index);
        }
        ReadOutcome::Committed
    }

    /// 정렬을 바꾼다. 가진 목록을 다시 정렬할 뿐 파일 시스템을 다시 읽지 않는다.
    pub fn set_sort(&mut self, sort: Sort) {
        if sort == self.sort {
            return;
        }
        self.sort = sort;
        self.list_gen += 1;
        self.list.set_sort(sort);
        // 즉시 검색: 서비스가 아는 열로 바꾸면 같은 질의를 그 정렬로 다시 보낸다(앱이 보낸다). 모르는 열은 받아 둔 것만 정렬한다.
        if let Some(wanted) = instant_search::server_sort(sort)
            && let Some(instant) = self.results.as_mut().and_then(|r| r.instant.as_mut())
            && instant.sent
            && instant.status.demotion.is_none()
            && instant.sent_sort != Some(wanted)
        {
            instant.sent = false;
            instant.refresh = false;
        }
        self.update_visible();
        self.reindex();
    }

    /// 숨김 항목 보이기/감추기. 앱 전체에 하나인 상태를 따르므로 보통 [`crate::window::App::set_show_hidden`]이
    /// 부른다. 다시 읽지 않는다.
    pub fn set_show_hidden(&mut self, show_hidden: bool) {
        if show_hidden == self.show_hidden {
            return;
        }
        self.show_hidden = show_hidden;
        self.update_visible();
        self.prune_selection();
        self.reindex();
    }

    /// 숨김 항목을 보이는가: 검색 결과 위치는 탐색 시작 때의 상태를 따르므로(결과에 이미 반영됨) 모두 보인다.
    fn shows_hidden(&self) -> bool {
        self.show_hidden || self.location.is_search_results()
    }

    fn update_visible(&mut self) {
        let show_hidden = self.shows_hidden();
        let hides = !show_hidden && self.list.iter().any(|i| i.attributes.hidden);
        if self.filter.is_none() {
            self.visible = if hides {
                Some(
                    self.list
                        .iter()
                        .enumerate()
                        .filter(|(_, item)| !item.attributes.hidden)
                        .map(|(i, _)| i)
                        .collect(),
                )
            } else {
                None
            };
            return;
        }
        // 필터 중: 맞은 번호가 지금 목록의 것이 아니면(그 사이 목록이 바뀜) 이름으로 다시 맞춘다.
        let gen_now = self.list_gen;
        if let Some(filter) = &mut self.filter
            && filter
                .matched
                .as_ref()
                .is_none_or(|(_, gen_at)| *gen_at != gen_now)
        {
            let matched = self
                .list
                .iter()
                .enumerate()
                .filter(|(_, item)| filter.matcher.is_match(&item.name))
                .map(|(i, _)| i)
                .collect();
            filter.matched = Some((matched, gen_now));
        }
        let matched = self
            .filter
            .as_ref()
            .and_then(|f| f.matched.as_ref())
            .map(|(matched, _)| matched.as_slice())
            .unwrap_or_default();
        self.visible = Some(
            matched
                .iter()
                .copied()
                .filter(|&i| {
                    show_hidden || self.list.get(i).is_some_and(|item| !item.attributes.hidden)
                })
                .collect(),
        );
    }

    // --- 항목 경로 ---

    /// 일괄 이름 바꾸기 미리 보기가 겹침을 볼 이름들: 실제 폴더면 탭이 읽은 그 폴더의 모든 이름(숨김 포함), 검색
    /// 결과면 결과 목록에 있는 항목들의 이름을 제 폴더별로. 목록을 다시 읽지 않는다.
    pub fn existing_names(&self) -> crate::batch_rename::ExistingNames {
        let mut names = crate::batch_rename::ExistingNames::new();
        match self.location.file_system_path() {
            Some(folder) => {
                for name in self.all_names() {
                    names.insert(folder, name);
                }
            }
            None => {
                for (index, item) in self.list.iter().enumerate() {
                    if let Some(folder) = self.list.folder(index) {
                        names.insert(folder, &item.name);
                    }
                }
            }
        }
        names
    }

    /// 보이는 항목 `index`가 든 폴더: 검색 결과 항목이면 그 항목의 폴더, 폴더 목록이면 탭 위치(실제 폴더일 때).
    pub fn folder_of(&self, index: usize) -> Option<&Path> {
        let list_index = self.list_index(index)?;
        match self.list.folder(list_index) {
            Some(folder) => Some(folder),
            None => self.location.file_system_path(),
        }
    }

    /// 보이는 항목 `index`의 전체 경로([`Tab::folder_of`] 안의 이름). 실제 파일 시스템 항목이 아니면 `None`이다.
    pub fn item_path(&self, index: usize) -> Option<PathBuf> {
        Some(self.folder_of(index)?.join(&self.get(index)?.name))
    }

    /// 커서 항목의 전체 경로.
    pub fn cursor_path(&self) -> Option<PathBuf> {
        self.item_path(self.cursor()?)
    }

    /// 선택한 항목의 전체 경로들(정렬된 순서). 검색 결과면 항목마다 제 폴더 경로다.
    pub fn selected_paths(&self) -> Vec<PathBuf> {
        self.selected_indices()
            .filter_map(|i| self.item_path(i))
            .collect()
    }

    // --- 검색 결과 위치 ---

    /// 검색 결과 위치면 검색 상태(검색 중인가, 진행, 끝난 요약). 다른 위치면 `None`이다.
    pub fn search(&self) -> Option<&SearchStatus> {
        self.results.as_ref().map(|results| &results.status)
    }

    /// 아직 결과 피드를 붙이지 않은 검색 결과 위치면 시작할 검색. 검색 결과 위치로 가는 읽기(이동·뒤로·앞으로·새로
    /// 고침)가 확정된 직후에만 있다. 숨김 항목 보이기는 그때의 탭 상태다.
    pub fn search_request(&self) -> Option<SearchRequest> {
        let results = self.results.as_ref()?;
        if results.feed.is_some()
            || results.content.is_some()
            || results.status.running
            || results.status.summary.is_some()
        {
            return None;
        }
        let scope = self.location.search_scope()?;
        if scope.kind != SearchKind::Name {
            return None;
        }
        // 시작할 때 `App::start_search`가 이미 해석해 본 글자다. 그래도 해석하지 못하면(오늘이 바뀌어도 문법은 그대로) 시작하지 않는다.
        let Ok(Parsed::Query(condition)) = query_syntax::parse(&scope.query, Today::now()) else {
            return None;
        };
        Some(SearchRequest {
            root: Location::file_system(&scope.folder),
            condition,
            show_hidden: results.status.show_hidden,
        })
    }

    /// [`Tab::search_request`]의 내용 검색 판: 아직 검색을 붙이지 않은 내용 검색 결과 위치면 시작할 내용 검색.
    pub fn content_request(&self) -> Option<ContentRequest> {
        let results = self.results.as_ref()?;
        if results.feed.is_some()
            || results.content.is_some()
            || results.status.running
            || results.status.summary.is_some()
        {
            return None;
        }
        let scope = self.location.search_scope()?;
        let SearchKind::Content { mode, name_pattern } = &scope.kind else {
            return None;
        };
        let mut request =
            ContentRequest::new(Location::file_system(&scope.folder), scope.query.clone());
        request.mode = *mode;
        request.name_pattern = name_pattern.clone();
        request.show_hidden = results.status.show_hidden;
        Some(request)
    }

    /// 내용 검색을 붙인다([`Tab::content_request`]로 시작한 검색). 검색 중이 된다. 내용 검색 결과 위치가 아니면 검색을
    /// 버리고(멈춘다) `false`다.
    pub fn attach_content_search(&mut self, search: ContentSearch) -> bool {
        let is_content = self
            .location
            .search_scope()
            .is_some_and(|scope| matches!(scope.kind, SearchKind::Content { .. }));
        let Some(results) = &mut self.results else {
            return false;
        };
        if !is_content {
            return false;
        }
        results.content = Some(search);
        results.status.running = true;
        results.status.summary = None;
        results.status.content = Some(ContentStatus::default());
        true
    }

    /// 결과를 흘려 넣을 피드를 붙인다([`Tab::search_request`]로 시작한 검색). 검색 중이 된다. 검색 결과 위치가 아니면
    /// 피드를 버리고(검색이 멈춘다) `false`다.
    pub fn attach_search(&mut self, feed: Box<dyn SearchFeed>) -> bool {
        let Some(results) = &mut self.results else {
            return false;
        };
        results.feed = Some(feed);
        results.status.running = true;
        results.status.summary = None;
        true
    }

    /// 검색을 시작하지 못했다(검색한 폴더가 사라짐 등): 까닭을 [`Tab::error`]에 둔다.
    pub fn search_failed(&mut self, error: ListError) {
        if let Some(results) = &mut self.results {
            results.feed = None;
            results.content = None;
            results.status.running = false;
        }
        self.error = Some(error);
    }

    /// 붙인 피드에서 온 사건을 모두 받아 결과에 넣는다. 바뀐 것이 있으면 `true`다. 끝남 사건을 받으면 피드를 버리고
    /// 검색 중이 아니다.
    pub fn pump_search(&mut self) -> bool {
        if self.results.as_ref().is_some_and(|r| r.content.is_some()) {
            return self.pump_content_search();
        }
        let Some(feed) = self.results.as_mut().and_then(|r| r.feed.as_mut()) else {
            return false;
        };
        let events = feed.poll();
        if events.is_empty() {
            return false;
        }
        for event in events {
            match event {
                SearchEvent::Batch { items, progress } => {
                    self.add_results(items);
                    if let Some(results) = &mut self.results {
                        results.status.progress = progress;
                    }
                }
                SearchEvent::Finished(summary) => {
                    if let Some(results) = &mut self.results {
                        results.feed = None;
                        results.status.running = false;
                        results.status.progress.found = summary.found;
                        results.status.progress.folders_scanned = summary.folders_scanned;
                        results.status.progress.folders_skipped = summary.folders_skipped;
                        results.status.summary = Some(summary);
                    }
                }
            }
        }
        true
    }

    /// [`Tab::pump_search`]의 내용 검색 판.
    fn pump_content_search(&mut self) -> bool {
        let Some(search) = self.results.as_mut().and_then(|r| r.content.as_mut()) else {
            return false;
        };
        let events = search.poll();
        if events.is_empty() {
            return false;
        }
        for event in events {
            match event {
                ContentEvent::Batch { items, progress } => {
                    self.add_content_results(items);
                    if let Some(results) = &mut self.results {
                        results.status.progress = SearchProgress {
                            found: progress.found,
                            folders_scanned: progress.folders_scanned,
                            folders_skipped: progress.folders_skipped,
                        };
                        results.status.content = Some(ContentStatus {
                            skipped: progress.skipped,
                            files_read: progress.files_read,
                            bytes_read: progress.bytes_read,
                        });
                    }
                }
                ContentEvent::Finished(summary) => {
                    if let Some(results) = &mut self.results {
                        results.content = None;
                        results.status.running = false;
                        let progress = summary.progress;
                        results.status.progress = SearchProgress {
                            found: progress.found,
                            folders_scanned: progress.folders_scanned,
                            folders_skipped: progress.folders_skipped,
                        };
                        results.status.content = Some(ContentStatus {
                            skipped: progress.skipped,
                            files_read: progress.files_read,
                            bytes_read: progress.bytes_read,
                        });
                        results.status.summary = Some(SearchSummary {
                            found: progress.found,
                            folders_scanned: progress.folders_scanned,
                            folders_skipped: progress.folders_skipped,
                            elapsed: summary.elapsed,
                            end: match summary.end {
                                ContentEnd::Finished => SearchEnd::Finished,
                                ContentEnd::Cancelled => SearchEnd::Cancelled,
                            },
                        });
                    }
                }
            }
        }
        true
    }

    /// 내용 검색으로 찾은 파일들(첫 일치 줄·일치 수 포함)을 결과 목록에 더한다. [`Tab::add_results`]와 같다.
    pub fn add_content_results(&mut self, found: Vec<ContentFound>) {
        if self.results.is_none() || found.is_empty() {
            return;
        }
        self.list_gen += 1;
        self.list.extend_hits(found.into_iter().map(|found| {
            let hit = Hit {
                first_line: found.first_line,
                matches: found.matches as u64,
            };
            (found.folder, found.item, Some(hit))
        }));
        self.update_visible();
        self.reindex();
    }

    /// 보이는 항목 `index`의 내용 검색 일치(첫 일치 줄·일치 수). 내용 검색 결과가 아니면 `None`이다.
    pub fn hit(&self, index: usize) -> Option<&Hit> {
        self.list.hit(self.list_index(index)?)
    }

    /// 찾은 항목들을 결과 목록에 더한다(정렬을 지킨다). 선택·커서는 경로로 남는다. 검색 결과 위치가 아니면 아무것도
    /// 하지 않는다. 결과를 넣는 쪽(실시간 탐색, 즉시 검색)과 상관없이 쓴다.
    pub fn add_results(&mut self, found: Vec<Found>) {
        if self.results.is_none() || found.is_empty() {
            return;
        }
        self.list_gen += 1;
        self.list
            .extend_found(found.into_iter().map(|found| (found.folder, found.item)));
        self.update_visible();
        self.reindex();
    }

    /// 검색을 시작하지 못한 채 끝낸다(시작 전 오류가 있는데 위치만 바뀐 경우): 검색 중이 아니고 결과는 비어 있다.
    pub fn stop_search_unstarted(&mut self) {
        if let Some(results) = &mut self.results {
            results.feed = None;
            results.content = None;
            results.status.running = false;
        }
    }

    /// 검색을 멈춘다(「검색 중지」): 지금까지의 결과는 남는다. 검색 중이 아니면 `false`다. 피드의 끝남 사건이 오면
    /// 요약([`SearchStatus::summary`])이 채워진다.
    pub fn stop_search(&mut self) -> bool {
        let Some(results) = &mut self.results else {
            return false;
        };
        if !results.status.running {
            return false;
        }
        if let Some(feed) = &results.feed {
            feed.stop();
        }
        if let Some(search) = &results.content {
            search.cancel();
        }
        // 즉시 검색: 기다리던 응답은 버린다(이미 받은 결과는 남는다).
        if let Some(instant) = &mut results.instant {
            instant.in_flight = None;
            instant.next = None;
            instant.live_due = None;
            instant.status.loading = false;
            instant.status.has_more = false;
        }
        results.status.running = false;
        true
    }

    // --- 즉시 검색 ---

    /// 즉시 검색의 범위: 「현재 폴더 아래만」인가. 위치를 옮겨도 탭에 남는다.
    pub fn instant_under(&self) -> bool {
        self.instant_under
    }

    /// 즉시 검색의 범위를 바꾼다. 즉시 검색 결과 위치(또는 그리로 가는 이동)면 같은 글자를 새 범위로 제자리에서 고치고
    /// ([`Tab::retarget_instant`]) 그 결과를 돌려준다. 아니면 범위만 탭에 남고 [`Retarget::NotSearching`]이다.
    pub fn set_instant_under(&mut self, under: bool) -> Retarget {
        self.instant_under = under;
        let current = if self.is_instant_search() {
            Some(self.location.clone())
        } else {
            self.pending
                .as_ref()
                .filter(|p| p.kind == Kind::New && is_instant_location(&p.location))
                .map(|p| p.location.clone())
        };
        match current {
            Some(Location::SearchResults(scope)) => self.retarget_instant(
                Location::instant_search_results(scope.folder, scope.query, under),
            ),
            _ => Retarget::NotSearching,
        }
    }

    /// 마지막으로 넘긴 즉시 검색 글자가 틀렸으면 까닭과 틀린 위치(UTF-8 바이트 범위).
    pub fn instant_error(&self) -> Option<&ParseError> {
        self.instant_error.as_ref()
    }

    /// 즉시 검색 글자 해석의 까닭을 두거나(`Some`) 지운다(`None`).
    pub fn set_instant_error(&mut self, error: Option<ParseError>) {
        self.instant_error = error;
    }

    /// 즉시 검색 결과 위치면 그 상태(전체 개수·덜 찼을 수 있음·서비스 없음). 다른 위치면 `None`이다.
    pub fn instant(&self) -> Option<&InstantStatus> {
        self.results
            .as_ref()
            .and_then(|r| r.instant.as_ref())
            .map(|instant| &instant.status)
    }

    /// 지금 위치가 즉시 검색 결과 위치인가.
    pub fn is_instant_search(&self) -> bool {
        is_instant_location(&self.location)
    }

    /// 즉시 검색 결과 위치로 가는 이동(새 위치)이 읽기를 기다리는 중인가.
    pub fn is_instant_search_pending(&self) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|p| p.kind == Kind::New && is_instant_location(&p.location))
    }

    /// 즉시 검색이 찾을 기준 폴더: 실제 폴더면 그 폴더, 즉시 검색 결과 위치면 검색 전 폴더. 다른 위치(이름·내용 검색
    /// 결과)면 `None`이고 모든 볼륨으로 찾는다. 「현재 폴더 아래만」은 이것이 있을 때만 켜진다.
    pub fn instant_base(&self) -> Option<PathBuf> {
        match self.location.search_scope() {
            None => self.location.file_system_path().map(Path::to_path_buf),
            Some(scope) if matches!(scope.kind, SearchKind::Instant { .. }) => {
                Some(scope.folder.clone())
            }
            Some(_) => None,
        }
    }

    /// 즉시 검색 위치를 `location`으로 고친다(입력하는 동안 기록 항목을 하나만 쓰려고, 글자·범위가 바뀔 때). 지금 위치가
    /// 즉시 검색 결과 위치면 그 자리에서 바꾸고 앞 결과는 새 응답이 올 때까지 그대로 둔다. 즉시 검색 결과 위치로 가는 이동이
    /// 읽기를 기다리는 중이면 그 이동의 목적지를 바꾼다. 둘 다 아니면 아무것도 하지 않는다(이동은 부르는 쪽 몫).
    pub fn retarget_instant(&mut self, location: Location) -> Retarget {
        if !is_instant_location(&location) {
            return Retarget::NotSearching;
        }
        if self.is_instant_search() && self.results.is_some() {
            if self.location == location {
                return Retarget::Unchanged;
            }
            self.location = location;
            if let Some(instant) = self.results.as_mut().and_then(|r| r.instant.as_mut()) {
                // 앞 질의의 응답은 더 받지 않는다(서비스는 새 질의로 앞 질의를 취소한다).
                instant.in_flight = None;
                instant.next = None;
                instant.sent = false;
                instant.refresh = false;
                instant.status.has_more = false;
                // 강등 중이던 앞선 실시간 탐색은 새 입력에서 멈춘다. 즉시 검색을 쓸 수 있는지는 다시 정한다.
                instant.live_due = None;
                instant.status.demotion = None;
                if let Some(results) = self.results.as_mut() {
                    results.feed = None;
                }
            }
            return Retarget::Replaced;
        }
        if let Some(pending) = &mut self.pending
            && pending.kind == Kind::New
            && is_instant_location(&pending.location)
        {
            if pending.location == location {
                return Retarget::Unchanged;
            }
            pending.location = location;
            return Retarget::Pending;
        }
        Retarget::NotSearching
    }

    /// 기다리는 읽기를 버린다(그 결과는 늦은 응답으로 버려진다). 위치는 그대로다.
    pub fn cancel_pending(&mut self) {
        self.pending = None;
    }

    /// 즉시 검색 질의를 아직 보내지 않았는가(위치가 막 확정됐거나, 글자·정렬이 바뀌었거나, 다시 보내야 한다).
    pub fn instant_needs_query(&self) -> bool {
        self.results
            .as_ref()
            .and_then(|r| r.instant.as_ref())
            .is_some_and(|instant| !instant.sent)
    }

    /// 지금 위치의 즉시 검색 질의: 글자를 질의 문법으로 해석한 조건 트리(「현재 폴더 아래만」이면 폴더 아래 조건을 그리고로
    /// 더함), 지금 정렬(서비스가 아는 열일 때), 한 화면 분량의 결과 수 제한([`PAGE_SIZE`]). 즉시 검색 결과 위치가 아니거나
    /// 글자가 빈 질의·틀린 문법이면 `None`이다.
    pub fn instant_query(&self) -> Option<facet_protocol::QueryMessage> {
        let scope = self.location.search_scope()?;
        let SearchKind::Instant { under } = scope.kind else {
            return None;
        };
        let Ok(Parsed::Query(condition)) = query_syntax::parse(&scope.query, Today::now()) else {
            return None;
        };
        let condition = if under {
            facet_protocol::Condition::And(vec![
                condition,
                facet_protocol::Condition::Under {
                    folder: scope.folder.to_string_lossy().into_owned(),
                },
            ])
        } else {
            condition
        };
        // 인덱스 변경을 따라 조용히 다시 받으면 스크롤로 이어 받은 만큼을 받는다.
        let refresh = self
            .results
            .as_ref()
            .and_then(|r| r.instant.as_ref())
            .is_some_and(|instant| instant.refresh);
        let limit = if refresh {
            instant_search::refresh_limit(self.list.len())
        } else {
            PAGE_SIZE
        };
        Some(facet_protocol::QueryMessage {
            condition,
            sort: instant_search::server_sort(self.sort),
            limit: Some(limit),
        })
    }

    /// 질의를 요청 `id`로 보냈다: 그 응답을 기다린다(앞선 요청의 응답은 버린다). 인덱스 변경을 따라 조용히 다시 받는
    /// 질의면([`Tab::instant_follow`]) 기다리는 동안에도 앞 결과와 상태가 그대로이고 「받는 중」이 되지 않는다.
    pub fn instant_sent(&mut self, id: IndexerRequestId) {
        let sort = instant_search::server_sort(self.sort);
        if let Some(results) = &mut self.results
            && let Some(instant) = &mut results.instant
        {
            if std::mem::take(&mut instant.refresh) {
                instant.in_flight = Some((id, Flight::Refresh));
                instant.sent = true;
                instant.sent_sort = sort;
                // 같은 연결의 새 질의는 서비스에서 앞 표지를 지운다.
                instant.next = None;
                return;
            }
            instant.in_flight = Some((id, Flight::Query));
            instant.sent = true;
            instant.sent_sort = sort;
            instant.next = None;
            instant.status.loading = true;
            instant.status.has_more = false;
            results.status.running = true;
        }
    }

    /// 서비스에 질의를 보낼 수 없었다(연결 판정 중): 기다린다. 서비스 없음·버전 다름은 [`Tab::instant_demote`]다.
    pub fn instant_unavailable(&mut self, why: Unavailable) {
        if let Some(results) = &mut self.results
            && let Some(instant) = &mut results.instant
        {
            instant.sent = true;
            instant.in_flight = None;
            instant.next = None;
            instant.status.unavailable = Some(why);
            instant.status.loading = false;
            instant.status.has_more = false;
            results.status.running = false;
        }
    }

    /// 범위 볼륨의 인덱스를 아직 확인하지 않아 상태 질의를 먼저 보내야 하는가.
    pub fn instant_needs_status(&self) -> bool {
        self.results
            .as_ref()
            .and_then(|r| r.instant.as_ref())
            .is_some_and(|i| !i.sent && !i.volume_checked && i.status.demotion.is_none())
    }

    /// 상태 질의를 요청 `id`로 보냈다.
    pub fn instant_status_sent(&mut self, id: IndexerRequestId) {
        if let Some(results) = &mut self.results
            && let Some(instant) = &mut results.instant
        {
            instant.in_flight = Some((id, Flight::Status));
            instant.sent = true;
            instant.status.loading = true;
            results.status.running = true;
        }
    }

    /// 즉시 검색을 실시간 탐색으로 강등한다: 같은 조건 트리로 검색 전 폴더 아래를 훑는다(앱이 시작한다,
    /// [`Tab::demoted_search_due`]). 범위가 모든 볼륨이었으면 폴더 아래로 좁아진다([`Demotion::narrowed_to_folder`]). 이어
    /// 치는 중인 입력이면 [`DEMOTED_SEARCH_DELAY`] 뒤에 시작한다. 즉시 검색 결과 위치가 아니면 아무것도 하지 않는다.
    pub fn instant_demote(&mut self, reason: DemotionReason, now: Instant) {
        let Some(SearchKind::Instant { under }) =
            self.location.search_scope().map(|s| s.kind.clone())
        else {
            return;
        };
        let Some(results) = &mut self.results else {
            return;
        };
        let Some(instant) = &mut results.instant else {
            return;
        };
        let delay = if instant.was_demoted {
            DEMOTED_SEARCH_DELAY
        } else {
            Duration::ZERO
        };
        instant.was_demoted = true;
        instant.sent = true;
        instant.refresh = false;
        instant.in_flight = None;
        instant.next = None;
        instant.live_due = Some(now + delay);
        instant.status.loading = false;
        instant.status.has_more = false;
        instant.status.unavailable = None;
        instant.status.total = None;
        instant.status.total_is_upper_bound = false;
        instant.status.building.clear();
        instant.status.demotion = Some(Demotion {
            reason,
            narrowed_to_folder: !under,
        });
        results.feed = None;
        results.status.running = true;
        results.status.summary = None;
    }

    /// 강등한 실시간 탐색을 시작할 시각. 이미 시작했거나 강등 중이 아니면 `None`이다.
    pub fn demoted_search_due(&self) -> Option<Instant> {
        let results = self.results.as_ref()?;
        if results.feed.is_some() {
            return None;
        }
        results.instant.as_ref()?.live_due
    }

    /// 강등한 실시간 탐색의 요청: 즉시 검색과 같은 조건 트리를 검색 전 폴더에서 훑는다(「현재 폴더 아래만」의 폴더 조건은
    /// 시작점이 이미 그 폴더라 더하지 않는다).
    pub fn demoted_search_request(&self) -> Option<SearchRequest> {
        self.demoted_search_due()?;
        let results = self.results.as_ref()?;
        let scope = self.location.search_scope()?;
        let Ok(Parsed::Query(condition)) = query_syntax::parse(&scope.query, Today::now()) else {
            return None;
        };
        Some(SearchRequest {
            root: Location::file_system(&scope.folder),
            condition,
            show_hidden: results.status.show_hidden,
        })
    }

    /// 강등한 실시간 탐색을 붙인다: 앞 결과(즉시 검색이나 앞선 탐색)는 비우고 새 탐색의 결과를 받는다.
    pub fn attach_demoted_search(&mut self, feed: Box<dyn SearchFeed>) {
        if self.results.is_none() {
            return;
        }
        self.list_gen += 1;
        self.list = DetailsList::new(Vec::new(), self.sort);
        self.update_visible();
        self.prune_selection();
        self.reindex();
        if let Some(results) = &mut self.results {
            if let Some(instant) = &mut results.instant {
                instant.live_due = None;
            }
            results.status.progress = SearchProgress::default();
        }
        self.attach_search(feed);
    }

    /// 강등한 실시간 탐색을 시작하지 못했다.
    pub fn demoted_search_failed(&mut self, error: ListError) {
        if let Some(instant) = self.results.as_mut().and_then(|r| r.instant.as_mut()) {
            instant.live_due = None;
        }
        self.search_failed(error);
    }

    /// 서비스 연결이 서비스 없음·버전 다름으로 판정돼 기다리던 탭이면 강등하고 `true`다.
    pub fn instant_connection_lost(&mut self, reason: DemotionReason, now: Instant) -> bool {
        let waiting = self
            .results
            .as_ref()
            .and_then(|r| r.instant.as_ref())
            .is_some_and(|i| matches!(i.status.unavailable, Some(Unavailable::Connection(_))));
        if waiting {
            self.instant_demote(reason, now);
        }
        waiting
    }

    /// 서비스에 닿지 못해 멈춰 있던 탭이면 다시 보낼 질의로 바꾸고 `true`다(연결이 맺어지면 앱이 부른다).
    pub fn instant_retry(&mut self) -> bool {
        let Some(instant) = self.results.as_mut().and_then(|r| r.instant.as_mut()) else {
            return false;
        };
        if matches!(
            instant.status.unavailable,
            Some(Unavailable::Connection(_) | Unavailable::Disconnected)
        ) {
            instant.sent = false;
            true
        } else {
            false
        }
    }

    /// 다른 탭이 질의를 보내 이 탭의 질의가 취소됐다: 기다리던 응답과 이어 받기를 놓는다(받은 결과는 남는다).
    pub fn instant_superseded(&mut self) {
        if let Some(results) = &mut self.results
            && let Some(instant) = &mut results.instant
        {
            instant.in_flight = None;
            instant.next = None;
            instant.status.loading = false;
            instant.status.has_more = false;
            results.status.running = false;
        }
    }

    /// 인덱스 변경을 확인할 때([`instant_search::FOLLOW_INTERVAL`]): 보이는 즉시 검색 결과 탭이 결과를 받아 둔 채 아무
    /// 응답도 기다리지 않을 때만 있다. 숨은 탭·강등한 탭·서비스에 닿지 못한 탭·결과가 아직 없는 탭은 `None`이다.
    pub fn instant_follow_due(&self) -> Option<Instant> {
        if !self.shown || !self.is_instant_search() {
            return None;
        }
        let instant = self.results.as_ref()?.instant.as_ref()?;
        if !instant.sent
            || instant.refresh
            || instant.in_flight.is_some()
            || instant.live_due.is_some()
            || instant.status.demotion.is_some()
            || instant.status.unavailable.is_some()
        {
            return None;
        }
        Some(instant.checked_at? + FOLLOW_INTERVAL)
    }

    /// 인덱스가 바뀌었는지 확인할 요청: 이어 받기 표지가 있으면 결과 0개짜리 이어 받기, 없으면 같은 질의를 조용히 다시
    /// 받기다. 시각은 보지 않는다(부르는 쪽이 [`Tab::instant_follow_due`]로 본다). 따라갈 결과가 없으면 `None`이다.
    pub fn instant_follow(&self) -> Option<Follow> {
        self.instant_follow_due()?;
        let instant = self.results.as_ref()?.instant.as_ref()?;
        Some(match instant.next {
            Some(cursor) => Follow::Probe(facet_protocol::ResumeMessage { cursor, limit: 0 }),
            None => Follow::Refresh,
        })
    }

    /// 확인하는 이어 받기를 요청 `id`로 보냈다(앞 결과와 상태는 그대로다).
    pub fn instant_probe_sent(&mut self, id: IndexerRequestId) {
        if let Some(instant) = self.results.as_mut().and_then(|r| r.instant.as_mut()) {
            instant.in_flight = Some((id, Flight::Probe));
        }
    }

    /// 다음 질의를 인덱스 변경을 따라가는 조용한 다시 받기로 한다(받은 만큼, 앞 결과를 그대로 보이며). 보낼 질의가
    /// 생겼으면 `true`이고 앱이 보낸다([`Tab::instant_needs_query`]).
    pub fn request_instant_refresh(&mut self) -> bool {
        let Some(instant) = self.results.as_mut().and_then(|r| r.instant.as_mut()) else {
            return false;
        };
        if instant.in_flight.is_some() || instant.status.demotion.is_some() {
            return false;
        }
        instant.refresh = true;
        instant.sent = false;
        true
    }

    /// 마지막으로 결과에서 열려던 항목(열기·새 탭에서 열기·「폴더에서 보기」)이 디스크에 없었으면 그 경로. 그 항목은
    /// 결과에서 뺐다. 다음 열기가 지운다.
    pub fn missing_item(&self) -> Option<&Path> {
        self.missing_item.as_deref()
    }

    pub(crate) fn set_missing_item(&mut self, path: Option<PathBuf>) {
        self.missing_item = path;
    }

    /// 이어 받을 결과가 더 있고 지금 응답을 기다리지 않는가.
    pub fn has_more_results(&self) -> bool {
        self.instant_resume().is_some()
    }

    /// 이어 받기 요청: 남은 결과가 있고 응답을 기다리는 중이 아닐 때만.
    pub fn instant_resume(&self) -> Option<facet_protocol::ResumeMessage> {
        let instant = self.results.as_ref()?.instant.as_ref()?;
        if instant.in_flight.is_some() {
            return None;
        }
        Some(facet_protocol::ResumeMessage {
            cursor: instant.next?,
            limit: PAGE_SIZE,
        })
    }

    /// 이어 받기를 요청 `id`로 보냈다.
    pub fn instant_resume_sent(&mut self, id: IndexerRequestId) {
        if let Some(results) = &mut self.results
            && let Some(instant) = &mut results.instant
        {
            instant.in_flight = Some((id, Flight::Resume));
            instant.status.loading = true;
            results.status.running = true;
        }
    }

    /// 질의나 이어 받기를 기다리는 요청 번호(다른 탭에 가려지지 않았다면).
    pub fn instant_in_flight(&self) -> Option<IndexerRequestId> {
        self.results
            .as_ref()?
            .instant
            .as_ref()?
            .in_flight
            .map(|(id, _)| id)
    }

    /// 서비스의 응답을 받는다. 이 탭이 기다리는 요청의 응답일 때만 반영한다(늦게 온 앞선 질의의 응답은 버린다).
    pub fn instant_reply(
        &mut self,
        id: IndexerRequestId,
        result: Result<Reply, RequestError>,
    ) -> InstantReply {
        let folder = self.location.search_scope().map(|s| s.folder.clone());
        let Some(instant) = self.results.as_mut().and_then(|r| r.instant.as_mut()) else {
            return InstantReply::Ignored;
        };
        let flight = match instant.in_flight {
            Some((waiting, flight)) if waiting == id => flight,
            _ => return InstantReply::Ignored,
        };
        instant.in_flight = None;
        if matches!(flight, Flight::Probe | Flight::Refresh) {
            return self.instant_follow_reply(flight, result);
        }
        instant.status.loading = false;
        let mut found = Vec::new();
        let mut replace = false;
        let mut requery = false;
        let mut demote = None;
        match result {
            Ok(Reply::Page(page)) if flight != Flight::Status => {
                instant.status.total = Some(page.total);
                instant.status.total_is_upper_bound = page.total_is_upper_bound;
                instant.status.building = page.building;
                instant.status.unavailable = None;
                instant.next = page.next;
                instant.status.has_more = page.next.is_some();
                instant.checked_at = Some(Instant::now());
                replace = flight == Flight::Query;
                found = page.hits.iter().map(found_from_hit).collect();
            }
            Ok(Reply::Status(status)) if flight == Flight::Status => {
                if folder
                    .as_deref()
                    .is_some_and(|folder| instant_search::volume_is_indexed(&status, folder))
                {
                    // 인덱스가 있다: 질의를 보낸다.
                    instant.volume_checked = true;
                    instant.sent = false;
                    requery = true;
                } else {
                    demote = Some(DemotionReason::VolumeNotIndexed);
                }
            }
            Ok(_) | Err(RequestError::Unexpected) => {
                instant.status.unavailable = Some(Unavailable::Disconnected);
            }
            Err(RequestError::Service(error))
                if error.kind == facet_protocol::ErrorKind::Cancelled =>
            {
                // 새 질의가 이 요청을 취소했다. 새 질의의 응답을 기다린다.
                return InstantReply::Ignored;
            }
            Err(RequestError::Service(error))
                if error.kind == facet_protocol::ErrorKind::StaleCursor =>
            {
                // 인덱스가 바뀌어 이어 받기가 무효다: 받은 만큼 조용히 다시 받는다(그 뒤 이어 받는다).
                instant.next = None;
                instant.sent = false;
                instant.refresh = true;
                requery = true;
            }
            Err(RequestError::Service(error)) => {
                instant.next = None;
                instant.status.has_more = false;
                instant.status.unavailable = Some(Unavailable::Service(error));
            }
            Err(RequestError::NotConnected(Connection::NoService)) => {
                demote = Some(DemotionReason::NoService);
            }
            Err(RequestError::NotConnected(Connection::VersionMismatch { .. })) => {
                demote = Some(DemotionReason::VersionMismatch);
            }
            Err(RequestError::NotConnected(connection)) => {
                instant.next = None;
                instant.status.has_more = false;
                instant.status.unavailable = Some(Unavailable::Connection(connection));
            }
            // 답을 기다리다 서비스가 멈췄다: 탭이 멈추지 않고 그 질의를 강등한다.
            Err(RequestError::Disconnected) => demote = Some(DemotionReason::NoService),
        }
        if let Some(reason) = demote {
            self.instant_demote(reason, Instant::now());
            return InstantReply::Demoted;
        }
        if let Some(results) = &mut self.results {
            results.status.running = false;
        }
        if requery {
            return InstantReply::Requery;
        }
        if replace {
            // 새 질의의 결과: 앞 결과를 바꾼다(앞 결과는 이 응답이 올 때까지 그대로 보였다).
            self.list_gen += 1;
            self.list = DetailsList::new(Vec::new(), self.sort);
        }
        if !found.is_empty() {
            self.list_gen += 1;
            self.list
                .extend_found(found.into_iter().map(|found| (found.folder, found.item)));
        }
        self.update_visible();
        self.prune_selection();
        self.reindex();
        let loaded = self.list.len();
        if let Some(results) = &mut self.results {
            results.status.progress.found = loaded;
        }
        InstantReply::Applied
    }

    /// 인덱스 변경을 확인한 요청의 응답. 확인하는 이어 받기가 표지 무효로 답하면 조용히 다시 받고([`InstantReply::Requery`]),
    /// 다시 받은 결과가 앞 결과와 다르면 바꾼다([`InstantReply::Applied`]). 선택·커서·범위 기준은 전체 경로로 남고 사라진
    /// 항목은 빠진다. 연결·서비스 오류는 결과를 그대로 두고 다음 확인 때 다시 본다(강등하지 않는다: 질의를 기다리던 것이 아니다).
    fn instant_follow_reply(
        &mut self,
        flight: Flight,
        result: Result<Reply, RequestError>,
    ) -> InstantReply {
        let now = Instant::now();
        let Some(instant) = self.results.as_mut().and_then(|r| r.instant.as_mut()) else {
            return InstantReply::Ignored;
        };
        instant.checked_at = Some(now);
        let page = match result {
            Ok(Reply::Page(page)) => page,
            Err(RequestError::Service(error))
                if error.kind == facet_protocol::ErrorKind::StaleCursor =>
            {
                instant.next = None;
                instant.refresh = true;
                instant.sent = false;
                return InstantReply::Requery;
            }
            Err(RequestError::Service(error))
                if error.kind == facet_protocol::ErrorKind::Cancelled =>
            {
                return InstantReply::Ignored;
            }
            _ => {
                // 표지는 그대로 쓸 수 있는지 모른다. 다음 확인은 같은 질의를 다시 받는다.
                instant.next = None;
                return InstantReply::Unchanged;
            }
        };
        instant.next = page.next;
        instant.status.has_more = page.next.is_some();
        if flight == Flight::Probe {
            // 표지가 아직 맞다: 인덱스가 그대로다.
            return InstantReply::Unchanged;
        }
        let summary_changed = instant.status.total != Some(page.total)
            || instant.status.total_is_upper_bound != page.total_is_upper_bound
            || instant.status.building != page.building;
        instant.status.total = Some(page.total);
        instant.status.total_is_upper_bound = page.total_is_upper_bound;
        instant.status.building = page.building;
        let found: Vec<Found> = page.hits.iter().map(found_from_hit).collect();
        let fingerprint = |folder: &Path, item: &Item| {
            (
                folder.join(&item.name),
                item.is_folder,
                item.size,
                item.modified,
                item.attributes.hidden,
                item.attributes.system,
                item.attributes.read_only,
            )
        };
        let mut before: Vec<_> = (0..self.list.len())
            .filter_map(|i| Some(fingerprint(self.list.folder(i)?, self.list.get(i)?)))
            .collect();
        let mut after: Vec<_> = found
            .iter()
            .map(|f| fingerprint(&f.folder, &f.item))
            .collect();
        before.sort();
        after.sort();
        if before == after {
            return if summary_changed {
                InstantReply::Applied
            } else {
                InstantReply::Unchanged
            };
        }
        self.list_gen += 1;
        self.list = DetailsList::new(Vec::new(), self.sort);
        self.list
            .extend_found(found.into_iter().map(|found| (found.folder, found.item)));
        self.update_visible();
        self.prune_selection();
        self.reindex();
        let loaded = self.list.len();
        if let Some(results) = &mut self.results {
            results.status.progress.found = loaded;
        }
        InstantReply::Applied
    }

    /// facet이 한 작업의 변화를 결과 목록에 반영한다: 지우거나 옮긴 항목(폴더면 그 아래 결과까지)은 빠지고, 같은 폴더
    /// 안에서 이름을 바꾼 항목은 새 이름이 된다(폴더면 그 아래 결과의 위치도 바뀐다). 선택·커서·범위 기준도 따라간다.
    /// 검색 결과 위치가 아니면 아무것도 하지 않고 `false`다.
    pub fn apply_path_changes(&mut self, changes: &[PathChange]) -> bool {
        if self.results.is_none() || changes.is_empty() {
            return false;
        }
        let renames: Vec<(&Path, &Path)> = changes
            .iter()
            .filter_map(|change| match change {
                PathChange::Renamed { from, to } if same_parent(from, to) => {
                    Some((from.as_path(), to.as_path()))
                }
                _ => None,
            })
            .collect();
        let gone: Vec<&Path> = changes
            .iter()
            .filter_map(|change| match change {
                PathChange::Removed(path) => Some(path.as_path()),
                PathChange::Renamed { from, to } if !same_parent(from, to) => Some(from.as_path()),
                PathChange::Renamed { .. } => None,
            })
            .collect();
        let changed = self.list.apply_path_changes(
            |folder, item| {
                let path = folder.join(&item.name);
                !gone.iter().any(|gone| is_within(&path, gone))
            },
            |folder, item| {
                let path = folder.join(&item.name);
                renames.iter().find_map(|(from, to)| {
                    if is_within(from, &path) && is_within(&path, from) {
                        // 그 항목의 이름이 바뀌었다.
                        Some((Arc::from(folder), to.file_name()?.to_os_string()))
                    } else {
                        // 위 폴더의 이름이 바뀌었다.
                        let folder = rebase(folder, from, to)?;
                        Some((Arc::from(folder.as_path()), item.name.clone()))
                    }
                })
            },
        );
        if !changed {
            return false;
        }
        // 선택·커서·범위 기준이 기억하던 경로를 새 경로로 옮긴다.
        let moved = |key: &OsString| -> Option<OsString> {
            renames.iter().find_map(|(from, to)| {
                rebase(Path::new(key), from, to).map(PathBuf::into_os_string)
            })
        };
        self.selection = std::mem::take(&mut self.selection)
            .into_iter()
            .map(|key| moved(&key).unwrap_or(key))
            .collect();
        if let Some(anchor) = self.anchor.take() {
            self.anchor = Some(moved(&anchor).unwrap_or(anchor));
        }
        if let Some((cursor, index)) = self.cursor.take() {
            self.cursor = Some((moved(&cursor).unwrap_or(cursor), index));
        }
        self.list_gen += 1;
        self.update_visible();
        self.prune_selection();
        self.reindex();
        true
    }

    /// 「폴더에서 보기」: 커서 결과 항목의 상위 폴더로 가는 이동을 시작하고 읽기 요청을 돌려준다. 확정되면 기록에 남고
    /// 그 항목이 선택·커서다. 검색 결과 위치가 아니거나 커서 항목이 없으면 `None`이다.
    pub fn show_in_folder(&mut self) -> Option<ReadRequest> {
        if !self.location.is_search_results() {
            return None;
        }
        let index = self.cursor()?;
        let folder = self.folder_of(index)?.to_path_buf();
        let name = self.get(index)?.name.clone();
        let request = self.navigate(Navigation::Go(Location::file_system(folder)))?;
        if let Some(pending) = &mut self.pending {
            pending.reselect = Some(name);
        }
        Some(request)
    }

    // --- 필터 ---

    /// 지금 적용한 필터 글자. 필터가 없으면 `None`이다. 위치를 바꾸면(뒤로·앞으로·위로·주소창 포함) 풀린다.
    pub fn filter(&self) -> Option<&str> {
        self.filter.as_ref().map(|f| f.text.as_str())
    }

    /// 필터 글자를 정한다: 이미 읽은 목록만 거르고 파일 시스템을 다시 읽지 않는다. 글자가 비면 필터를 풀고
    /// `None`이다(곧바로 반영). 아니면 거를 요청을 돌려주니, 일꾼 스레드에서 [`FilterRequest::run`]해 그 결과를
    /// [`Tab::finish_filter`]에 넘긴다. 그동안 목록은 앞 필터 그대로다. 앞선 요청의 결과는 늦게 오면 버린다.
    pub fn request_filter(&mut self, text: &str) -> Option<FilterRequest> {
        if text.is_empty() {
            self.clear_filter();
            return None;
        }
        let id = FilterId(self.next_filter);
        self.next_filter += 1;
        // 앞 결과가 부분 일치였고 글자를 덧붙였으면 앞 결과 안에서만 거른다(와일드카드가 든 앞 글자는 덧붙여도
        // 좁아진다고 할 수 없어 처음부터).
        let narrowed = self.filter.as_ref().filter(|f| {
            !has_wildcard(&f.text) && text.to_lowercase().starts_with(&f.text.to_lowercase())
        });
        let candidates = match narrowed.and_then(|f| f.matched.as_ref()) {
            // 앞 결과의 번호가 지금 목록의 것일 때만 그 안에서 거른다.
            Some((matched, gen_at)) if *gen_at == self.list_gen => Some(matched.clone()),
            _ => None,
        };
        let names = match &self.names {
            Some((gen_at, names)) if *gen_at == self.list_gen => Arc::clone(names),
            _ => {
                let names = Arc::new(self.list.iter().map(|item| item.name.clone()).collect());
                self.names = Some((self.list_gen, Arc::clone(&names)));
                names
            }
        };
        Some(FilterRequest {
            id,
            text: text.to_string(),
            names,
            candidates,
            list_gen: self.list_gen,
        })
    }

    /// 거르기 결과를 받는다. 마지막 요청의 결과일 때만 반영하고 `true`다. 늦게 온 앞선 요청의 결과는 버리고
    /// `false`다. 거르는 동안 목록을 다시 읽었으면 새 목록을 처음부터 다시 거른다.
    pub fn finish_filter(&mut self, result: FilterResult) -> bool {
        if result.id.0 + 1 != self.next_filter {
            return false;
        }
        // 거르는 동안 목록이 바뀌었으면 번호가 어긋나므로 이름으로 다시 맞춘다(`update_visible`).
        let matched =
            (result.list_gen == self.list_gen).then_some((result.matched, result.list_gen));
        self.filter = Some(Filter {
            matcher: NameMatcher::new(&result.text),
            text: result.text,
            matched,
        });
        self.after_filter_change();
        true
    }

    /// 요청·결과 없이 곧바로 거른다(작은 목록과 시험용). 막으므로 큰 목록은 [`Tab::request_filter`]를 쓴다.
    pub fn set_filter(&mut self, text: &str) {
        if let Some(request) = self.request_filter(text) {
            let result = request.run();
            self.finish_filter(result);
        }
    }

    /// 필터를 풀고 원래 목록으로 돌아간다. 기다리는 거르기 결과는 버린다.
    pub fn clear_filter(&mut self) {
        self.next_filter += 1;
        if self.filter.take().is_some() {
            self.after_filter_change();
        }
    }

    fn after_filter_change(&mut self) {
        self.update_visible();
        self.prune_selection();
        self.reindex();
    }

    // --- 선택·커서 ---

    /// 지금 보기 상태를 꺼내 간다(기록에 남길 때). 선택·커서·기준은 비고 스크롤은 0이 된다.
    fn take_view(&mut self) -> ViewState {
        // 다른 위치로 가면 사각형 선택은 끝난다(앞 위치의 이름을 들고 가지 않는다).
        self.marquee_base = None;
        ViewState {
            selection: std::mem::take(&mut self.selection),
            cursor: self.cursor.take().map(|(name, _)| name),
            anchor: self.anchor.take(),
            scroll: std::mem::take(&mut self.scroll),
        }
    }

    /// 보이는 항목 `index`의 `list` 안 번호.
    fn list_index(&self, index: usize) -> Option<usize> {
        match &self.visible {
            Some(visible) => visible.get(index).copied(),
            None => (index < self.list.len()).then_some(index),
        }
    }

    /// 보이는 항목 `index`의 선택 열쇠: 폴더 목록이면 이름, 검색 결과면 전체 경로(다른 폴더의 같은 이름을 가른다).
    fn key_ref(&self, index: usize) -> Option<Cow<'_, OsStr>> {
        self.list.selection_key(self.list_index(index)?)
    }

    fn key(&self, index: usize) -> Option<OsString> {
        self.key_ref(index).map(Cow::into_owned)
    }

    /// 열쇠가 `key`(폴더 목록이면 이름, 검색 결과면 전체 경로)인 보이는 항목의 번호. 항목 수만큼 훑는다.
    fn position_of(&self, key: &OsStr) -> Option<usize> {
        (0..self.len()).find(|&i| {
            self.list_index(i)
                .is_some_and(|list_index| self.list.has_key(list_index, key))
        })
    }

    /// 보이지 않거나 사라진 항목을 선택·기준에서 뺀다. 이름 집합을 한 번 만들어 대조하므로 항목 수에 비례한다.
    fn prune_selection(&mut self) {
        let base_empty = self.marquee_base.as_ref().is_none_or(HashSet::is_empty);
        if !self.selection.is_empty() || !base_empty {
            let mut selection = std::mem::take(&mut self.selection);
            let mut base = self.marquee_base.take();
            let present: HashSet<Cow<'_, OsStr>> =
                (0..self.len()).filter_map(|i| self.key_ref(i)).collect();
            selection.retain(|name| present.contains(name.as_os_str()));
            // 사각형 선택 중이면 끌기 전 선택에서도 뺀다.
            if let Some(base) = base.as_mut() {
                base.retain(|name| present.contains(name.as_os_str()));
            }
            self.selection = selection;
            self.marquee_base = base;
        }
        if let Some(anchor) = &self.anchor
            && self.position_of(anchor).is_none()
        {
            self.anchor = None;
        }
    }

    /// 커서의 번호를 지금 목록에 맞춘다. 커서 항목이 사라졌으면 원래 번호 근처 항목으로 옮긴다.
    fn reindex(&mut self) {
        let Some((name, old)) = self.cursor.take() else {
            return;
        };
        let len = self.len();
        if len == 0 {
            return;
        }
        let index = self.position_of(&name).unwrap_or_else(|| old.min(len - 1));
        if let Some(key) = self.key(index) {
            self.cursor = Some((key, index));
        }
    }

    /// 커서 항목의 번호.
    pub fn cursor(&self) -> Option<usize> {
        self.cursor.as_ref().map(|(_, index)| *index)
    }

    /// 커서 항목.
    pub fn cursor_item(&self) -> Option<&Item> {
        self.get(self.cursor()?)
    }

    fn set_cursor(&mut self, index: usize) -> Option<usize> {
        let key = self.key(index)?;
        self.cursor = Some((key, index));
        Some(index)
    }

    /// 커서를 옮긴다. 선택은 바꾸지 않는다. 옮긴 뒤의 번호를 돌려주고, 항목이 없으면 `None`이다.
    ///
    /// 커서가 없으면 첫 항목에 둔다. 끝에서는 더 가지 않고 그 자리에 머문다.
    pub fn move_cursor(&mut self, movement: CursorMove) -> Option<usize> {
        let len = self.len();
        if len == 0 {
            return None;
        }
        let target = match (self.cursor(), movement) {
            (_, CursorMove::First) => 0,
            (_, CursorMove::Last) => len - 1,
            (None, _) => 0,
            (Some(at), CursorMove::Up) => at.saturating_sub(1),
            (Some(at), CursorMove::Down) => (at + 1).min(len - 1),
            (Some(at), CursorMove::PageUp(page)) => at.saturating_sub(page.max(1)),
            (Some(at), CursorMove::PageDown(page)) => (at + page.max(1)).min(len - 1),
        };
        self.set_cursor(target)
    }

    /// `index`번째 항목이 선택됐는가.
    pub fn is_selected(&self, index: usize) -> bool {
        self.key_ref(index)
            .is_some_and(|key| self.selection.contains(&*key))
    }

    /// 선택한 항목 수.
    pub fn selected_count(&self) -> usize {
        self.selection.len()
    }

    /// 선택한 항목을 정렬된 순서로.
    pub fn selected_items(&self) -> impl Iterator<Item = &Item> + '_ {
        self.selected_indices().filter_map(|i| self.get(i))
    }

    /// 선택한 항목의 보이는 번호를 정렬된 순서로.
    pub fn selected_indices(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.len()).filter(|&i| !self.selection.is_empty() && self.is_selected(i))
    }

    /// 단일 선택(클릭): `index`번째만 선택하고 커서와 범위 기준을 거기 둔다. 범위 밖이면 아무것도 안 한다.
    pub fn select_only(&mut self, index: usize) {
        let Some(name) = self.key(index) else {
            return;
        };
        self.selection.clear();
        self.selection.insert(name.clone());
        self.anchor = Some(name);
        self.set_cursor(index);
    }

    /// Shift 범위: 범위 기준부터 `index`까지를 선택하고(앞선 선택은 버린다) 커서를 `index`에 둔다.
    /// 기준이 없으면 커서, 그것도 없으면 `index` 자신이 기준이다.
    pub fn select_range(&mut self, index: usize) {
        if index >= self.len() {
            return;
        }
        let anchor = self
            .anchor
            .as_deref()
            .and_then(|name| self.position_of(name))
            .or(self.cursor())
            .unwrap_or(index);
        let (from, to) = (anchor.min(index), anchor.max(index));
        let names: HashSet<OsString> = (from..=to).filter_map(|i| self.key(i)).collect();
        let anchor_name = self.key(anchor);
        self.selection = names;
        self.anchor = anchor_name;
        self.set_cursor(index);
    }

    /// Ctrl 토글: `index`번째의 선택을 뒤집고 커서와 범위 기준을 거기 둔다.
    pub fn toggle_select(&mut self, index: usize) {
        let Some(name) = self.key(index) else {
            return;
        };
        if !self.selection.remove(&name) {
            self.selection.insert(name.clone());
        }
        self.anchor = Some(name);
        self.set_cursor(index);
    }

    /// `index`번째 항목을 가리킨다(클릭 등): `pick`대로 선택을 바꾸고 커서를 거기 둔다. 범위 밖이면 아무것도 안 한다.
    pub fn pick(&mut self, index: usize, pick: Pick) {
        match pick {
            Pick::Only => self.select_only(index),
            Pick::Range => self.select_range(index),
            Pick::Toggle => self.toggle_select(index),
            Pick::CursorOnly => {
                self.set_cursor(index);
            }
        }
    }

    /// 커서 키: 커서를 옮기고([`Tab::move_cursor`]) 옮긴 항목을 `pick`대로 고른다. 옮긴 뒤의 번호를 돌려주고,
    /// 항목이 없으면 `None`이다.
    pub fn move_cursor_picking(&mut self, movement: CursorMove, pick: Pick) -> Option<usize> {
        // 범위의 기준은 옮기기 전 커서다(기준이 없을 때).
        if pick == Pick::Range && self.anchor.is_none() {
            self.anchor = self.cursor().and_then(|index| self.key(index));
        }
        let index = self.move_cursor(movement)?;
        self.pick(index, pick);
        Some(index)
    }

    /// 글자로 이동하고([`Tab::type_to_find`]) 찾은 항목만 선택한다(탐색기와 같다). 찾지 못하면 선택도 커서도
    /// 그대로고 `None`이다.
    pub fn type_to_select(&mut self, c: char, now: Instant) -> Option<usize> {
        let index = self.type_to_find(c, now)?;
        self.select_only(index);
        Some(index)
    }

    /// 커서 항목의 선택을 뒤집는다(Ctrl+Space). 커서가 없으면 아무것도 안 한다.
    pub fn toggle_cursor_selection(&mut self) {
        if let Some(index) = self.cursor() {
            self.toggle_select(index);
        }
    }

    /// 사각형 선택(빈 곳에서 시작한 끌기)을 시작한다. `add`(Ctrl)면 지금 선택에 더하고, 아니면 사각형이 걸친 행만
    /// 선택한다. 끄는 동안 [`Tab::marquee_select`]로 걸친 행을 알리고, 놓으면 [`Tab::end_marquee`]를 부른다.
    ///
    /// 선택은 이름으로 기억하므로 끄는 동안이나 뒤에 정렬·새로 고침해도 고른 항목이 그대로 남는다.
    pub fn begin_marquee(&mut self, add: bool) {
        let base = if add {
            self.selection.clone()
        } else {
            HashSet::new()
        };
        self.marquee_base = Some(base);
    }

    /// 사각형 선택 중이면 참이다.
    pub fn is_marquee_active(&self) -> bool {
        self.marquee_base.is_some()
    }

    /// 사각형이 걸친 행으로 선택을 바꾼다: 선택은 시작할 때 남긴 선택(Ctrl이면 끌기 전 선택)과 `rows`의 행이다.
    /// `rows`는 (시작한 점 쪽 행, 끄는 점 쪽 행)이고 둘 다 든다(순서는 어느 쪽이든 된다). 목록 밖 번호는 끝 행으로
    /// 자른다. `None`이면 걸친 행이 없다(목록 아래 빈 곳만 감쌈).
    ///
    /// 커서와 범위 기준은 걸친 행 가운데 끄는 점 쪽 끝 행에 둔다(탐색기처럼 끄는 쪽 마지막 항목이 커서가 되어
    /// 이어서 Shift 클릭·Shift+화살표가 거기서 시작한다). 걸친 행이 없으면 커서와 기준은 그대로다.
    ///
    /// [`Tab::begin_marquee`] 없이 부르면 바꾸기(Ctrl 없음)로 시작한 것으로 본다.
    pub fn marquee_select(&mut self, rows: Option<(usize, usize)>) {
        let mut selection = self.marquee_base.get_or_insert_with(HashSet::new).clone();
        let len = self.len();
        if let Some((start, end)) = rows
            && len > 0
        {
            let (start, end) = (start.min(len - 1), end.min(len - 1));
            for i in start.min(end)..=start.max(end) {
                if let Some(key) = self.key(i) {
                    selection.insert(key);
                }
            }
            self.selection = selection;
            self.anchor = self.key(end);
            self.set_cursor(end);
        } else {
            self.selection = selection;
        }
    }

    /// 사각형 선택을 끝낸다. 선택은 마지막 [`Tab::marquee_select`] 그대로 남는다.
    pub fn end_marquee(&mut self) {
        self.marquee_base = None;
    }

    /// 보이는 항목을 모두 선택한다. 커서는 그대로다.
    pub fn select_all(&mut self) {
        self.selection = (0..self.len()).filter_map(|i| self.key(i)).collect();
    }

    /// 선택을 모두 푼다. 커서는 그대로다.
    pub fn clear_selection(&mut self) {
        self.selection.clear();
    }

    /// 화면이 맨 위에 보이는 행 번호.
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// 화면이 맨 위에 보이는 행 번호를 알린다. 뒤로·앞으로 기록에 함께 남는다.
    pub fn set_scroll(&mut self, scroll: usize) {
        self.scroll = scroll;
    }

    /// 글자로 이동: `c`로 시작하는 항목으로 커서를 옮기고 그 번호를 돌려준다. 없으면 커서는 그대로고 `None`이다.
    ///
    /// - [`crate::selection::TYPE_AHEAD_TIMEOUT`] 안에 이어 친 글자는 접두어로 이어 붙인다.
    ///   그보다 늦으면 새로 시작한다.
    /// - 같은 글자를 거듭 치면 그 글자로 시작하는 항목을 커서 다음부터 차례로 돈다.
    /// - 대소문자를 가리지 않는다. 한글은 IME가 조합을 끝낸 글자를 넘긴다.
    ///
    /// 시각 `now`는 부르는 쪽이 넘긴다. 선택은 바꾸지 않는다.
    pub fn type_to_find(&mut self, c: char, now: Instant) -> Option<usize> {
        let search = self.type_ahead.push(c, now);
        let len = self.len();
        if len == 0 {
            return None;
        }
        let start = match self.cursor() {
            Some(at) if search.include_cursor => at,
            Some(at) => at + 1,
            None => 0,
        };
        let found = (0..len).map(|offset| (start + offset) % len).find(|&i| {
            self.get(i)
                .is_some_and(|item| starts_with_folded(&item.name, &search.prefix))
        })?;
        self.set_cursor(found)
    }

    /// 상태 표시줄 수치: 항목 수, 선택 수, 선택한 파일 크기 합.
    pub fn status(&self) -> StatusSummary {
        // 필터 중이면 전체 수: 필터가 없을 때 보일 항목(숨김 항목 설정은 그대로) 수.
        let total_count = self.filter.as_ref().map(|_| {
            if self.shows_hidden() {
                self.list.len()
            } else {
                self.list.iter().filter(|i| !i.attributes.hidden).count()
            }
        });
        let selected_size = if self.selection.is_empty() {
            0
        } else {
            self.selected_items()
                .filter(|item| !item.is_folder)
                .map(|item| item.size)
                .sum()
        };
        StatusSummary {
            item_count: self.len(),
            total_count,
            selected_count: self.selection.len(),
            selected_size,
        }
    }
}

/// 두 경로가 같은 폴더 안인가(이름만 바꿈).
fn same_parent(a: &Path, b: &Path) -> bool {
    match (a.parent(), b.parent()) {
        (Some(a), Some(b)) => is_within(a, b) && is_within(b, a),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    //! 감시를 걸 수 없는 위치처럼 실제로 만들기 어려운 결과를 직접 넣어 본다. 실제 감시는 `tests/watch.rs`.

    use super::*;

    fn temp_dir() -> tempfile::TempDir {
        tempfile::TempDir::new().unwrap()
    }

    fn batch(tab: &Tab, changes: Vec<(OsString, Option<Item>)>, reload: bool) -> ChangeBatch {
        ChangeBatch {
            watch: tab.watching.as_ref().map(|(id, _)| *id).unwrap(),
            changes,
            reload,
        }
    }

    /// 감시가 걸린 것처럼 읽는다: 버리면 아무것도 하지 않는 감시를 붙인다.
    fn watched_tab(dir: &std::path::Path) -> Tab {
        let mut tab = Tab::new(Location::file_system(dir), Sort::default(), false);
        let request = tab.navigate(Navigation::Refresh).unwrap();
        let mut result = request.run();
        result.watch = Some(Ok(Watch::new(())));
        assert_eq!(tab.finish(result), ReadOutcome::Committed);
        tab
    }

    #[test]
    fn 감시를_걸_수_없으면_자동_새로_고침_안_됨이고_새로_고침은_된다() {
        let dir = temp_dir();
        let location = Location::file_system(dir.path());
        let mut tab = Tab::new(location.clone(), Sort::default(), false);
        assert!(matches!(tab.auto_refresh(), AutoRefresh::NotWatched));

        let request = tab.navigate(Navigation::Refresh).unwrap();
        let mut result = request.run();
        result.watch = Some(Err(WatchError::Unsupported {
            location: location.clone(),
        }));
        assert_eq!(tab.finish(result), ReadOutcome::Committed);
        let AutoRefresh::Off(error) = tab.auto_refresh() else {
            panic!("자동 새로 고침 안 됨이어야 한다");
        };
        assert_eq!(error.location(), &location);
        assert!(error.to_string().contains("감시할 수 없습니다"));
        assert_eq!(tab.watched_location(), None);

        // 새로 고침(F5)은 그대로 된다. 감시 없이 같은 위치를 다시 읽으면 상태는 그대로다.
        std::fs::write(dir.path().join("새 파일"), b"x").unwrap();
        let request = tab.navigate(Navigation::Refresh).unwrap();
        assert_eq!(tab.finish(request.run()), ReadOutcome::Committed);
        assert_eq!(tab.len(), 1);
        assert!(matches!(tab.auto_refresh(), AutoRefresh::Off(_)));

        // 다시 감시가 걸리면 켜진다.
        let request = tab.navigate(Navigation::Refresh).unwrap();
        let mut result = request.run();
        result.watch = Some(Ok(Watch::new(())));
        tab.finish(result);
        assert!(matches!(tab.auto_refresh(), AutoRefresh::On));
    }

    #[test]
    fn 지금_감시가_아닌_변경은_버리고_넘침은_다시_읽기를_요구한다() {
        let dir = temp_dir();
        std::fs::write(dir.path().join("a"), b"a").unwrap();
        let mut tab = watched_tab(dir.path());
        let mut stale = batch(&tab, vec![("a".into(), None)], false);
        stale.watch = RequestId(stale.watch.0 + 100);
        assert_eq!(tab.apply_changes(stale), ChangeOutcome::Stale);
        assert_eq!(tab.len(), 1);

        let overflow = batch(&tab, Vec::new(), true);
        assert_eq!(tab.apply_changes(overflow), ChangeOutcome::Reload);
        let request = tab.reload().expect("기다리는 읽기가 없으면 다시 읽는다");
        // 기다리는 읽기가 있으면 가로채지 않는다.
        assert!(tab.reload().is_none());
        assert_eq!(tab.finish(request.run()), ReadOutcome::Committed);
        assert_eq!(tab.location(), &Location::file_system(dir.path()));
        assert!(tab.relocation().is_none());

        // 감시 없이 다른 위치로 옮기면 앞 위치의 감시가 풀린다.
        let mut tab = watched_tab(dir.path());
        let other = temp_dir();
        let request = tab
            .navigate(Navigation::Go(Location::file_system(other.path())))
            .unwrap();
        tab.finish(request.run());
        assert_eq!(tab.watched_location(), None);
    }

    #[test]
    fn 바뀐_항목만_넣고_빼고_다시_정렬한다() {
        let dir = temp_dir();
        for name in ["b", "d"] {
            std::fs::write(dir.path().join(name), name).unwrap();
        }
        let mut tab = watched_tab(dir.path());
        let item = |name: &str, size| Item {
            name: name.into(),
            is_folder: false,
            size,
            modified: std::time::SystemTime::UNIX_EPOCH,
            attributes: Default::default(),
        };
        let changes = vec![
            ("a".into(), Some(item("a", 1))),
            ("d".into(), None),
            ("b".into(), Some(item("b", 7))),
        ];
        let applied = batch(&tab, changes, false);
        assert_eq!(tab.apply_changes(applied), ChangeOutcome::Applied);
        let names: Vec<_> = tab.items().map(|i| i.name.clone()).collect();
        assert_eq!(names, ["a", "b"]);
        assert_eq!(tab.get(1).unwrap().size, 7);
    }
}

/// 즉시 검색 결과 위치인가.
fn is_instant_location(location: &Location) -> bool {
    location
        .search_scope()
        .is_some_and(|scope| matches!(scope.kind, SearchKind::Instant { .. }))
}

/// [`Tab::retarget_instant`]의 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retarget {
    /// 즉시 검색 결과 위치를 제자리에서 바꿨다. 질의를 다시 보내야 한다([`Tab::instant_needs_query`]).
    Replaced,
    /// 즉시 검색 결과 위치로 가는 이동의 목적지를 바꿨다. 이동이 확정되면 질의를 보낸다.
    Pending,
    /// 이미 그 위치다.
    Unchanged,
    /// 즉시 검색 중이 아니다(부르는 쪽이 이동한다).
    NotSearching,
}

/// [`Tab::instant_reply`]의 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstantReply {
    /// 이 탭이 기다리는 요청의 응답이 아니라 버렸다.
    Ignored,
    /// 응답을 반영했다(결과·전체 개수·서비스 없음 등이 바뀜).
    Applied,
    /// 인덱스 변경을 확인했지만 보이는 것은 바뀌지 않았다(인덱스가 그대로이거나 다시 받은 결과가 같음).
    Unchanged,
    /// 질의를 다시 보내야 한다: 이어 받기가 무효라 받은 만큼 조용히 다시 받거나, 범위 볼륨의 인덱스를 확인했다.
    Requery,
    /// 서비스를 쓸 수 없어 실시간 탐색으로 강등했다([`Tab::demoted_search_due`]를 앱이 시작한다).
    Demoted,
}
