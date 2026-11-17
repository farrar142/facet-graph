//! facet 측정 도구.
//!
//! - 시험 폴더 생성: 실제 폴더와 비슷한 모양(한글·이모지·긴 이름, 폴더와 파일이 섞임)의
//!   항목을 정해진 수만큼 만든다. 같은 수로 다시 돌리면 결과가 같다.
//! - 코어 벤치마크: 「위치의 항목 목록 읽기」(파일 시스템 공급자의 [`Provider::list`]), 「읽기+정렬」,
//!   「정렬 바꾸기」([`DetailsList::set_sort`])를 재고 각각 상한([`LIST_LIMIT`]·[`LIST_SORT_LIMIT`]·
//!   [`RESORT_LIMIT`])과 비교한다. `cargo xtask ci`가 이것을 부른다.
//! - 인덱스 벤치마크: 시험 폴더와 같은 이름의 합성 인덱스([`synthetic_index`])를 만들어
//!   메모리([`INDEX_MEMORY_LIMIT`])와 부분 일치 질의 시간([`QUERY_LIMIT`])을 잰다. 인덱서 서비스가
//!   파이프로 받은 질의처럼 폴더 권한으로 거르는 질의도 권한 검사 기억이 찬 상태로 같은 상한에
//!   견준다([`measure_filtered_query`]). `cargo xtask ci`가 이것도 부른다.
//! - MFT 파싱 벤치마크: 저장소의 MFT 조각을 되풀이한 덩어리([`mft_image`])를 인덱스로 만드는
//!   시간([`MFT_PARSE_LIMIT`])을 잰다. `cargo xtask ci`가 이것도 부른다.
//! - 세션 벤치마크: 탭 100개 세션 파일을 읽고 해석하는 시간([`SESSION_PARSE_LIMIT`])을 잰다
//!   ([`measure_session`]). `cargo xtask ci`가 이것도 부른다.
//! - 클립보드 형식 벤치마크: 파일 1만 개 목록을 `CF_HDROP` 바이트로 만들고 읽는 시간([`CLIPBOARD_LIMIT`])을 잰다
//!   ([`measure_clipboard`]). `cargo xtask ci`가 이것도 부른다.
//! - 일괄 이름 바꾸기 미리 보기 벤치마크: 선택 1만 개의 미리 보기 계산 시간([`BATCH_RENAME_LIMIT`])을 잰다([`measure_batch_rename`]).
//! - 오른쪽 메뉴 구성 벤치마크: 선택 1만 개에서 메뉴를 만드는 시간([`MENU_LIMIT`])을 잰다([`measure_menu`]).
//!   `cargo xtask ci`가 이것도 부른다.
//! - 질의 문법 해석 벤치마크: 연산자·조건을 고루 쓴 긴 질의 하나를 해석하는 시간([`QUERY_SYNTAX_LIMIT`])을 잰다([`measure_query_syntax`]).
//! - 필터 벤치마크: 10만 항목 목록에서 필터 한 글자·두 글자를 정하는 시간([`FILTER_LIMIT`])을 잰다([`measure_filter`]).
//!   `cargo xtask ci`가 이것도 부른다.
//! - 실시간 탐색 벤치마크: 10만 항목 시험 나무([`make_test_tree`])를 실시간 탐색으로 끝까지 훑는 시간
//!   ([`SEARCH_LIMIT`])을 잰다([`measure_live_search`]). `cargo xtask ci`가 이것도 부른다.
//! - 내용 검색 벤치마크: 본문 128MB 시험 나무를 내용 검색으로 읽는 처리량([`content::CONTENT_THROUGHPUT_LIMIT`])을 잰다
//!   ([`content::measure_content_search`]). `cargo xtask ci`가 이것도 부른다.
//! - 복사 벤치마크: 작업 대기열의 복사 작업으로 256MiB 파일 하나의 처리량([`copy::COPY_THROUGHPUT_LIMIT`])과
//!   4KiB 파일 2천 개가 든 폴더의 복사 시간([`copy::COPY_SMALL_LIMIT`])을 잰다. `cargo xtask ci`가 이것도 부른다.
//!   같은 복사를 셸의 `IFileOperation`으로도 해 견주는 탐색기 비교 측정([`copy::compare`])은 사람이 잰다.

pub mod content;
pub mod copy;
pub mod delete;
pub mod screen;

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use facet_core::details::{Column, DetailsList, Direction, Sort};
use facet_core::{FileSystemProvider, ListError, Location, Provider};

/// 파일 시스템 공급자로 실제 폴더 `path`의 항목 목록을 읽는다.
fn list_folder(path: &Path) -> Result<Vec<facet_core::Item>, ListError> {
    FileSystemProvider.list(&Location::file_system(path))
}

/// 벤치마크 폴더의 기본 항목 수.
pub const DEFAULT_COUNT: usize = 100_000;

/// 10만 항목 목록 읽기의 속도 기준(로컬, 릴리스 빌드). spec #1의 통과 기준이다.
pub const LIST_TARGET: Duration = Duration::from_millis(100);

/// CI가 실패시키는 10만 항목 목록 읽기의 상한: 로컬 기준 [`LIST_TARGET`]의 10배.
///
/// 공유 러너는 디스크·CPU 편차가 커서 로컬 기준을 그대로 쓰면 이유 없는 실패가 난다.
/// 이 상한은 알고리즘이 한 단계 나빠지는 실수(예: 항목마다 시스템 호출을 더 하거나
/// 제곱 시간이 드는 처리)를 잡는 용도이지 미세한 회귀를 잡는 용도가 아니다
/// (`docs/roadmap.md`의 「속도는 두 군데서 지킨다」).
pub const LIST_LIMIT: Duration = Duration::from_millis(1000);

/// 시험 폴더의 항목 하나의 모양.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestEntry {
    pub name: String,
    pub is_folder: bool,
    /// 파일의 바이트 크기. 폴더는 0.
    pub size: u64,
}

/// 시험 폴더의 `index`번째 항목. 같은 번호는 늘 같은 항목이다.
///
/// 열 개마다 두 개가 폴더이고, 나머지는 확장자·길이·문자가 다른 파일이다.
pub fn test_folder_entry(index: usize) -> TestEntry {
    let i = index;
    let (name, is_folder) = match i % 10 {
        0 => (format!("보고서 {i:06}.txt"), false),
        1 => (format!("photo_{i:06} 🎉.jpg"), false),
        2 => (
            format!("{}{i:06}.md", "아주 긴 이름을 가진 회의록 문서 ".repeat(6)),
            false,
        ),
        3 => (format!("data-{i:06}.bin"), false),
        4 => (format!("폴더 {i:06}"), true),
        5 => (format!("README {i:06}"), false),
        6 => (format!("🦀🚀 이모지 {i:06}.rs"), false),
        7 => (
            format!("Quarterly Report {i:06} (final) - copy.xlsx"),
            false,
        ),
        8 => (format!("archive.{i:06}.tar.gz"), false),
        _ => (format!("📁 프로젝트 {i:06}"), true),
    };
    let size = if is_folder {
        0
    } else {
        (i as u64).wrapping_mul(37) % 1024
    };
    TestEntry {
        name,
        is_folder,
        size,
    }
}

/// [`make_test_folder`]의 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MakeReport {
    /// 새로 만든 항목 수.
    pub created: usize,
    /// 이미 있던 항목 수(크기를 고친 파일 포함).
    pub kept: usize,
}

/// 시험 폴더를 만들지 못한 까닭.
#[derive(Debug)]
pub enum TestFolderError {
    /// 폴더 안에 시험 폴더의 항목이 아닌 것이 있다. 사용자 파일을 지우지 않으려고 멈춘다.
    Unexpected { path: PathBuf },
    /// 같은 이름이 있지만 폴더/파일 종류가 다르다.
    WrongKind { path: PathBuf },
    /// 입출력 오류.
    Io { path: PathBuf, source: io::Error },
}

