//! facet 측정 도구 실행 파일. 보통 `cargo xtask test-folder`·`cargo xtask bench`로 부른다.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use facet_bench::copy::{
    self, COPY_BENCH_BIG_SIZE, COPY_BENCH_RUNS, COPY_BENCH_SMALL_COUNT, COPY_BIG_SIZE,
    COPY_SMALL_COUNT, COPY_SMALL_LIMIT, COPY_SMALL_SIZE, COPY_SMALL_TARGET, COPY_THROUGHPUT_LIMIT,
    COPY_THROUGHPUT_TARGET, ComparePlan, CopyFixture, Engine,
};
use facet_bench::delete::{
    self, DELETE_BENCH_COUNT, DELETE_BENCH_RUNS, DELETE_LIMIT, DELETE_TARGET, RECYCLE_COUNT,
    UNDO_TARGET_LIMIT, UNDO_TARGET_RUNS, UNDO_TARGET_TARGET,
};
use facet_bench::screen::{self, DEFAULT_SCROLL_ROWS, Plan};
use facet_bench::{
    BENCH_QUERY, BENCH_RESULT_LIMIT, BENCH_SORT, BENCH_SORTED_QUERY, BenchCaller, DEFAULT_COUNT,
    INDEX_COUNT, INDEX_MEMORY_LIMIT, LIST_LIMIT, LIST_SORT_LIMIT, LIST_SORT_TARGET, LIST_TARGET,
    MFT_PARSE_LIMIT, MFT_PARSE_TARGET, MFT_RECORDS, Measurement, QUERY_LIMIT, QUERY_TARGET,
    RESORT_LIMIT, RESORT_TARGET, SESSION_PARSE_LIMIT, SESSION_PARSE_TARGET, SESSION_TABS,
    SYNTHETIC_VOLUME, check_limit, check_median, check_memory, check_mft_limit, check_query_limit,
    check_session_limit, make_test_folder, measure_filtered_query, measure_list,
    measure_list_sorted, measure_mft, measure_query, measure_resort, measure_session, mft_image,
    synthetic_index,
};