impl fmt::Display for TestFolderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unexpected { path } => write!(
                f,
                "시험 폴더의 항목이 아닌 것이 있어 멈춥니다(빈 폴더나 새 경로를 주세요): {}",
                path.display()
            ),
            Self::WrongKind { path } => write!(
                f,
                "이름은 같지만 폴더/파일 종류가 다른 항목이 있습니다: {}",
                path.display()
            ),
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
        }
    }
}

impl std::error::Error for TestFolderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub(crate) fn io_err(path: &Path) -> impl FnOnce(io::Error) -> TestFolderError + '_ {
    move |source| TestFolderError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// `path`에 `count`개 항목([`test_folder_entry`]의 0..count번)이 든 시험 폴더를 만든다.
///
/// 폴더가 없으면 만든다. 이미 있으면 빠진 항목만 만들고 크기가 다른 파일은 고친다.
/// 그래서 같은 `count`로 다시 돌리면 결과(이름·종류·크기)가 같다.
/// 시험 폴더의 항목이 아닌 것이 있으면 아무것도 지우지 않고 오류를 낸다.
pub fn make_test_folder(path: &Path, count: usize) -> Result<MakeReport, TestFolderError> {
    fs::create_dir_all(path).map_err(io_err(path))?;

    let mut wanted: BTreeMap<String, TestEntry> = (0..count)
        .map(test_folder_entry)
        .map(|e| (e.name.clone(), e))
        .collect();

    // 먼저 전부 확인한 뒤에야 쓴다: 모르는 항목이 있으면 아무것도 바꾸지 않는다.
    let existing = list_folder(path).map_err(|e| TestFolderError::Io {
        path: path.to_path_buf(),
        source: io::Error::other(e.to_string()),
    })?;
    let mut to_resize = Vec::new();
    let mut kept = 0;
    for item in existing {
        let item_path = path.join(&item.name);
        let Some(entry) = item.name.to_str().and_then(|n| wanted.remove(n)) else {
            return Err(TestFolderError::Unexpected { path: item_path });
        };
        if entry.is_folder != item.is_folder {
            return Err(TestFolderError::WrongKind { path: item_path });
        }
        if !entry.is_folder && entry.size != item.size {
            to_resize.push((item_path, entry.size));
        }
        kept += 1;
    }

    for (item_path, size) in to_resize {
        OpenOptions::new()
            .write(true)
            .open(&item_path)
            .and_then(|f| f.set_len(size))
            .map_err(io_err(&item_path))?;
    }

    let mut created = 0;
    for entry in wanted.into_values() {
        let item_path = path.join(&entry.name);
        let made = if entry.is_folder {
            fs::create_dir(&item_path)
        } else {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&item_path)
                .and_then(|f| f.set_len(entry.size))
        };
        match made {
            Ok(()) => created += 1,
            // 목록을 읽은 뒤 다른 프로세스(같은 폴더를 채우는 다른 실행, 끝나지 않고 남은 앞선 실행)가
            // 먼저 만들었다. 이미 있는 항목으로 보고 확인한다.
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                keep_existing(&item_path, &entry)?;
                kept += 1;
            }
            Err(e) => return Err(io_err(&item_path)(e)),
        }
    }

    Ok(MakeReport { created, kept })
}

/// 만들려는 순간 이미 있던 항목을 확인한다. 종류가 다르면 거절하고 파일 크기가 다르면 고친다.
fn keep_existing(item_path: &Path, entry: &TestEntry) -> Result<(), TestFolderError> {
    let metadata = fs::symlink_metadata(item_path).map_err(io_err(item_path))?;
    if metadata.is_dir() != entry.is_folder {
        return Err(TestFolderError::WrongKind {
            path: item_path.to_path_buf(),
        });
    }
    if !entry.is_folder && metadata.len() != entry.size {
        OpenOptions::new()
            .write(true)
            .open(item_path)
            .and_then(|f| f.set_len(entry.size))
            .map_err(io_err(item_path))?;
    }
    Ok(())
}

/// 목록 읽기 측정 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Measurement {
    /// 읽은 항목 수.
    pub item_count: usize,
    /// 회차별 걸린 시간.
    pub times: Vec<Duration>,
}

impl Measurement {
    /// 회차별 시간의 중앙값(짝수 개면 가운데 둘 중 큰 쪽).
    pub fn median(&self) -> Duration {
        let mut sorted = self.times.clone();
        sorted.sort();
        sorted.get(sorted.len() / 2).copied().unwrap_or_default()
    }

    /// 가장 오래 걸린 회차.
    pub fn max(&self) -> Duration {
        self.times.iter().max().copied().unwrap_or_default()
    }
}

/// `path`의 목록 읽기를 `runs`번 잰다. 재기 전에 한 번 읽어 캐시를 데운다.
pub fn measure_list(path: &Path, runs: usize) -> Result<Measurement, ListError> {
    let mut item_count = list_folder(path)?.len();
    let mut times = Vec::with_capacity(runs);
    for _ in 0..runs {
        let start = Instant::now();
        let items = list_folder(path)?;
        times.push(start.elapsed());
        item_count = items.len();
    }
    Ok(Measurement { item_count, times })
}

/// 10만 항목 「읽기+정렬」(목록을 읽어 기본 정렬인 이름 오름차순으로 늘어놓기)의 속도 기준(로컬, 릴리스 빌드).
///
/// spec #16의 「10만 항목 폴더 열기부터 첫 화면까지 150ms 미만」에서 코어가 맡는 몫이다. 화면을 그리는 시간은
/// 측정 명령(`cargo xtask measure`)이 따로 잰다.
pub const LIST_SORT_TARGET: Duration = Duration::from_millis(150);

/// CI가 실패시키는 「읽기+정렬」 상한: 로컬 기준 [`LIST_SORT_TARGET`]의 10배.
/// [`LIST_LIMIT`]과 같은 까닭으로 넉넉하다(알고리즘이 한 단계 나빠지는 실수만 잡는다).
pub const LIST_SORT_LIMIT: Duration = Duration::from_millis(1500);

/// 10만 항목 「정렬 바꾸기」(이미 읽은 목록을 다른 열·방향으로 다시 정렬)의 속도 기준(로컬, 릴리스 빌드). spec #16.
pub const RESORT_TARGET: Duration = Duration::from_millis(50);

/// CI가 실패시키는 「정렬 바꾸기」 상한: 로컬 기준 [`RESORT_TARGET`]의 10배.
pub const RESORT_LIMIT: Duration = Duration::from_millis(500);

/// 정렬 바꾸기 벤치마크가 회차마다 차례로 바꾸는 정렬. 이웃한 두 정렬이 늘 다르다.
///
/// 이름 오름 → 내림처럼 순서를 뒤집기만 하는 바꾸기는 정렬 알고리즘이 거꾸로 된 덩어리를 알아보고
/// 거의 공짜로 끝내므로, 열을 바꾸는 쪽을 섞어 실제 비교가 일어나게 한다.
pub const RESORT_CYCLE: [Sort; 5] = [
    Sort {
        column: Column::Size,
        direction: Direction::Descending,
    },
    Sort {
        column: Column::Type,
        direction: Direction::Ascending,
    },
    Sort {
        column: Column::Modified,
        direction: Direction::Descending,
    },
    Sort {
        column: Column::Name,
        direction: Direction::Descending,
    },
    Sort {
        column: Column::Size,
        direction: Direction::Ascending,
    },
];

/// `path`의 「읽기+정렬」(목록 읽기와 기본 정렬)을 `runs`번 잰다. 재기 전에 한 번 해서 캐시를 데운다.
pub fn measure_list_sorted(path: &Path, runs: usize) -> Result<Measurement, ListError> {
    let read_sorted = || -> Result<DetailsList, ListError> {
        Ok(DetailsList::new(list_folder(path)?, Sort::default()))
    };
    let mut item_count = read_sorted()?.len();
    let mut times = Vec::with_capacity(runs);
    for _ in 0..runs {
        let start = Instant::now();
        let list = read_sorted()?;
        times.push(start.elapsed());
        item_count = list.len();
    }
    Ok(Measurement { item_count, times })
}

/// `path`의 목록을 한 번 읽어 두고 「정렬 바꾸기」를 `runs`번 잰다. 회차마다 [`RESORT_CYCLE`]의 다음
/// 정렬로 바꾸며, 재기 전에 그 순환을 한 바퀴 돌려 데운다. 파일 시스템은 다시 읽지 않는다.
pub fn measure_resort(path: &Path, runs: usize) -> Result<Measurement, ListError> {
    let mut list = DetailsList::new(list_folder(path)?, Sort::default());
    for sort in RESORT_CYCLE {
        list.set_sort(sort);
    }
    let mut times = Vec::with_capacity(runs);
    for sort in RESORT_CYCLE.iter().cycle().take(runs) {
        let start = Instant::now();
        list.set_sort(*sort);
        times.push(start.elapsed());
    }
    Ok(Measurement {
        item_count: list.len(),
        times,
    })
}

/// 목록 읽기 중앙값이 `limit`을 넘으면 넘은 수치를 담은 메시지를 돌려준다.
pub fn check_limit(m: &Measurement, limit: Duration) -> Result<(), String> {
    check_median("목록 읽기", m, limit)
}

/// `what`의 중앙값이 `limit`을 넘으면 무엇이 얼마나 넘었는지 담은 메시지를 돌려준다.
pub fn check_median(what: &str, m: &Measurement, limit: Duration) -> Result<(), String> {
    let median = m.median();
    if median <= limit {
        Ok(())
    } else {
        Err(format!(
            "{what} 상한 초과: {}항목 중앙값 {}ms > 상한 {}ms (최악 {}ms)",
            m.item_count,
            median.as_millis(),
            limit.as_millis(),
            m.max().as_millis()
        ))
    }
}

/// 합성 인덱스의 기본 항목 수.
pub const INDEX_COUNT: usize = 1_000_000;

/// 합성 인덱스 100만 개 부분 일치 질의(결과 100개 제한)의 속도 기준(로컬, 릴리스 빌드). spec #22.
pub const QUERY_TARGET: Duration = Duration::from_millis(30);

/// CI가 실패시키는 질의 시간 상한: 로컬 기준 [`QUERY_TARGET`]의 10배.
/// [`LIST_LIMIT`]과 같은 까닭으로 넉넉하다(알고리즘이 한 단계 나빠지는 실수만 잡는다).
pub const QUERY_LIMIT: Duration = Duration::from_millis(300);

/// 합성 인덱스 100만 개가 쓸 수 있는 메모리 상한(spec #22의 「150MB 미만」).
/// 10진 MB로 센다(150 × 10⁶바이트). MiB보다 작아 더 엄격하다.
pub const INDEX_MEMORY_LIMIT: usize = 150_000_000;

/// 벤치마크 질의의 찾을 조각. 대문자로 써서 대소문자 접기를 거치게 하고,
/// 100만 개 중 마지막 쪽 항목 하나(`Quarterly Report 999997 …`)만 맞게 골라
/// 결과 100개 제한에 일찍 멈추지 않고 인덱스 전체를 훑게 한다(나쁜 쪽의 경우).
pub const BENCH_QUERY: &str = "REPORT 99999";

/// 벤치마크 질의의 결과 수 제한.
pub const BENCH_RESULT_LIMIT: usize = 100;

/// 정렬 벤치마크 질의의 찾을 조각. 열 개 중 하나(`Quarterly Report …`)가 맞아 100만 개에서
/// 결과가 10만 개다. 결과 100개 제한 질의가 결과 전체를 매번 다 정렬하면 상한을 넘게 한다.
pub const BENCH_SORTED_QUERY: &str = "REPORT";

/// 정렬 벤치마크 질의의 정렬 기준. 비교가 가장 비싼 경로 정렬(내림)이다.
pub const BENCH_SORT: (facet_index::SortKey, facet_index::Order) =
    (facet_index::SortKey::Path, facet_index::Order::Descending);

/// 합성 인덱스의 볼륨 루트 경로와 루트 번호(NTFS 루트 폴더의 MFT 레코드 번호).
pub const SYNTHETIC_ROOT: &str = r"X:\";
pub const SYNTHETIC_ROOT_NUMBER: u64 = 5;

/// `index`번 항목의 항목 번호. NTFS의 예약 레코드(0..16)를 피한다.
fn synthetic_number(index: usize) -> u64 {
    16 + index as u64
}

/// `index`번 항목이 들어 있는 폴더의 번호.
///
/// 열 개씩 한 묶음이고 묶음마다 폴더가 둘(4번째, 9번째)이다. 첫 묶음은 볼륨 루트 아래,
/// 그 뒤 `k`번 묶음은 `k / 5`번 묶음의 폴더 하나 아래에 둔다. 폴더마다 약 25개 항목이
/// 들고 100만 개에서 깊이는 9 안팎이다.
fn synthetic_parent(index: usize) -> u64 {
    let group = index / 10;
    if group == 0 {
        return SYNTHETIC_ROOT_NUMBER;
    }
    let slot = if group.is_multiple_of(2) { 4 } else { 9 };
    synthetic_number(group / 5 * 10 + slot)
}

/// 항목 `count`개의 합성 인덱스. 이름은 시험 폴더와 같다([`test_folder_entry`]의 0..count번):
/// 한글·이모지·긴 이름이 섞이고 열 개 중 두 개가 폴더다. 같은 수면 늘 같은 인덱스다.
pub fn synthetic_index(count: usize) -> facet_index::Index {
    let mut index = facet_index::Index::new(SYNTHETIC_ROOT, SYNTHETIC_ROOT_NUMBER);
    for i in 0..count {
        let entry = test_folder_entry(i);
        let item = facet_index::NewItem {
            number: synthetic_number(i),
            parent: synthetic_parent(i),
            name: &entry.name,
            size: entry.size,
            // 2022년 무렵부터 1초씩 늦어지는 FILETIME.
            modified: 133_000_000_000_000_000 + i as u64 * 10_000_000,
            attributes: if entry.is_folder {
                facet_index::ATTRIBUTE_DIRECTORY
            } else {
                0x20 // FILE_ATTRIBUTE_ARCHIVE
            },
        };
        index
            .insert(item)
            .expect("시험 폴더의 이름은 인덱스에 넣을 수 있다");
    }
    index
}

/// `index`에 `query`를 `runs`번 재어 돌린다. 재기 전에 한 번 돌려 데운다.
/// [`Measurement::item_count`]는 결과 수다.
pub fn measure_query(
    index: &facet_index::Index,
    query: &facet_index::Query,
    runs: usize,
) -> Measurement {
    let mut item_count = index.query(query).len();
    let mut times = Vec::with_capacity(runs);
    for _ in 0..runs {
        let start = Instant::now();
        let hits = index.query(query);
        times.push(start.elapsed());
        item_count = hits.len();
    }
    Measurement { item_count, times }
}

/// 권한 거르기 벤치마크에서 합성 인덱스를 담는 볼륨 이름.
pub const SYNTHETIC_VOLUME: &str = "X:";

/// 권한 거르기 벤치마크의 가짜 사용자. 폴더 이름의 번호가 50으로 나눠 24가 남는 폴더
/// (`폴더 …024`, `…074`, …, 「폴더」 다섯 개 중 하나)의 목록을 읽을 수 없다. 그 아래 항목은 모두
/// 걸러지고, 걸러진 만큼 뒤 후보로 채운다.
#[derive(Debug, Default)]
pub struct BenchCaller {
    asked: std::sync::atomic::AtomicUsize,
}