const USAGE: &str = "\
사용법:
  facet-bench make-folder <경로> [--count N]
      N개(기본 100000) 항목이 든 시험 폴더를 만든다. 다시 돌려도 결과가 같다.
  facet-bench list <경로> [--count N] [--runs N] [--limit-ms N]
      시험 폴더를 만들거나 확인한 뒤 코어 목록 읽기를 N회(기본 5) 재고,
      중앙값이 상한(기본 1000ms)을 넘으면 실패한다. 이어서 「읽기+정렬」(상한 1500ms)과
      「정렬 바꾸기」(상한 500ms)도 N회씩 재어 중앙값이 상한을 넘으면 실패한다.
      --limit-ms는 목록 읽기의 상한만 바꾼다.
  facet-bench index [--count N] [--runs N] [--limit-ms N]
      항목 N개(기본 1000000)의 합성 인덱스를 만들어 메모리를 재고(150MB 이상이면 실패),
      부분 일치 질의와 결과 10만 개를 경로로 정렬하는 질의(둘 다 결과 100개 제한)를
      N회(기본 5)씩 재어 중앙값이 상한(기본 300ms)을 넘으면 실패한다.
      인덱스 파일 저장·읽기 시간도 출력한다(상한 없음).
  facet-bench mft [--count N] [--runs N] [--limit-ms N]
      저장소의 MFT 조각을 되풀이해 레코드 N개(기본 1000000)의 MFT 덩어리를 만들고, 그것을
      인덱스로 만드는 시간을 N회(기본 5) 재어 중앙값이 상한(기본 20000ms)을 넘으면 실패한다.
  facet-bench session [--runs N] [--limit-ms N]
      탭 100개 세션 파일(창 다섯 개, 패널 둘, 긴 경로와 한글 경로 섞임)을 읽고 해석하는 시간을
      N회(기본 5) 재어 중앙값이 상한(기본 50ms)을 넘으면 실패한다.
  facet-bench clipboard [--runs N] [--limit-ms N]
      파일 1만 개 목록을 클립보드 파일 목록 형식(CF_HDROP) 바이트로 만들고 읽는 시간을 각각 N회(기본 5) 재어
      중앙값이 상한(기본 50ms)을 넘으면 실패한다.
  facet-bench menu [--runs N] [--limit-ms N]
      항목 1만 개 시험 폴더에서 1만 개를 선택한 상태로 오른쪽 메뉴(항목 메뉴·빈 곳 메뉴)를 만드는 시간을 각각
      N회(기본 5) 재어 중앙값이 상한(기본 10ms)을 넘으면 실패한다.
  facet-bench batch-rename [--runs N] [--limit-ms N]
      항목 1만 개 시험 폴더에서 1만 개를 선택하고 찾아 바꾸기·앞·뒤 붙이기·번호를 모두 쓴 규칙으로 일괄 이름 바꾸기
      미리 보기를 만드는 시간을 N회(기본 5) 재어 중앙값이 상한(기본 160ms)을 넘으면 실패한다.
  facet-bench query-syntax [--runs N] [--limit-ms N]
      연산자·조건을 고루 쓴 긴 질의 하나를 해석하는 시간을 N회(기본 5) 재어 중앙값이 상한(기본 10ms)을 넘으면
      실패한다.
  facet-bench filter [--runs N] [--limit-ms N]
      항목 10만 개 시험 폴더를 읽은 탭에서 필터 한 글자(빈 필터에서)와 두 글자(한 글자에서)를 정하는 시간을
      각각 N회(기본 5) 재어 중앙값이 상한(기본 160ms)을 넘으면 실패한다.
  facet-bench search [경로] [--count N] [--runs N] [--limit-ms N]
      항목 N개(기본 100000, 가지 폴더 1110개 포함)의 시험 나무(경로를 빼면 임시 폴더의 facet-test-folders/tree-N)를
      만들거나 확인한 뒤, 「보고서」로 실시간 탐색을 시작해 끝날 때까지의 시간을 N회(기본 5) 재어 중앙값이
      상한(기본 10000ms)을 넘으면 실패한다.
  facet-bench content [경로] [--runs N]
      임시 폴더의 본문 128MB 시험 나무(경로를 빼면 facet-test-folders/content-search)를 내용 검색으로 끝까지
      읽는 처리량을 N회(기본 5) 재어(한 번 데운 뒤) 중앙값이 하한(40MB/s)보다 낮으면 실패한다.
  facet-bench copy [--size-mb N] [--count N] [--runs N]
      임시 폴더에 큰 파일 하나(기본 256MiB)와 4KiB 파일 N개(기본 2000)가 든 폴더를 만들고, 작업 대기열의
      복사 작업으로 각각 N회(기본 3) 복사해(한 번 데운 뒤, 대상은 매번 지움) 큰 파일 처리량 중앙값이 하한
      (10MB/s)보다 낮거나 작은 파일 복사 시간 중앙값이 상한(20000ms)을 넘으면 실패한다. Windows가 아니면 건너뛴다.
  facet-bench delete [--count N] [--runs N]
      임시 폴더에 4KiB 파일 N개(기본 10000)가 든 폴더를 만들고 작업 대기열의 영구 삭제 작업으로 지우는 시간을
      N회(기본 5) 재어(한 번 데운 뒤, 회차마다 폴더를 새로 만듦) 중앙값이 상한(10000ms)을 넘으면 실패한다.
      이어 작업 기록 100개(되돌릴 수 없는 것과 섞음)에서 실행취소 대상을 찾는 시간도 재어 중앙값이
      상한(10ms)을 넘으면 실패한다. 삭제는 Windows가 아니면 건너뛴다.
  facet-bench compare-recycle <폴더> [--count N] [--runs N] [--json <파일>]
      폴더 안에 파일 N개(기본 10000)를 만들어 facet 휴지통 삭제 작업과 탐색기 엔진(IFileOperation 휴지통 보내기)이
      번갈아 N회(기본 5)씩 휴지통에 보내는 시간을 표로 낸다(상한 없음, 비교만). --json이면 JSON도 쓴다. 휴지통에 넣은
      항목은 끝나면 휴지통에서 지운다. Windows가 아니면 그렇다고 알리고 실패한다. 보통 `cargo xtask measure-recycle`로 부른다.
  facet-bench compare-copy <원본 폴더> <대상 폴더> [--size-mb N] [--count N] [--runs N] [--json <파일>]
      원본 폴더에 큰 파일 하나와 작은 파일 N개를 만들고 대상 폴더로 facet 엔진과 탐색기(IFileOperation)가
      번갈아 N회(기본 5)씩 복사해 시간·처리량을 표로 내고, --json이면 JSON도 쓴다. 큰 파일 처리량이 탐색기의
      90% 이상, 작은 파일 시간이 탐색기보다 길지 않아야 통과하고, 못 맞추면 실패한다. 만든 파일은 끝나면 지운다.
      Windows가 아니면 그렇다고 알리고 실패한다. 보통 `cargo xtask measure-copy`로 부른다.
  facet-bench screen <경로> --app <facet 실행 파일> [--count N] [--runs N] [--scroll-runs N]
                     [--scroll-rows N] [--refresh-hz N] [--json <파일>]
      시험 폴더를 만들거나 확인한 뒤 앱을 측정 모드로 띄워 콜드 스타트(--runs회, 기본 5),
      시험 폴더의 상위로 띄운 창에서 시험 폴더로 이동한 폴더 열기 첫 화면(--runs회),
      스크롤 프레임 시간(--scroll-runs회, 기본 1)을 잰다. 표를 출력하고 --json이면 JSON도 쓴다.
      통과 기준을 못 맞추면 실패한다. 보통 `cargo xtask measure`로 부른다.";

const DEFAULT_RUNS: usize = 5;

struct Args {
    command: String,
    path: Option<PathBuf>,
    /// 두 번째 경로(`compare-copy`의 대상 폴더).
    second: Option<PathBuf>,
    count: Option<usize>,
    size_mb: Option<u64>,
    /// `--runs`. 빼면 명령마다의 기본값([`Args::runs`], 복사 벤치마크는 `COPY_BENCH_RUNS`).
    runs: Option<usize>,
    limit: Option<Duration>,
    app: Option<PathBuf>,
    scroll_runs: usize,
    scroll_rows: usize,
    refresh_hz: Option<f64>,
    json: Option<PathBuf>,
}

impl Args {
    /// 잴 횟수: `--runs`, 빼면 [`DEFAULT_RUNS`].
    fn runs(&self) -> usize {
        self.runs.unwrap_or(DEFAULT_RUNS)
    }