impl BenchCaller {
    /// 권한 검사 기억에 없어 물어본 폴더 수.
    pub fn asked(&self) -> usize {
        self.asked.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl facet_indexer::access::Caller for BenchCaller {
    fn user(&self) -> Option<String> {
        Some("S-1-5-21-0-0-0-1001".to_owned())
    }

    fn can_list(&self, folder: &str) -> bool {
        self.asked
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        !(folder.ends_with("24") || folder.ends_with("74"))
    }
}

/// 권한으로 거르는 질의(인덱서 서비스가 파이프로 받은 질의와 같은 길, `Volumes::search_as`)를
/// `runs`번 잰다. 재기 전에 한 번 돌려 데우며, 그때 권한 검사 기억이 찬다. 잴 때 기억에 없어
/// 다시 물은 폴더가 있으면 실패한다(「기억이 찬 상태」를 재지 못한 것이다).
/// [`Measurement::item_count`]는 결과 수다.
pub fn measure_filtered_query(
    volumes: &facet_indexer::volumes::Volumes,
    caller: &BenchCaller,
    query: &facet_protocol::QueryMessage,
    runs: usize,
) -> Result<Measurement, String> {
    let cancel = facet_indexer::session::Cancel::new();
    let run = || {
        volumes
            .search_as(query, caller, &cancel)
            .map(|page| page.hits.len())
            .map_err(|e| e.message)
    };
    let mut item_count = run()?;
    let warmed = caller.asked();
    let mut times = Vec::with_capacity(runs);
    for _ in 0..runs {
        let start = Instant::now();
        item_count = run()?;
        times.push(start.elapsed());
    }
    if caller.asked() != warmed {
        return Err(format!(
            "권한 검사 기억이 찬 뒤에도 폴더 {}개를 다시 물었습니다",
            caller.asked() - warmed
        ));
    }
    Ok(Measurement { item_count, times })
}

/// 질의 중앙값이 `limit`을 넘으면 넘은 수치를 담은 메시지를 돌려준다.
pub fn check_query_limit(m: &Measurement, limit: Duration) -> Result<(), String> {
    let median = m.median();
    if median <= limit {
        Ok(())
    } else {
        Err(format!(
            "인덱스 질의 상한 초과: 중앙값 {}ms > 상한 {}ms (최악 {}ms, 결과 {}개)",
            median.as_millis(),
            limit.as_millis(),
            m.max().as_millis(),
            m.item_count
        ))
    }
}

/// 인덱스 메모리가 `limit` 바이트 이상이면 넘은 수치를 담은 메시지를 돌려준다.
pub fn check_memory(bytes: usize, limit: usize) -> Result<(), String> {
    if bytes < limit {
        Ok(())
    } else {
        Err(format!(
            "인덱스 메모리 상한 초과: {:.1}MB ≥ 상한 {:.1}MB",
            bytes as f64 / 1e6,
            limit as f64 / 1e6
        ))
    }
}

/// MFT 파싱 벤치마크의 재료: 저장소의 MFT 조각(`testdata/mft/ntfs-basic.mft`, 레코드 256개).
pub const MFT_FIXTURE: &[u8] = include_bytes!("../../../testdata/mft/ntfs-basic.mft");

/// MFT 조각의 레코드 크기.
pub const MFT_RECORD_SIZE: usize = 1024;

/// MFT 파싱 벤치마크의 기본 레코드 수. 항목 100만 개 볼륨의 MFT 크기 무렵이다.
pub const MFT_RECORDS: usize = 1_000_000;

/// MFT 레코드 100만 개(1GB)를 인덱스로 만드는 파싱 시간의 기준(로컬, 릴리스 빌드).
///
/// spec #22의 기준은 「항목 100만 개 볼륨의 첫 인덱스 10초 미만」이고, 그 안에 MFT 읽기·파싱·저장이
/// 모두 든다. MFT 100만 레코드는 약 1GB다. 이를 디스크에서 순서대로 읽는 데 느린 쪽(초당 150MB 안팎의
/// 하드 디스크)으로 약 7초, 인덱스 저장과 서비스 준비에 약 1초를 잡으면 파싱 몫은 2초가 남는다.
/// SSD에서는 읽기가 1~2초라 여유가 더 있다.
pub const MFT_PARSE_TARGET: Duration = Duration::from_millis(2000);

/// CI가 실패시키는 MFT 파싱 시간 상한: 로컬 기준 [`MFT_PARSE_TARGET`]의 10배.
/// [`LIST_LIMIT`]과 같은 까닭으로 넉넉하다(알고리즘이 한 단계 나빠지는 실수만 잡는다).
pub const MFT_PARSE_LIMIT: Duration = Duration::from_millis(20_000);

/// MFT 조각을 `records`개 레코드가 되도록 되풀이해 이어 붙인 MFT 덩어리.
///
/// 레코드마다 고정값이 맞으므로 모두 정상 레코드로 읽힌다. 되풀이한 레코드의 부모 참조는 첫 조각의
/// 폴더를 가리키므로 항목 대부분이 경로를 갖는다.
pub fn mft_image(records: usize) -> Vec<u8> {
    let len = records * MFT_RECORD_SIZE;
    MFT_FIXTURE.iter().copied().cycle().take(len).collect()
}

/// MFT 덩어리 `image`를 인덱스로 만드는 시간을 `runs`번 잰다. 재기 전에 한 번 돌려 데운다.
/// [`Measurement::item_count`]는 만든 인덱스의 항목 수다.
pub fn measure_mft(image: &[u8], runs: usize) -> Result<Measurement, String> {
    let parse = || {
        facet_index::Index::from_mft(SYNTHETIC_ROOT, MFT_RECORD_SIZE, image)
            .map_err(|e| e.to_string())
    };
    let (index, report) = parse()?;
    if report.skipped != 0 {
        return Err(format!(
            "MFT 조각에서 건너뛴 레코드가 있습니다: {}개",
            report.skipped
        ));
    }
    let mut item_count = index.len();
    drop(index);
    let mut times = Vec::with_capacity(runs);
    for _ in 0..runs {
        let start = Instant::now();
        let (index, _) = parse()?;
        times.push(start.elapsed());
        item_count = index.len();
    }
    Ok(Measurement { item_count, times })
}

/// MFT 파싱 중앙값이 `limit`을 넘으면 넘은 수치를 담은 메시지를 돌려준다.
pub fn check_mft_limit(m: &Measurement, records: usize, limit: Duration) -> Result<(), String> {
    let median = m.median();
    if median <= limit {
        Ok(())
    } else {
        Err(format!(
            "MFT 파싱 상한 초과: 레코드 {records}개 중앙값 {}ms > 상한 {}ms (최악 {}ms)",
            median.as_millis(),
            limit.as_millis(),
            m.max().as_millis()
        ))
    }
}

/// 세션 벤치마크의 탭 수: 창 다섯 개 × 패널 둘 × 패널마다 탭 열 개.
pub const SESSION_TABS: usize = 100;

/// 탭 100개 세션 파일을 읽고 해석하는 시간의 기준(로컬, 릴리스 빌드).
///
/// 세션 복원은 콜드 스타트(200ms 미만)에 끼어드는 일이라 파일 읽기와 JSON 해석이 한 자릿수 ms를
/// 넘으면 안 된다. 탭 100개 세션 파일은 수십 KB라 5ms면 넉넉하다.
pub const SESSION_PARSE_TARGET: Duration = Duration::from_millis(5);

/// CI가 실패시키는 세션 읽기·해석 시간 상한: 로컬 기준 [`SESSION_PARSE_TARGET`]의 10배.
/// [`LIST_LIMIT`]과 같은 까닭으로 넉넉하다(알고리즘이 한 단계 나빠지는 실수만 잡는다).
pub const SESSION_PARSE_LIMIT: Duration = Duration::from_millis(50);

/// 창 `windows`개, 창마다 패널 둘, 패널마다 탭 `tabs_per_panel`개인 세션. 탭의 위치는 `location(창, 패널, 탭)`이
/// 정하고, 정렬은 열과 방향을 돌려 가며 섞는다. 창마다 활성 패널·활성 탭은 오른쪽·마지막 탭이다.
pub fn session_fixture(
    windows: usize,
    tabs_per_panel: usize,
    mut location: impl FnMut(usize, usize, usize) -> facet_core::Location,
) -> facet_core::session::Session {
    use facet_core::session::{PanelSession, Session, TabSession, WindowSession};
    let columns = [Column::Name, Column::Modified, Column::Type, Column::Size];
    let windows = (0..windows)
        .map(|w| WindowSession {
            panels: (0..2)
                .map(|p| PanelSession {
                    tabs: (0..tabs_per_panel)
                        .map(|t| TabSession {
                            location: location(w, p, t),
                            sort: Sort {
                                column: columns[(w + p + t) % columns.len()],
                                direction: if t % 2 == 0 {
                                    Direction::Ascending
                                } else {
                                    Direction::Descending
                                },
                            },
                        })
                        .collect(),
                    active_tab: tabs_per_panel.saturating_sub(1),
                })
                .collect(),
            active_panel: 1,
            split_ratio: 0.5,
            placement: None,
        })
        .collect();
    Session {
        windows,
        active_window: 0,
        recent_targets: Vec::new(),
    }
}

/// 벤치마크 세션의 탭 위치: 긴 경로와 한글 경로를 섞는다.
fn session_bench_location(w: usize, p: usize, t: usize) -> facet_core::Location {
    let path = match (w + p + t) % 3 {
        0 => format!(r"C:\사용자\홍길동\문서\프로젝트 {w}\보고서 {p}\분기별 회의록 {t}"),
        1 => format!(
            r"D:\{}\항목 {w}-{p}-{t}",
            "아주 긴 이름을 가진 폴더 ".repeat(8)
        ),
        _ => format!(r"E:\work\src\crate-{w}\module-{p}\tests\case-{t:03}"),
    };
    facet_core::Location::file_system(path)
}

/// 탭 [`SESSION_TABS`]개 세션 파일을 `runs`번 읽고 해석하는 시간을 잰다(`session::load`: 파일 읽기와 JSON 해석).
/// 재기 전에 한 번 읽어 데운다. 읽은 세션이 쓴 것과 다르면 실패한다.
/// [`Measurement::item_count`]는 읽은 탭 수다.
pub fn measure_session(runs: usize) -> Result<Measurement, String> {
    let session = session_fixture(5, SESSION_TABS / 10, session_bench_location);
    let dir = std::env::temp_dir().join(format!("facet-bench-session-{}", std::process::id()));
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("session.json");
    let result = (|| {
        facet_core::session::save(&path, &session).map_err(|e| e.to_string())?;
        let read = || match facet_core::session::load(&path) {
            facet_core::session::Loaded::Restored(read) => Ok(read),
            other => Err(format!("세션을 읽지 못했습니다: {other:?}")),
        };
        if read()? != session {
            return Err("읽은 세션이 쓴 세션과 다릅니다".to_owned());
        }
        let mut times = Vec::with_capacity(runs);
        let mut item_count = 0;
        for _ in 0..runs {
            let start = Instant::now();
            let read = read()?;
            times.push(start.elapsed());
            item_count = read
                .windows
                .iter()
                .flat_map(|w| &w.panels)
                .map(|p| p.tabs.len())
                .sum();
        }
        Ok(Measurement { item_count, times })
    })();
    let _ = fs::remove_dir_all(&dir);
    result
}

/// 세션 읽기·해석 중앙값이 `limit`을 넘으면 넘은 수치를 담은 메시지를 돌려준다.
pub fn check_session_limit(m: &Measurement, limit: Duration) -> Result<(), String> {
    let median = m.median();
    if median <= limit {
        Ok(())
    } else {
        Err(format!(
            "세션 읽기·해석 상한 초과: 탭 {}개 중앙값 {:.1}ms > 상한 {}ms (최악 {:.1}ms)",
            m.item_count,
            median.as_secs_f64() * 1000.0,
            limit.as_millis(),
            m.max().as_secs_f64() * 1000.0
        ))
    }
}

/// 클립보드 형식 벤치마크의 파일 수.
pub const CLIPBOARD_COUNT: usize = 10_000;

/// 파일 1만 개 목록을 `CF_HDROP` 바이트로 만들기·읽기 시간의 기준(로컬, 릴리스 빌드).
///
/// 1만 개는 경로 평균 60자 안팎이라 약 1.2MB다. 만들기는 글자 변환과 복사, 읽기는 한 번 훑으며 경로를 만드는 일이라
/// 각각 5ms면 넉넉하다. 붙여넣기·끌어 놓기가 UI 응답에 끼어드는 일이라 한 프레임 안에 들어야 한다.
pub const CLIPBOARD_TARGET: Duration = Duration::from_millis(5);

/// CI가 실패시키는 클립보드 형식 시간 상한: 로컬 기준 [`CLIPBOARD_TARGET`]의 10배([`LIST_LIMIT`]과 같은 까닭).
pub const CLIPBOARD_LIMIT: Duration = Duration::from_millis(50);

/// 만들기와 읽기 각각의 측정.
pub struct ClipboardMeasurement {
    /// 목록을 바이트로 만드는 시간.
    pub build: Measurement,
    /// 바이트에서 목록을 읽는 시간.
    pub parse: Measurement,
}

/// 파일 [`CLIPBOARD_COUNT`]개 목록의 `CF_HDROP` 만들기·읽기를 `runs`번씩 잰다(한 번 데운 뒤).
/// 읽은 목록이 넣은 목록과 다르면 실패한다. [`Measurement::item_count`]는 경로 수다.
pub fn measure_clipboard(runs: usize) -> Result<ClipboardMeasurement, String> {
    let paths: Vec<PathBuf> = (0..CLIPBOARD_COUNT)
        .map(|i| {
            PathBuf::from(format!(
                r"C:\사용자\홍길동\문서\프로젝트 {}\보고서 {i} 😀.txt",
                i % 50
            ))
        })
        .collect();
    let bytes = facet_core::clipboard::hdrop_bytes(&paths);
    if facet_core::clipboard::parse_hdrop(&bytes).map_err(|e| e.to_string())? != paths {
        return Err("읽은 목록이 만든 목록과 다릅니다".to_owned());
    }
    let mut build = Vec::with_capacity(runs);
    let mut parse = Vec::with_capacity(runs);
    for _ in 0..runs {
        let start = Instant::now();
        let made = facet_core::clipboard::hdrop_bytes(&paths);
        build.push(start.elapsed());
        std::hint::black_box(&made);
        let start = Instant::now();
        let read = facet_core::clipboard::parse_hdrop(&bytes).map_err(|e| e.to_string())?;
        parse.push(start.elapsed());
        std::hint::black_box(&read);
    }
    Ok(ClipboardMeasurement {
        build: Measurement {
            item_count: paths.len(),
            times: build,
        },
        parse: Measurement {
            item_count: paths.len(),
            times: parse,
        },
    })
}

/// 만들기·읽기 중앙값이 `limit`을 넘으면 넘은 수치를 담은 메시지를 돌려준다.
pub fn check_clipboard_limit(m: &ClipboardMeasurement, limit: Duration) -> Result<(), String> {
    for (name, t) in [("만들기", &m.build), ("읽기", &m.parse)] {
        if t.median() > limit {
            return Err(format!(
                "클립보드 형식 {name} 상한 초과: 경로 {}개 중앙값 {:.2}ms > 상한 {}ms (최악 {:.2}ms)",
                t.item_count,
                t.median().as_secs_f64() * 1000.0,
                limit.as_millis(),
                t.max().as_secs_f64() * 1000.0
            ));
        }
    }
    Ok(())
}

/// 오른쪽 메뉴 구성 벤치마크에서 선택하는 항목 수.
pub const MENU_SELECTED: usize = 10_000;

/// 선택 1만 개에서 오른쪽 메뉴를 만드는 시간의 기준(로컬, 릴리스 빌드).
///
/// 구성은 이미 가진 상태만 보는 순수 함수라 선택 수와 상관없이 항목 열댓 개를 늘어놓는 일이다. 값은 마이크로초 아래이고,
/// 선택한 항목을 훑는 실수(선택 수에 비례)만 잡으려는 기준이다. 오른쪽 클릭에서 메뉴가 뜨기까지는 한 프레임 안이어야 하므로
/// 1ms면 넉넉하다.
pub const MENU_TARGET: Duration = Duration::from_millis(1);

/// CI가 실패시키는 메뉴 구성 시간 상한: 로컬 기준 [`MENU_TARGET`]의 10배([`LIST_LIMIT`]과 같은 까닭).
pub const MENU_LIMIT: Duration = Duration::from_millis(10);

/// 항목 메뉴와 빈 곳 메뉴 구성의 측정. 둘 다 선택 [`MENU_SELECTED`]개에서 재고, 맥락을 모으는 시간을 포함한다.
pub struct MenuMeasurement {
    pub item: Measurement,
    pub background: Measurement,
}

/// 시험 폴더(항목 [`MENU_SELECTED`]개)를 열어 모두 선택한 활성 탭에서 오른쪽 메뉴를 만드는 시간을 `runs`번씩 잰다
/// (한 번 데운 뒤). 선택 수가 다르거나 메뉴가 비면 실패한다. [`Measurement::item_count`]는 선택 수다.
pub fn measure_menu(runs: usize) -> Result<MenuMeasurement, String> {
    use facet_core::Location;
    use facet_core::menu::MenuKind;
    use facet_core::window::App;

    let path = std::env::temp_dir()
        .join("facet-test-folders")
        .join(format!("items-{MENU_SELECTED}"));
    make_test_folder(&path, MENU_SELECTED).map_err(|e| e.to_string())?;
    let mut app = App::new(Some(Location::file_system(&path)));
    app.wait(Duration::from_secs(60))
        .ok_or("시험 폴더를 읽지 못했습니다")?;
    {
        let tab = app.active_tab_mut();
        tab.select_only(0);
        tab.select_range(MENU_SELECTED - 1);
        if tab.selected_count() != MENU_SELECTED {
            return Err(format!(
                "선택 수가 다릅니다: {MENU_SELECTED}개를 기대했지만 {}개",
                tab.selected_count()
            ));
        }
    }
    let measure = |kind: MenuKind| -> Result<Measurement, String> {
        if app.context_menu(kind).entries.is_empty() {
            return Err("메뉴가 비었습니다".to_owned());
        }
        let mut times = Vec::with_capacity(runs);
        for _ in 0..runs {
            let start = Instant::now();
            let menu = app.context_menu(kind);
            times.push(start.elapsed());
            std::hint::black_box(&menu);
        }
        Ok(Measurement {
            item_count: MENU_SELECTED,
            times,
        })
    };
    Ok(MenuMeasurement {
        item: measure(MenuKind::Item)?,
        background: measure(MenuKind::Background)?,
    })
}

/// 항목 메뉴·빈 곳 메뉴 구성 중앙값이 `limit`을 넘으면 넘은 수치를 담은 메시지를 돌려준다.
pub fn check_menu_limit(m: &MenuMeasurement, limit: Duration) -> Result<(), String> {
    for (name, t) in [("항목 메뉴", &m.item), ("빈 곳 메뉴", &m.background)] {
        if t.median() > limit {
            return Err(format!(
                "{name} 구성 상한 초과: 선택 {}개 중앙값 {:.3}ms > 상한 {}ms (최악 {:.3}ms)",
                t.item_count,
                t.median().as_secs_f64() * 1000.0,
                limit.as_millis(),
                t.max().as_secs_f64() * 1000.0
            ));
        }
    }
    Ok(())
}

/// 일괄 이름 바꾸기 미리 보기 벤치마크에서 선택하는 항목 수.
pub const BATCH_RENAME_SELECTED: usize = 10_000;

/// 선택 1만 개의 일괄 이름 바꾸기 미리 보기 계산 시간의 기준(로컬, 릴리스 빌드): 한 프레임(16ms).
///
/// 입력란에 글자를 칠 때마다 UI 스레드에서 미리 보기를 다시 만들므로 한 프레임 안이어야 끊김이 없다.
pub const BATCH_RENAME_TARGET: Duration = Duration::from_millis(16);

/// CI가 실패시키는 미리 보기 계산 시간 상한: 로컬 기준 [`BATCH_RENAME_TARGET`]의 10배([`LIST_LIMIT`]과 같은 까닭).
pub const BATCH_RENAME_LIMIT: Duration = Duration::from_millis(160);

/// 일괄 이름 바꾸기 미리 보기 계산의 측정. [`Measurement::item_count`]는 바뀌는 항목 수다.
pub struct BatchRenameMeasurement {
    pub preview: Measurement,
    /// 미리 보기가 센 바뀌는 항목 수.
    pub rename_count: usize,
}

/// 시험 폴더(항목 [`BATCH_RENAME_SELECTED`]개)를 열어 모두 선택하고, 찾아 바꾸기·앞·뒤 붙이기·번호를 모두 쓴 규칙으로
/// `App::batch_rename_preview`(항목 모으기 포함)를 만드는 시간을 `runs`번 잰다(한 번 데운 뒤).
/// 선택 수나 바뀌는 항목 수가 다르면 실패한다.
pub fn measure_batch_rename(runs: usize) -> Result<BatchRenameMeasurement, String> {
    use facet_core::Location;
    use facet_core::batch_rename::{NumberPosition, Numbering, RenameRule};
    use facet_core::window::App;

    let path = std::env::temp_dir()
        .join("facet-test-folders")
        .join(format!("items-{BATCH_RENAME_SELECTED}"));
    make_test_folder(&path, BATCH_RENAME_SELECTED).map_err(|e| e.to_string())?;
    let mut app = App::new(Some(Location::file_system(&path)));
    app.wait(Duration::from_secs(60))
        .ok_or("시험 폴더를 읽지 못했습니다")?;
    {
        let tab = app.active_tab_mut();
        tab.select_only(0);
        tab.select_range(BATCH_RENAME_SELECTED - 1);
        if tab.selected_count() != BATCH_RENAME_SELECTED {
            return Err(format!(
                "선택 수가 다릅니다: {BATCH_RENAME_SELECTED}개를 기대했지만 {}개",
                tab.selected_count()
            ));
        }
    }
    let rule = RenameRule {
        find: "e".to_owned(),
        replace: "E".to_owned(),
        prefix: "p-".to_owned(),
        suffix: "-s".to_owned(),
        numbering: Some(Numbering {
            start: 1,
            digits: 5,
            position: NumberPosition::End,
            separator: "_".to_owned(),
        }),
    };
    let preview = |app: &facet_core::window::App| {
        app.batch_rename_preview(&rule)
            .ok_or_else(|| "미리 보기를 만들 수 없습니다".to_owned())
    };
    let rename_count = preview(&app)?.rename_count();
    if rename_count != BATCH_RENAME_SELECTED {
        return Err(format!(
            "바뀌는 항목 수가 다릅니다: {BATCH_RENAME_SELECTED}개를 기대했지만 {rename_count}개"
        ));
    }
    let mut times = Vec::with_capacity(runs);
    for _ in 0..runs {
        let start = Instant::now();
        let p = preview(&app)?;
        times.push(start.elapsed());
        std::hint::black_box(&p);
    }
    Ok(BatchRenameMeasurement {
        preview: Measurement {
            item_count: rename_count,
            times,
        },
        rename_count,
    })
}

/// 미리 보기 계산 중앙값이 `limit`을 넘으면 넘은 수치를 담은 메시지를 돌려준다.
pub fn check_batch_rename_limit(m: &BatchRenameMeasurement, limit: Duration) -> Result<(), String> {
    let t = &m.preview;
    if t.median() > limit {
        return Err(format!(
            "일괄 이름 바꾸기 미리 보기 상한 초과: 선택 {}개 중앙값 {:.3}ms > 상한 {}ms (최악 {:.3}ms)",
            t.item_count,
            t.median().as_secs_f64() * 1000.0,
            limit.as_millis(),
            t.max().as_secs_f64() * 1000.0
        ));
    }
    Ok(())
}

/// 필터 벤치마크에서 거르는 항목 수(기본 시험 폴더).
pub const FILTER_COUNT: usize = 100_000;

/// 10만 항목 목록에서 필터 한 글자를 정하는 시간의 로컬 기준(릴리스 빌드): 한 프레임(16ms).
///
/// 글자를 칠 때마다 목록이 좁혀지는 것이 한 프레임 안에 보여야 「치는 즉시」다. 시간은 UI 스레드가 하는 일(후보
/// 요청 만들기 + 결과 받아 보이는 목록 다시 만들기)과 일꾼 스레드가 하는 거르기(이름 일치)를 모두 더한 것이다.
pub const FILTER_TARGET: Duration = Duration::from_millis(16);

/// CI가 실패시키는 필터 시간 상한: 로컬 기준 [`FILTER_TARGET`]의 10배([`LIST_LIMIT`]과 같은 까닭: 공유 러너의
/// 편차를 견디면서 알고리즘이 한 단계 나빠지는 실수만 잡는다).
pub const FILTER_LIMIT: Duration = Duration::from_millis(160);

/// 필터 측정 글자: 빈 필터에서 한 글자, 그 한 글자에서 두 글자.
pub const FILTER_STEPS: [&str; 2] = ["r", "re"];

/// 필터 측정: 빈 필터에서 한 글자(`first`), 한 글자에서 두 글자(`second`).
pub struct FilterMeasurement {
    pub first: Measurement,
    pub second: Measurement,
}

/// 시험 폴더(항목 [`FILTER_COUNT`]개)를 읽은 탭에서 필터 글자를 정하는 시간을 `runs`번씩 잰다(한 번 데운 뒤).
/// 요청 만들기 → 거르기 → 결과 반영을 모두 더한 시간이다. 목록이 바뀐 뒤 첫 요청은 이름 한 벌을 모으느라 더 걸리고
/// (그 한 번만, 탭이 나눠 쓴다) 이 측정은 한 번 데운 뒤의 값이다. [`Measurement::item_count`]는 남은 항목 수다.
/// 거른 결과가 처음부터 거른 것과 다르거나 비면 실패한다.
pub fn measure_filter(runs: usize) -> Result<FilterMeasurement, String> {
    use facet_core::Location;
    use facet_core::tab::{Navigation, ReadOutcome, Tab};

    let path = std::env::temp_dir()
        .join("facet-test-folders")
        .join(format!("items-{FILTER_COUNT}"));
    make_test_folder(&path, FILTER_COUNT).map_err(|e| e.to_string())?;
    let mut tab = Tab::new(Location::file_system(&path), Sort::default(), true);
    let request = tab
        .navigate(Navigation::Refresh)
        .ok_or("시험 폴더를 읽지 못했습니다")?;
    if tab.finish(request.run()) != ReadOutcome::Committed {
        return Err("시험 폴더를 읽지 못했습니다".to_owned());
    }
    if tab.len() != FILTER_COUNT {
        return Err(format!(
            "항목 수가 다릅니다: {FILTER_COUNT}개를 기대했지만 {}개",
            tab.len()
        ));
    }
    let apply = |tab: &mut Tab, text: &str| -> Result<Duration, String> {
        let start = Instant::now();
        let request = tab.request_filter(text).ok_or("거르기 요청이 없습니다")?;
        let result = request.run();
        let applied = tab.finish_filter(result);
        let elapsed = start.elapsed();
        if !applied {
            return Err("거르기 결과를 반영하지 못했습니다".to_owned());
        }
        Ok(elapsed)
    };
    // 처음부터 거른 것과 같은지: 두 글자를 곧바로 걸어 본다.
    tab.set_filter(FILTER_STEPS[1]);
    let direct = tab.len();
    tab.clear_filter();
    let mut first = Vec::with_capacity(runs);
    let mut second = Vec::with_capacity(runs);
    let mut first_count = 0;
    let mut second_count = 0;
    for round in 0..=runs {
        tab.clear_filter();
        let a = apply(&mut tab, FILTER_STEPS[0])?;
        first_count = tab.len();
        let b = apply(&mut tab, FILTER_STEPS[1])?;
        second_count = tab.len();
        // 첫 회는 데우기다.
        if round > 0 {
            first.push(a);
            second.push(b);
        }
    }
    if second_count != direct || first_count == 0 || second_count == 0 || second_count > first_count
    {
        return Err(format!(
            "필터 결과가 이상합니다: 한 글자 {first_count}개, 두 글자 {second_count}개, 처음부터 거르면 {direct}개"
        ));
    }
    Ok(FilterMeasurement {
        first: Measurement {
            item_count: first_count,
            times: first,
        },
        second: Measurement {
            item_count: second_count,
            times: second,
        },
    })
}

/// 필터 중앙값이 `limit`을 넘으면 넘은 수치를 담은 메시지를 돌려준다.
pub fn check_filter_limit(m: &FilterMeasurement, limit: Duration) -> Result<(), String> {
    for (name, t) in [("한 글자", &m.first), ("두 글자", &m.second)] {
        if t.median() > limit {
            return Err(format!(
                "필터 {name} 상한 초과: 남은 {}개 중앙값 {:.2}ms > 상한 {}ms (최악 {:.2}ms)",
                t.item_count,
                t.median().as_secs_f64() * 1000.0,
                limit.as_millis(),
                t.max().as_secs_f64() * 1000.0
            ));
        }
    }
    Ok(())
}

/// 질의 문법 해석 벤치마크가 해석하는 긴 질의: 연산자(그리고·또는·아님·묶기·구절)와 모든 조건을 고루 쓴다.
pub const QUERY_SYNTAX_TEXT: &str = r#"보고서 "final draft" <ext:pdf;docx;xlsx|ext:txt> !temp size:>1mb dm:2026-01..thisweek path:src regex:^a.*z$ *.log <folder:|file:> !<backup|old> size:large <dm:thisyear|dm:2025-06> path:*\src\* 🎉 파티 c:\temp"#;

/// 질의 문법 해석 시간의 로컬 기준(릴리스 빌드): 1ms. 검색창에 치는 글자마다 부르므로 한 프레임에 비해 무시할 만해야
/// 하고, 정상 값은 수십 마이크로초다.
pub const QUERY_SYNTAX_TARGET: Duration = Duration::from_millis(1);

/// CI가 실패시키는 질의 문법 해석 시간 상한: 로컬 기준 [`QUERY_SYNTAX_TARGET`]의 10배([`LIST_LIMIT`]과 같은 까닭:
/// 공유 러너의 편차를 견디면서 알고리즘이 한 단계 나빠지는 실수만 잡는다).
pub const QUERY_SYNTAX_LIMIT: Duration = Duration::from_millis(10);

/// 질의 문법 해석 측정: [`QUERY_SYNTAX_TEXT`] 해석 한 번의 시간. [`Measurement::item_count`]는 조건 트리의 말단 수다.
pub fn measure_query_syntax(runs: usize) -> Result<Measurement, String> {
    use facet_core::query_syntax::{Parsed, Today, parse};
    let today = Today {
        date: chrono::NaiveDate::from_ymd_opt(2026, 11, 18).ok_or("날짜")?,
        utc_offset_seconds: 9 * 3600,
    };
    fn leaves(c: &facet_protocol::Condition) -> usize {
        use facet_protocol::Condition::*;
        match c {
            And(v) | Or(v) => v.iter().map(leaves).sum(),
            Not(i) => leaves(i),
            _ => 1,
        }
    }
    let run = || -> Result<usize, String> {
        match parse(std::hint::black_box(QUERY_SYNTAX_TEXT), today) {
            Ok(Parsed::Query(c)) => Ok(leaves(&c)),
            other => Err(format!("긴 질의를 해석하지 못했습니다: {other:?}")),
        }
    };
    let count = run()?;
    let mut times = Vec::with_capacity(runs);
    for _ in 0..runs {
        let start = Instant::now();
        let n = run()?;
        times.push(start.elapsed());
        if n != count {
            return Err("해석 결과가 달라졌습니다".to_owned());
        }
    }
    Ok(Measurement {
        item_count: count,
        times,
    })
}

/// 질의 문법 해석 중앙값이 `limit`을 넘으면 넘은 수치를 담은 메시지를 돌려준다.
pub fn check_query_syntax_limit(m: &Measurement, limit: Duration) -> Result<(), String> {
    if m.median() > limit {
        return Err(format!(
            "질의 문법 해석 상한 초과: 중앙값 {:.3}ms > 상한 {}ms (최악 {:.3}ms)",
            m.median().as_secs_f64() * 1000.0,
            limit.as_millis(),
            m.max().as_secs_f64() * 1000.0
        ));
    }
    Ok(())
}

/// 실시간 탐색 벤치마크의 항목 수(가지 폴더 포함).
pub const SEARCH_COUNT: usize = 100_000;

/// 시험 나무의 가지 폴더 갈래 수: 단계마다 10개씩 3단계(가지 10 + 100 + 1000 = 1110개, 마지막 단계 1000개가 잎).
pub const TREE_FANOUT: usize = 10;

/// 시험 나무의 가지 단계 수.
pub const TREE_DEPTH: u32 = 3;

/// 실시간 탐색 벤치마크의 이름 조건: 시험 항목 열 개 중 하나(「보고서 n.txt」)가 맞는 한글 부분 일치.
pub const SEARCH_PATTERN: &str = "보고서";

/// 10만 항목 나무의 실시간 탐색 전체 시간(시작부터 끝남 사건까지)의 로컬 기준(릴리스 빌드): 1초.
///
/// 나무에는 폴더가 약 2만 1천 개(가지 1110개와 시험 항목 열 개 중 두 개인 빈 폴더) 있어 폴더마다 여는 비용이
/// 대부분이다. 2026-11-16 GitHub 호스트 Windows 러너(논리 코어 4개)에서 같은 나무를 스레드 하나로 재귀해 훑으면
/// 약 1.6초, 이 엔진(일꾼 4개)은 중앙값 약 0.86초였다. 사람이 큰 폴더 나무에서 「다 찾았다」를 기다릴 만한 시간이고
/// 코어가 더 많은 개발 기계에서는 이보다 빠르다.
pub const SEARCH_TARGET: Duration = Duration::from_millis(1000);

/// CI가 실패시키는 실시간 탐색 시간 상한: 로컬 기준 [`SEARCH_TARGET`]의 10배([`LIST_LIMIT`]과 같은 까닭: 공유
/// 러너의 편차를 견디면서 한 단계 나빠지는 실수(항목마다 시스템 호출을 더 하기, 링크를 따라 들어가 같은 폴더를
/// 되풀이해 훑기 등)만 잡는다). 러너 값의 약 12배라 스레드를 하나로 줄인 정도(러너에서 약 2배)는 잡지 못한다.
pub const SEARCH_LIMIT: Duration = Duration::from_millis(10_000);

/// 가지 폴더 수.
fn tree_branch_count() -> usize {
    (1..=TREE_DEPTH).map(|d| TREE_FANOUT.pow(d)).sum()
}

/// 잎 폴더(마지막 단계 가지) `leaf`번의 시작점 기준 경로.
fn tree_leaf(root: &Path, leaf: usize) -> PathBuf {
    let digits = format!("{leaf:0width$}", width = TREE_DEPTH as usize);
    (1..=digits.len()).fold(root.to_path_buf(), |path, n| {
        path.join(format!("가지 {}", &digits[..n]))
    })
}

/// `path`에 항목 `count`개(가지 폴더 포함)가 든 시험 나무를 만든다: 가지 폴더가 단계마다 [`TREE_FANOUT`]개씩
/// [`TREE_DEPTH`]단계이고, 나머지 항목은 시험 폴더 항목([`test_folder_entry`]의 0번부터, 폴더와 파일이 섞임)을 잎
/// 폴더에 차례로 나눠 담는다. 빠진 항목만 만들고 크기가 다른 파일은 고치므로 다시 돌리면 결과가 같다. 이미 있는
/// 항목의 종류가 다르면 실패한다(시험 나무에 없는 항목이 있는지는 탐색 벤치마크가 수로 확인한다).
pub fn make_test_tree(path: &Path, count: usize) -> Result<MakeReport, TestFolderError> {
    let branches = tree_branch_count();
    let leaves = TREE_FANOUT.pow(TREE_DEPTH);
    let mut report = MakeReport {
        created: 0,
        kept: 0,
    };
    for leaf in 0..leaves {
        let leaf_path = tree_leaf(path, leaf);
        fs::create_dir_all(&leaf_path).map_err(io_err(&leaf_path))?;
    }
    report.kept += branches;
    for index in 0..count.saturating_sub(branches) {
        let entry = test_folder_entry(index);
        let item_path = tree_leaf(path, index % leaves).join(&entry.name);
        let made = if entry.is_folder {
            fs::create_dir(&item_path)
        } else {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&item_path)
                .and_then(|f| f.set_len(entry.size))
        };
        match made {
            Ok(()) => report.created += 1,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                keep_existing(&item_path, &entry)?;
                report.kept += 1;
            }
            Err(e) => return Err(io_err(&item_path)(e)),
        }
    }
    Ok(report)
}

/// 실시간 탐색 벤치마크의 시험 나무 자리: 임시 폴더 아래 `facet-test-folders/tree-N`(`target/` 밑이 아님).
pub fn test_tree_path(count: usize) -> PathBuf {
    std::env::temp_dir()
        .join("facet-test-folders")
        .join(format!("tree-{count}"))
}

/// 실시간 탐색을 끝까지 돌려 (찾은 수, 요약)을 낸다.
fn run_live_search(
    root: &Path,
    pattern: &str,
) -> Result<(usize, facet_core::live_search::SearchSummary), String> {
    use facet_core::live_search::{LiveSearch, SearchEnd, SearchEvent, SearchRequest};
    let mut search = LiveSearch::start(SearchRequest::name(
        Location::file_system(root),
        pattern,
        true,
    ))
    .map_err(|e| e.to_string())?;
    let mut found = 0;
    loop {
        match search.wait_event(Duration::from_secs(120)) {
            Some(SearchEvent::Batch { items, .. }) => found += items.len(),
            Some(SearchEvent::Finished(summary)) => {
                if summary.end != SearchEnd::Finished || summary.found != found {
                    return Err(format!(
                        "탐색 요약이 이상합니다: {summary:?}, 받은 {found}개"
                    ));
                }
                if summary.folders_skipped != 0 {
                    return Err(format!(
                        "시험 나무에서 건너뛴 폴더가 있습니다: {}",
                        summary.folders_skipped
                    ));
                }
                return Ok((found, summary));
            }
            None => return Err("탐색이 끝나지 않았습니다".to_owned()),
        }
    }
}