    fn path(&self) -> Result<&PathBuf, String> {
        self.path
            .as_ref()
            .ok_or_else(|| "경로가 없습니다".to_owned())
    }
}

fn parse(mut it: impl Iterator<Item = String>) -> Result<Args, String> {
    let command = it.next().ok_or("명령이 없습니다")?;
    let mut path = None;
    let mut second = None;
    let mut count = None;
    let mut size_mb = None;
    let mut runs = None;
    let mut limit = None;
    let mut app = None;
    let mut scroll_runs = 1;
    let mut scroll_rows = DEFAULT_SCROLL_ROWS;
    let mut refresh_hz = None;
    let mut json = None;
    while let Some(arg) = it.next() {
        let mut text = |name: &str| it.next().ok_or(format!("{name} 뒤에 값이 없습니다"));
        match arg.as_str() {
            "--app" => {
                app = Some(PathBuf::from(text("--app")?));
                continue;
            }
            "--json" => {
                json = Some(PathBuf::from(text("--json")?));
                continue;
            }
            "--refresh-hz" => {
                let v = text("--refresh-hz")?;
                refresh_hz = Some(
                    v.parse::<f64>()
                        .ok()
                        .filter(|hz| *hz > 0.0)
                        .ok_or(format!("--refresh-hz의 값이 양수가 아닙니다: {v}"))?,
                );
                continue;
            }
            _ => {}
        }
        let mut number = |name: &str| -> Result<u64, String> {
            let v = it.next().ok_or(format!("{name} 뒤에 값이 없습니다"))?;
            v.parse()
                .map_err(|_| format!("{name}의 값이 수가 아닙니다: {v}"))
        };
        match arg.as_str() {
            "--count" => count = Some(number("--count")? as usize),
            "--size-mb" => size_mb = Some(number("--size-mb")?.max(1)),
            "--runs" => runs = Some((number("--runs")? as usize).max(1)),
            "--scroll-runs" => scroll_runs = number("--scroll-runs")? as usize,
            "--scroll-rows" => scroll_rows = (number("--scroll-rows")? as usize).max(1),
            "--limit-ms" => limit = Some(Duration::from_millis(number("--limit-ms")?)),
            _ if path.is_none() && !arg.starts_with("--") => path = Some(PathBuf::from(arg)),
            _ if command == "compare-copy" && second.is_none() && !arg.starts_with("--") => {
                second = Some(PathBuf::from(arg));
            }
            _ => return Err(format!("알 수 없는 인자: {arg}")),
        }
    }
    Ok(Args {
        command,
        path,
        second,
        count,
        size_mb,
        runs,
        limit,
        app,
        scroll_runs,
        scroll_rows,
        refresh_hz,
        json,
    })
}