/// 시험 나무(항목 `count`개, [`make_test_tree`])에서 [`SEARCH_PATTERN`]으로 실시간 탐색을 시작해 끝남 사건을 받을
/// 때까지를 `runs`번 잰다(한 번 데운 뒤). 데울 때 빈 조건으로 훑어 나무의 항목 수가 `count`인지, 잴 때마다 찾은 수가
/// 이름 일치 규칙으로 센 수와 같은지 확인하고 다르면 실패한다. [`Measurement::item_count`]는 찾은 수다.
pub fn measure_live_search(path: &Path, count: usize, runs: usize) -> Result<Measurement, String> {
    let (all, _) = run_live_search(path, "")?;
    if all != count {
        return Err(format!(
            "시험 나무의 항목 수가 다릅니다: {count}개를 기대했지만 {all}개(다른 항목이 섞였으면 {}를 지우고 다시 돌리세요)",
            path.display()
        ));
    }
    let matcher = facet_core::name_match::NameMatcher::new(SEARCH_PATTERN);
    let expected = (0..count.saturating_sub(tree_branch_count()))
        .filter(|&i| matcher.is_match(std::ffi::OsStr::new(&test_folder_entry(i).name)))
        .count();
    let mut times = Vec::with_capacity(runs);
    for _ in 0..runs {
        let start = Instant::now();
        let (found, _) = run_live_search(path, SEARCH_PATTERN)?;
        times.push(start.elapsed());
        if found != expected {
            return Err(format!(
                "찾은 수가 다릅니다: {expected}개를 기대했지만 {found}개"
            ));
        }
    }
    Ok(Measurement {
        item_count: expected,
        times,
    })
}