fn main() -> ExitCode {
    let args = match parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    let result = match args.command.as_str() {
        "make-folder" => make_folder(&args),
        "list" => make_folder(&args).and_then(|()| list(&args)),
        "index" => index(&args),
        "mft" => mft(&args),
        "session" => session(&args),
        "clipboard" => clipboard(&args),
        "menu" => menu(&args),
        "batch-rename" => batch_rename(&args),
        "filter" => filter(&args),
        "query-syntax" => query_syntax(&args),
        "search" => live_search(&args),
        "content" => content_search(&args),
        "copy" => copy_bench(&args),
        "delete" => delete_bench(&args),
        "compare-recycle" => compare_recycle(&args),
        "compare-copy" => compare_copy(&args),
        "screen" => make_folder(&args).and_then(|()| screen(&args)),
        other => Err(format!("알 수 없는 명령: {other}\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("\n{e}");
            ExitCode::FAILURE
        }
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn warn_debug() {
    if cfg!(debug_assertions) {
        println!("경고: 디버그 빌드입니다. 수치는 --release 빌드에서 재야 의미가 있습니다.");
    }
}

fn print_times(m: &Measurement) {
    for (n, t) in m.times.iter().enumerate() {
        println!("  {}회: {:.1}ms", n + 1, ms(*t));
    }
}

fn make_folder(args: &Args) -> Result<(), String> {
    let path = args.path()?;
    let count = args.count.unwrap_or(DEFAULT_COUNT);
    println!("시험 폴더: {} ({}항목)", path.display(), count);
    let start = Instant::now();
    let report = make_test_folder(path, count).map_err(|e| e.to_string())?;
    println!(
        "  새로 만듦 {} / 이미 있음 {} ({:.1}s)",
        report.created,
        report.kept,
        start.elapsed().as_secs_f64()
    );
    Ok(())
}

fn list(args: &Args) -> Result<(), String> {
    warn_debug();
    let count = args.count.unwrap_or(DEFAULT_COUNT);
    let limit = args.limit.unwrap_or(LIST_LIMIT);
    let m = measure_list(args.path()?, args.runs()).map_err(|e| e.to_string())?;
    if m.item_count != count {
        return Err(format!(
            "항목 수가 다릅니다: {}개를 기대했지만 {}개를 읽었습니다",
            count, m.item_count
        ));
    }
    println!(
        "코어 목록 읽기 ({}항목, {}회):",
        m.item_count,
        m.times.len()
    );
    print_times(&m);
    println!(
        "  중앙값 {:.1}ms, 최악 {:.1}ms / 로컬 기준 {}ms, CI 상한 {}ms",
        ms(m.median()),
        ms(m.max()),
        LIST_TARGET.as_millis(),
        limit.as_millis()
    );
    check_limit(&m, limit)?;
    println!("  상한 안: 통과");

    let path = args.path()?;
    let m = measure_list_sorted(path, args.runs()).map_err(|e| e.to_string())?;
    println!(
        "코어 읽기+정렬 ({}항목, 이름 오름차순, {}회):",
        m.item_count,
        m.times.len()
    );
    report_median("읽기+정렬", &m, LIST_SORT_TARGET, LIST_SORT_LIMIT)?;

    let m = measure_resort(path, args.runs()).map_err(|e| e.to_string())?;
    println!(
        "코어 정렬 바꾸기 ({}항목, 열·방향을 차례로 바꿈, {}회):",
        m.item_count,
        m.times.len()
    );
    report_median("정렬 바꾸기", &m, RESORT_TARGET, RESORT_LIMIT)
}

/// 측정값을 출력하고 중앙값이 `limit`을 넘으면 실패한다.
fn report_median(
    what: &str,
    m: &Measurement,
    target: Duration,
    limit: Duration,
) -> Result<(), String> {
    print_times(m);
    println!(
        "  중앙값 {:.1}ms, 최악 {:.1}ms / 로컬 기준 {}ms, CI 상한 {}ms",
        ms(m.median()),
        ms(m.max()),
        target.as_millis(),
        limit.as_millis()
    );
    check_median(what, m, limit)?;
    println!("  상한 안: 통과");
    Ok(())
}

fn index(args: &Args) -> Result<(), String> {
    if args.path.is_some() {
        return Err(format!("index 명령은 경로를 받지 않습니다\n\n{USAGE}"));
    }
    warn_debug();
    let count = args.count.unwrap_or(INDEX_COUNT);
    let limit = args.limit.unwrap_or(QUERY_LIMIT);

    let start = Instant::now();
    let index = synthetic_index(count);
    let bytes = index.memory_bytes();
    println!(
        "합성 인덱스 ({}항목): 만들기 {:.1}ms, 메모리 {:.1}MB / 상한 {:.1}MB",
        index.len(),
        ms(start.elapsed()),
        bytes as f64 / 1e6,
        INDEX_MEMORY_LIMIT as f64 / 1e6
    );
    check_memory(bytes, INDEX_MEMORY_LIMIT)?;
    report_store(&index, args.runs())?;

    let query = facet_index::Query::name_contains(BENCH_QUERY).limit(BENCH_RESULT_LIMIT);
    let m = measure_query(&index, &query, args.runs());
    println!(
        "인덱스 부분 일치 질의 (\"{BENCH_QUERY}\", 결과 {}개 제한, {}개 나옴, {}회):",
        BENCH_RESULT_LIMIT,
        m.item_count,
        m.times.len()
    );
    report_query(&m, limit)?;

    let (key, order) = BENCH_SORT;
    let query = facet_index::Query::name_contains(BENCH_SORTED_QUERY)
        .sort(key, order)
        .limit(BENCH_RESULT_LIMIT);
    let total = index.search(&query).total;
    let m = measure_query(&index, &query, args.runs());
    println!(
        "인덱스 정렬 질의 (\"{BENCH_SORTED_QUERY}\", {key:?} {order:?}, 결과 {}개 제한, 전체 {total}개 중 {}개 나옴, {}회):",
        BENCH_RESULT_LIMIT,
        m.item_count,
        m.times.len()
    );
    report_query(&m, limit)?;
    report_filtered(index, args.runs(), limit)
}

/// 폴더 권한으로 거르는 질의(인덱서 서비스가 파이프로 받는 질의)를 권한 검사 기억이 찬 상태로
/// 잰다. 상한은 거르지 않는 질의와 같다.
fn report_filtered(index: facet_index::Index, runs: usize, limit: Duration) -> Result<(), String> {
    use facet_protocol::{Case, Condition, QueryMessage};

    let volumes = facet_indexer::volumes::Volumes::with_index(SYNTHETIC_VOLUME, index);
    let caller = BenchCaller::default();
    let query = |part: &str, sort| QueryMessage {
        condition: Condition::NameContains {
            part: part.to_owned(),
            case: Case::Insensitive,
        },
        sort,
        limit: Some(BENCH_RESULT_LIMIT as u32),
    };
    let (key, order) = BENCH_SORT;
    let sort = Some((key.into(), order.into()));
    for (part, sort) in [(BENCH_QUERY, None), (BENCH_SORTED_QUERY, sort)] {
        let q = query(part, sort);
        let m = measure_filtered_query(&volumes, &caller, &q, runs)?;
        // 걸러진 결과가 있어야 「걸러진 만큼 뒤 후보로 채우기」까지 잰다.
        let cancel = facet_indexer::session::Cancel::new();
        let shown = |page: facet_indexer::volumes::Page| -> Vec<String> {
            page.hits.into_iter().map(|h| h.path).collect()
        };
        let unfiltered = shown(volumes.search(&q, &cancel).map_err(|e| e.message)?);
        let filtered = shown(
            volumes
                .search_as(&q, &caller, &cancel)
                .map_err(|e| e.message)?,
        );
        let hidden = unfiltered.iter().filter(|p| !filtered.contains(p)).count();
        if sort.is_some() && hidden == 0 {
            return Err(format!(
                "권한으로 거르는 질의 \"{part}\"의 앞쪽 결과에서 걸러진 것이 없습니다(채우기를 재지 못함)"
            ));
        }
        let sorted = match sort {
            Some(_) => format!(", {key:?} {order:?}"),
            None => String::new(),
        };
        println!(
            "권한으로 거르는 질의 (\"{part}\"{sorted}, 결과 {}개 제한, {}개 나옴, 거르지 않은 앞쪽에서 {hidden}개 걸러짐, 기억한 폴더 {}개, {}회):",
            BENCH_RESULT_LIMIT,
            m.item_count,
            volumes.access().len(),
            m.times.len()
        );
        report_query(&m, limit)?;
    }
    Ok(())
}

fn mft(args: &Args) -> Result<(), String> {
    if args.path.is_some() {
        return Err(format!(
            "mft 명령은 경로를 받지 않습니다

{USAGE}"
        ));
    }
    warn_debug();
    let records = args.count.unwrap_or(MFT_RECORDS);
    let limit = args.limit.unwrap_or(MFT_PARSE_LIMIT);
    let image = mft_image(records);
    let m = measure_mft(&image, args.runs())?;
    let mb = image.len() as f64 / 1e6;
    println!(
        "MFT 파싱 (조각을 되풀이한 레코드 {records}개, {mb:.1}MB → 항목 {}개, {}회):",
        m.item_count,
        m.times.len()
    );
    print_times(&m);
    let median = m.median();
    println!(
        "  중앙값 {:.1}ms ({:.0}MB/s, 레코드 100만 개당 {:.1}ms), 최악 {:.1}ms / 로컬 기준 {}ms, CI 상한 {}ms",
        ms(median),
        mb / median.as_secs_f64().max(1e-9),
        ms(median) * 1e6 / records.max(1) as f64,
        ms(m.max()),
        MFT_PARSE_TARGET.as_millis(),
        limit.as_millis()
    );
    check_mft_limit(&m, records, limit)?;
    println!("  상한 안: 통과");
    Ok(())
}

/// 인덱스 저장·읽기 시간을 잰다(재부팅 뒤 바로 검색의 근거). 시간 상한은 두지 않고 출력만 한다.
/// 읽은 인덱스가 같은 항목 수가 아니면 실패한다.
fn report_store(index: &facet_index::Index, runs: usize) -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!("facet-bench-store-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("bench.idx");
    let result = (|| {
        let mut saves = Vec::new();
        let mut loads = Vec::new();
        for _ in 0..runs {
            let t = Instant::now();
            index.save(&path).map_err(|e| e.to_string())?;
            saves.push(t.elapsed());
            let t = Instant::now();
            let loaded = facet_index::Index::load(&path).map_err(|e| e.to_string())?;
            loads.push(t.elapsed());
            if loaded.len() != index.len() {
                return Err(format!(
                    "읽은 인덱스의 항목 수가 다릅니다: {} / {}",
                    loaded.len(),
                    index.len()
                ));
            }
        }
        let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
        let median = |mut v: Vec<Duration>| {
            v.sort();
            v[v.len() / 2]
        };
        println!(
            "인덱스 파일 ({:.1}MB, {}회): 저장 중앙값 {:.1}ms, 읽기 중앙값 {:.1}ms",
            size as f64 / 1e6,
            runs,
            ms(median(saves)),
            ms(median(loads))
        );
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// 질의 측정값을 출력하고 중앙값이 `limit`을 넘으면 실패한다.
fn report_query(m: &Measurement, limit: Duration) -> Result<(), String> {
    print_times(m);
    println!(
        "  중앙값 {:.1}ms, 최악 {:.1}ms / 로컬 기준 {}ms, CI 상한 {}ms",
        ms(m.median()),
        ms(m.max()),
        QUERY_TARGET.as_millis(),
        limit.as_millis()
    );
    check_query_limit(m, limit)?;
    println!("  상한 안: 통과");
    Ok(())
}

fn session(args: &Args) -> Result<(), String> {
    if args.path.is_some() {
        return Err(format!(
            "session 명령은 경로를 받지 않습니다

{USAGE}"
        ));
    }
    warn_debug();
    let limit = args.limit.unwrap_or(SESSION_PARSE_LIMIT);
    let m = measure_session(args.runs())?;
    println!(
        "세션 읽기·해석 (탭 {}개, 기대 {SESSION_TABS}개, {}회):",
        m.item_count,
        m.times.len()
    );
    print_times(&m);
    println!(
        "  중앙값 {:.2}ms, 최악 {:.2}ms / 로컬 기준 {}ms, CI 상한 {}ms",
        ms(m.median()),
        ms(m.max()),
        SESSION_PARSE_TARGET.as_millis(),
        limit.as_millis()
    );
    check_session_limit(&m, limit)?;
    println!("  상한 안: 통과");
    Ok(())
}

fn clipboard(args: &Args) -> Result<(), String> {
    if args.path.is_some() {
        return Err(format!(
            "clipboard 명령은 경로를 받지 않습니다

{USAGE}"
        ));
    }
    warn_debug();
    let limit = args.limit.unwrap_or(facet_bench::CLIPBOARD_LIMIT);
    let m = facet_bench::measure_clipboard(args.runs())?;
    for (name, t) in [("만들기", &m.build), ("읽기", &m.parse)] {
        println!(
            "클립보드 형식 {name} (경로 {}개, {}회):",
            t.item_count,
            t.times.len()
        );
        print_times(t);
        println!(
            "  중앙값 {:.3}ms, 최악 {:.3}ms / 로컬 기준 {}ms, CI 상한 {}ms",
            ms(t.median()),
            ms(t.max()),
            facet_bench::CLIPBOARD_TARGET.as_millis(),
            limit.as_millis()
        );
    }
    facet_bench::check_clipboard_limit(&m, limit)?;
    println!("  상한 안: 통과");
    Ok(())
}

fn menu(args: &Args) -> Result<(), String> {
    if args.path.is_some() {
        return Err(format!(
            "menu 명령은 경로를 받지 않습니다

{USAGE}"
        ));
    }
    warn_debug();
    let limit = args.limit.unwrap_or(facet_bench::MENU_LIMIT);
    let m = facet_bench::measure_menu(args.runs())?;
    for (name, t) in [("항목 메뉴", &m.item), ("빈 곳 메뉴", &m.background)] {
        println!(
            "오른쪽 메뉴 구성: {name} (선택 {}개, {}회):",
            t.item_count,
            t.times.len()
        );
        print_times(t);
        println!(
            "  중앙값 {:.3}ms, 최악 {:.3}ms / 로컬 기준 {}ms, CI 상한 {}ms",
            ms(t.median()),
            ms(t.max()),
            facet_bench::MENU_TARGET.as_millis(),
            limit.as_millis()
        );
    }
    facet_bench::check_menu_limit(&m, limit)?;
    println!("  상한 안: 통과");
    Ok(())
}

fn batch_rename(args: &Args) -> Result<(), String> {
    if args.path.is_some() {
        return Err(format!(
            "batch-rename 명령은 경로를 받지 않습니다

{USAGE}"
        ));
    }
    warn_debug();
    let limit = args.limit.unwrap_or(facet_bench::BATCH_RENAME_LIMIT);
    let m = facet_bench::measure_batch_rename(args.runs())?;
    let t = &m.preview;
    println!(
        "일괄 이름 바꾸기 미리 보기 (선택 {}개, 바뀜 {}개, {}회):",
        facet_bench::BATCH_RENAME_SELECTED,
        m.rename_count,
        t.times.len()
    );
    print_times(t);
    println!(
        "  중앙값 {:.3}ms, 최악 {:.3}ms / 로컬 기준 {}ms, CI 상한 {}ms",
        ms(t.median()),
        ms(t.max()),
        facet_bench::BATCH_RENAME_TARGET.as_millis(),
        limit.as_millis()
    );
    facet_bench::check_batch_rename_limit(&m, limit)?;
    println!("  상한 안: 통과");
    Ok(())
}

fn query_syntax(args: &Args) -> Result<(), String> {
    if args.path.is_some() {
        return Err(format!(
            "query-syntax 명령은 경로를 받지 않습니다

{USAGE}"
        ));
    }
    warn_debug();
    let limit = args.limit.unwrap_or(facet_bench::QUERY_SYNTAX_LIMIT);
    let m = facet_bench::measure_query_syntax(args.runs())?;
    println!(
        "질의 문법 해석: 긴 질의 하나(말단 {}개, {}회):",
        m.item_count,
        m.times.len()
    );
    print_times(&m);
    println!(
        "  중앙값 {:.3}ms, 최악 {:.3}ms / 로컬 기준 {}ms, CI 상한 {}ms",
        ms(m.median()),
        ms(m.max()),
        facet_bench::QUERY_SYNTAX_TARGET.as_millis(),
        limit.as_millis()
    );
    facet_bench::check_query_syntax_limit(&m, limit)?;
    println!("  상한 안: 통과");
    Ok(())
}

fn filter(args: &Args) -> Result<(), String> {
    if args.path.is_some() {
        return Err(format!(
            "filter 명령은 경로를 받지 않습니다

{USAGE}"
        ));
    }
    warn_debug();
    let limit = args.limit.unwrap_or(facet_bench::FILTER_LIMIT);
    let m = facet_bench::measure_filter(args.runs())?;
    for (name, t) in [("한 글자", &m.first), ("두 글자", &m.second)] {
        println!(
            "필터: {name} ({}항목 중 {}개 남음, {}회):",
            facet_bench::FILTER_COUNT,
            t.item_count,
            t.times.len()
        );
        print_times(t);
        println!(
            "  중앙값 {:.2}ms, 최악 {:.2}ms / 로컬 기준 {}ms, CI 상한 {}ms",
            ms(t.median()),
            ms(t.max()),
            facet_bench::FILTER_TARGET.as_millis(),
            limit.as_millis()
        );
    }
    facet_bench::check_filter_limit(&m, limit)?;
    println!("  상한 안: 통과");
    Ok(())
}

fn live_search(args: &Args) -> Result<(), String> {
    warn_debug();
    let count = args.count.unwrap_or(facet_bench::SEARCH_COUNT);
    let limit = args.limit.unwrap_or(facet_bench::SEARCH_LIMIT);
    let path = args
        .path
        .clone()
        .unwrap_or_else(|| facet_bench::test_tree_path(count));
    println!("시험 나무: {} ({count}항목)", path.display());
    let start = Instant::now();
    let report = facet_bench::make_test_tree(&path, count).map_err(|e| e.to_string())?;
    println!(
        "  새로 만듦 {} / 이미 있음 {} ({:.1}s)",
        report.created,
        report.kept,
        start.elapsed().as_secs_f64()
    );
    let m = facet_bench::measure_live_search(&path, count, args.runs())?;
    println!(
        "실시간 탐색: 「{}」 ({count}항목 나무에서 {}개 찾음, 일꾼 스레드 {}개, {}회):",
        facet_bench::SEARCH_PATTERN,
        m.item_count,
        facet_core::live_search::thread_count(),
        m.times.len()
    );
    print_times(&m);
    println!(
        "  중앙값 {:.1}ms, 최악 {:.1}ms / 로컬 기준 {}ms, CI 상한 {}ms",
        ms(m.median()),
        ms(m.max()),
        facet_bench::SEARCH_TARGET.as_millis(),
        limit.as_millis()
    );
    check_median("실시간 탐색", &m, limit)?;
    println!("  상한 안: 통과");
    Ok(())
}

fn content_search(args: &Args) -> Result<(), String> {
    use facet_bench::content;
    warn_debug();
    let path = args.path.clone().unwrap_or_else(content::test_content_path);
    println!("내용 검색 시험 나무: {}", path.display());
    let m = content::measure_content_search(&path, args.runs())?;
    println!(
        "내용 검색: 「{}」 (파일 {}개 {:.0}MB, 일꾼 스레드 {}개, {}회):",
        content::CONTENT_NEEDLE,
        m.item_count,
        content::CONTENT_TOTAL_BYTES as f64 / 1e6,
        facet_core::content_search::thread_count(),
        m.times.len()
    );
    print_times(&m);
    println!(
        "  중앙값 {:.1}MB/s, 최악 {:.1}MB/s / 로컬 기준 {}MB/s, CI 하한 {}MB/s",
        content::content_throughput(m.median()) / 1e6,
        content::content_throughput(m.max()) / 1e6,
        content::CONTENT_THROUGHPUT_TARGET / 1_000_000,
        content::CONTENT_THROUGHPUT_LIMIT / 1_000_000
    );
    content::check_content_throughput(&m, content::CONTENT_THROUGHPUT_LIMIT)?;
    println!("  하한 위: 통과");
    Ok(())
}

fn screen(args: &Args) -> Result<(), String> {
    let app = args.app.as_deref().ok_or("--app이 없습니다")?;
    let plan = Plan {
        app,
        folder: args.path()?,
        item_count: args.count.unwrap_or(DEFAULT_COUNT),
        runs: args.runs(),
        scroll_runs: args.scroll_runs,
        scroll_rows: args.scroll_rows,
        refresh_hz: args.refresh_hz,
    };
    let result = screen::measure(&plan)?;
    println!(
        "
{}",
        result.to_table()
    );
    if let Some(json) = &args.json {
        std::fs::write(json, result.to_json())
            .map_err(|e| format!("{}를 쓰지 못함: {e}", json.display()))?;
        println!("JSON: {}", json.display());
    }
    if result.verdict().pass() {
        Ok(())
    } else {
        Err("통과 기준을 맞추지 못했습니다".to_owned())
    }
}

/// 큰 파일 크기(`--size-mb`, MiB). 빼면 `default`.
fn big_size(args: &Args, default: u64) -> u64 {
    args.size_mb.map_or(default, |mb| mb << 20)
}

/// 복사 벤치마크: 작업 대기열의 복사 작업으로 큰 파일 처리량과 작은 파일들의 복사 시간을 잰다.
fn copy_bench(args: &Args) -> Result<(), String> {
    if args.path.is_some() {
        return Err(format!("copy 명령은 경로를 받지 않습니다\n\n{USAGE}"));
    }
    if !cfg!(windows) {
        println!("복사 벤치마크: Windows가 아니므로 건너뜀(복사 엔진은 Windows 전용)");
        return Ok(());
    }
    warn_debug();
    let size = big_size(args, COPY_BENCH_BIG_SIZE);
    let runs = args.runs.unwrap_or(COPY_BENCH_RUNS);
    let count = args.count.unwrap_or(COPY_BENCH_SMALL_COUNT);
    let root = std::env::temp_dir().join(format!("facet-bench-copy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let start = Instant::now();
    let fixture = CopyFixture::create(&root.join("원본"), size, count, COPY_SMALL_SIZE)
        .map_err(|e| format!("{}: {e}", root.display()))?;
    println!(
        "복사 원본 만듦: {} ({:.1}s)",
        fixture.root.display(),
        start.elapsed().as_secs_f64()
    );
    let target = root.join("대상");
    let result = (|| {
        let m = copy::measure_copy(
            Engine::Facet,
            std::slice::from_ref(&fixture.big),
            &target,
            fixture.big_items(),
            runs,
        )?;
        println!(
            "복사 큰 파일 하나 ({:.1}MB, 작업 대기열, {}회):",
            size as f64 / 1e6,
            m.times.len()
        );
        print_times(&m);
        println!(
            "  중앙값 {:.1}ms ({:.1}MB/s), 최악 {:.1}ms ({:.1}MB/s) / 로컬 기준 {:.0}MB/s, CI 하한 {:.0}MB/s",
            ms(m.median()),
            copy::throughput(size, m.median()) / 1e6,
            ms(m.max()),
            copy::throughput(size, m.max()) / 1e6,
            COPY_THROUGHPUT_TARGET as f64 / 1e6,
            COPY_THROUGHPUT_LIMIT as f64 / 1e6
        );
        copy::check_throughput(&m, size, COPY_THROUGHPUT_LIMIT)?;
        println!("  하한 위: 통과");

        let m = copy::measure_copy(
            Engine::Facet,
            std::slice::from_ref(&fixture.small),
            &target,
            fixture.small_items(),
            runs,
        )?;
        println!(
            "복사 작은 파일 {count}개 ({}바이트씩, 폴더 하나, 작업 대기열, {}회):",
            COPY_SMALL_SIZE,
            m.times.len()
        );
        report_median("작은 파일 복사", &m, COPY_SMALL_TARGET, COPY_SMALL_LIMIT)
    })();
    let _ = std::fs::remove_dir_all(&root);
    result
}

/// 탐색기 비교 측정: 같은 복사를 facet 엔진과 `IFileOperation`으로 잰다.
fn compare_copy(args: &Args) -> Result<(), String> {
    let source = args.path.as_ref().ok_or("원본 폴더가 없습니다")?;
    let target = args.second.as_ref().ok_or("대상 폴더가 없습니다")?;
    if !cfg!(windows) {
        return Err("탐색기 비교 측정은 Windows에서만 잴 수 있습니다(IFileOperation)".into());
    }
    warn_debug();
    let plan = ComparePlan {
        source,
        target,
        big_size: big_size(args, COPY_BIG_SIZE),
        small_count: args.count.unwrap_or(COPY_SMALL_COUNT),
        small_size: COPY_SMALL_SIZE,
        runs: args.runs(),
    };
    let result = copy::compare(&plan)?;
    println!("\n{}", result.to_table());
    if let Some(json) = &args.json {
        std::fs::write(json, result.to_json())
            .map_err(|e| format!("{}를 쓰지 못함: {e}", json.display()))?;
        println!("JSON: {}", json.display());
    }
    if result.verdict().pass() {
        Ok(())
    } else {
        Err("통과 기준을 맞추지 못했습니다".to_owned())
    }
}

/// 영구 삭제 벤치마크와 실행취소 대상 찾기: CI가 상한을 지킨다.
fn delete_bench(args: &Args) -> Result<(), String> {
    if args.path.is_some() {
        return Err(format!(
            "delete 명령은 경로를 받지 않습니다

{USAGE}"
        ));
    }
    warn_debug();
    let runs = args.runs.unwrap_or(DELETE_BENCH_RUNS);
    let count = args.count.unwrap_or(DELETE_BENCH_COUNT);
    if cfg!(windows) {
        let root = std::env::temp_dir().join(format!("facet-bench-delete-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
        let result = delete::measure_permanent_delete(&root, count, runs);
        let _ = std::fs::remove_dir_all(&root);
        let m = result?;
        println!(
            "영구 삭제 (4KiB 파일 {count}개가 든 폴더, 항목 {}개, 작업 대기열, {}회):",
            m.item_count,
            m.times.len()
        );
        report_median("영구 삭제", &m, DELETE_TARGET, DELETE_LIMIT)?;
    } else {
        println!("영구 삭제 벤치마크: Windows가 아니므로 건너뜀(삭제 엔진은 Windows 전용)");
    }
    let m = delete::measure_undo_target(args.runs.unwrap_or(UNDO_TARGET_RUNS))?;
    println!(
        "실행취소 대상 찾기 (작업 기록 {}개, 되돌릴 수 없는 것과 섞음, {}회):",
        m.item_count,
        m.times.len()
    );
    print_times_us(&m);
    println!(
        "  중앙값 {:.1}µs, 최악 {:.1}µs / 로컬 기준 {}ms, CI 상한 {}ms",
        ms(m.median()) * 1000.0,
        ms(m.max()) * 1000.0,
        UNDO_TARGET_TARGET.as_millis(),
        UNDO_TARGET_LIMIT.as_millis()
    );
    facet_bench::check_median("실행취소 대상 찾기", &m, UNDO_TARGET_LIMIT)?;
    println!("  상한 안: 통과");
    Ok(())
}

fn print_times_us(m: &Measurement) {
    for (n, t) in m.times.iter().enumerate() {
        println!("  {}회: {:.1}µs", n + 1, ms(*t) * 1000.0);
    }
}

/// 휴지통 보내기 비교: facet 휴지통 삭제 작업과 탐색기 엔진을 같은 기계에서 재어 견준다(상한 없음).
fn compare_recycle(args: &Args) -> Result<(), String> {
    let root = args.path.as_ref().ok_or("폴더가 없습니다")?;
    if !cfg!(windows) {
        return Err("휴지통 보내기 비교는 Windows에서만 잴 수 있습니다(IFileOperation)".into());
    }
    warn_debug();
    let result = delete::compare_recycle(root, args.count.unwrap_or(RECYCLE_COUNT), args.runs())?;
    println!(
        "
{}",
        result.to_table()
    );
    if let Some(json) = &args.json {
        std::fs::write(json, result.to_json())
            .map_err(|e| format!("{}를 쓰지 못함: {e}", json.display()))?;
        println!("JSON: {}", json.display());
    }
    Ok(())
}
