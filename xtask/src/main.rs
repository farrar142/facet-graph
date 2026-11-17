//! 저장소 작업 명령. `cargo xtask <명령>`으로 부른다.
//!
//! - `ci`: 형식 검사 → lint(경고를 오류로) → 빌드 → 테스트 → 실제 볼륨 테스트 → 코어 벤치마크
//!   → 인덱스 벤치마크 → MFT 파싱 벤치마크 → 세션 읽기·해석 벤치마크 → 클립보드 형식·오른쪽 메뉴·일괄 이름 바꾸기 미리 보기·필터·실시간 탐색 벤치마크 → 복사 벤치마크 → 삭제 벤치마크. CI와 로컬이 같은 명령을 쓴다. 실제 볼륨 테스트는 관리자일 때만 시험 VHD를
//!   만들어 돌린다.
//! - `test-folder`: 시험 폴더를 만든다(`facet-bench make-folder`).
//! - `bench`: 벤치마크만 돌린다(`facet-bench list`·`facet-bench index`·`facet-bench mft`·`facet-bench session`·
//!   `facet-bench copy`·`facet-bench delete`, 릴리스 빌드).
//!
//! - `measure [인자]`: 앱과 측정 도구를 릴리스 빌드해 화면 수치(콜드 스타트·세션 복원 콜드 스타트·폴더 열기 첫 화면·스크롤 프레임)를 잰다.
//! - `measure-copy <원본 폴더> <대상 폴더> [인자]`: 측정 도구를 릴리스 빌드해 같은 복사를 facet 엔진과 탐색기
//!   (`IFileOperation`)로 재어 견준다(`facet-bench compare-copy`, 사람이 잼).
//! - `measure-recycle <폴더> [인자]`: 측정 도구를 릴리스 빌드해 같은 파일 1만 개를 facet 휴지통 삭제 작업과 탐색기
//!   (`IFileOperation`)로 휴지통에 보내 견준다(`facet-bench compare-recycle`, 사람이 잼, 상한 없음).
//! - `mft-fixture`: 시험 VHD에서 MFT 조각과 기대값 목록을 만든다(`mft_fixture` 모듈, 관리자).
//! - `dist <label>`: release 빌드한 facet을 portable zip과 체크섬으로 묶는다(`dist` 모듈).
//! - `indexer <install|uninstall>`: 인덱서 서비스를 release로 빌드해 개발 기계에 설치하거나 지운다(관리자).
//!
//! 벤치마크 상한과 그 근거는 `crates/facet-bench/src/lib.rs`의 `LIST_LIMIT`·`QUERY_LIMIT`·
//! `INDEX_MEMORY_LIMIT`·`MFT_PARSE_LIMIT`·`SESSION_PARSE_LIMIT`·`CLIPBOARD_LIMIT`·`MENU_LIMIT`·`BATCH_RENAME_LIMIT`·`FILTER_LIMIT`·`SEARCH_LIMIT`, `crates/facet-bench/src/copy.rs`의
//! `COPY_THROUGHPUT_LIMIT`·`COPY_SMALL_LIMIT`, `crates/facet-bench/src/delete.rs`의 `DELETE_LIMIT`·
//! `UNDO_TARGET_LIMIT`에 있다.

mod dist;
mod mft_fixture;
mod vhd;

use std::env;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

const USAGE: &str = "\
사용법: cargo xtask <명령>

명령:
  ci                             형식 검사, lint(경고를 오류로), 빌드, 테스트, 실제 볼륨 테스트,
                                 코어 벤치마크, 인덱스 벤치마크, MFT 파싱 벤치마크, 세션 읽기·해석 벤치마크, 복사 벤치마크, 삭제 벤치마크를 차례로 돌린다. 실제 볼륨 테스트는
                                 관리자일 때만 시험 VHD를 만들어 돌리고, 아니면 건너뛴다고 출력한다
  test-folder [경로] [--count N]  N개(기본 100000) 항목이 든 시험 폴더를 만든다.
                                 경로를 빼면 임시 폴더의 facet-test-folders/items-N
  bench                          10만 항목 시험 폴더를 만들거나 확인하고 코어 목록 읽기·읽기+정렬·정렬 바꾸기를 잰 뒤,
                                 합성 인덱스 100만 개의 메모리와 부분 일치 질의를 재고, MFT 조각을
                                 되풀이한 레코드 100만 개를 인덱스로 만드는 시간을 재고, 탭 100개 세션 파일을 읽고 해석하는 시간을 재고,
                                 10만 항목 시험 나무를 실시간 탐색으로 끝까지 훑는 시간을 재고,
                                 임시 폴더에서 1GiB 파일 하나의 복사 처리량과 4KiB 파일 1만 개 폴더의 복사 시간을 재고,
                                 4KiB 파일 1만 개 폴더의 영구 삭제 시간과 작업 기록 100개에서 실행취소 대상 찾는 시간을 잰다(릴리스 빌드)
  measure [--count N] [--runs N] [--scroll-runs N] [--scroll-rows N] [--refresh-hz N] [--json 파일] [경로]
                                 앱을 릴리스 빌드하고 시험 폴더(기본 10만 항목)를 열어 콜드 스타트, 떠 있는 창에서
                                 그 폴더로 이동한 폴더 열기 첫 화면, 탭 10개짜리 창 둘의 세션 복원 콜드 스타트(모두 중앙값·최악값), 스크롤 프레임 시간을 잰다. 표와 JSON(기본 target/measure.json)을 내고,
                                 통과 기준을 못 맞추면 실패한다. CI는 부르지 않는다(사람이 잰다)
  measure-copy <원본 폴더> <대상 폴더> [--size-mb N] [--count N] [--runs N] [--json 파일]
                                 측정 도구를 릴리스 빌드하고 원본 폴더에 큰 파일 하나(기본 1024MiB)와 4KiB 파일 N개(기본 10000)를
                                 만들어 대상 폴더로 facet 엔진과 탐색기(IFileOperation)가 번갈아 복사한 시간·처리량을 표와
                                 JSON(기본 target/measure-copy.json)으로 낸다. 큰 파일 처리량이 탐색기의 90% 이상, 작은 파일이
                                 탐색기보다 느리지 않아야 통과하고 못 맞추면 실패한다. 같은 디스크 안·디스크 사이·USB 드라이브를
                                 폴더로 골라 잰다. CI는 부르지 않는다(사람이 잰다)
  measure-recycle <폴더> [--count N] [--runs N] [--json 파일]
                                 측정 도구를 릴리스 빌드하고 폴더 안에 파일 N개(기본 10000)를 만들어 facet 휴지통 삭제 작업과
                                 탐색기(IFileOperation)가 번갈아 휴지통에 보낸 시간을 표와 JSON(기본 target/measure-recycle.json)으로
                                 낸다. 상한 없이 비교만 낸다. 휴지통에 넣은 항목은 끝나면 휴지통에서 지운다. 휴지통이 있는(고정 디스크)
                                 폴더를 골라야 한다. CI는 부르지 않는다(사람이 잼)
  mft-fixture                    (관리자) 시험 VHD에 정해진 항목을 만들고 MFT를 떠서
                                 testdata/mft/에 조각과 기대값 목록을 쓴다
  dist <label>                   facet을 릴리스 빌드해 target/dist/에 facet-<label>-windows-x64.zip과
                                 그 .sha256을 만든다. label은 nightly나 태그 이름(예: v0.1.0)이고
                                 영문자·숫자·`.`·`-`·`_`만 쓸 수 있다
  indexer <install|uninstall>    (관리자) install: facet-indexer를 릴리스 빌드하고 target/release의 실행 파일을
                                 인덱서 서비스(LocalSystem, 자동 시작)로 등록해 시작한다.
                                 uninstall: 서비스를 멈추고 지우고 인덱스 폴더도 지운다. 서비스가 도는 동안에는 실행 파일이 잠겨
                                 다시 빌드하지 못하므로, 코드를 고친 뒤에는 uninstall 다음 install을 한다";

/// 벤치마크 폴더의 항목 수.
const BENCH_COUNT: &str = "100000";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("ci") => run_ci(),
        Some("bench") => run_steps("bench", &bench_steps()),
        Some("test-folder") => match test_folder_step(&args[1..]) {
            Ok(step) => run_steps("test-folder", &[step]),
            Err(e) => {
                eprintln!("{e}\n\n{USAGE}");
                ExitCode::FAILURE
            }
        },
        Some("measure") => run_measure(&args[1..]),
        Some("measure-copy") => run_measure_copy(&args[1..]),
        Some("measure-recycle") => run_measure_recycle(&args[1..]),
        Some("mft-fixture") => match mft_fixture::run(&workspace_root()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("xtask mft-fixture: {e}");
                ExitCode::FAILURE
            }
        },
        Some("dist") => match dist_label(&args[1..]) {
            Ok(label) => run_dist(label),
            Err(e) => {
                eprintln!("{e}\n\n{USAGE}");
                ExitCode::FAILURE
            }
        },
        Some("indexer") => match indexer_action(&args[1..]) {
            Ok(action) => run_indexer(action),
            Err(e) => {
                eprintln!("{e}\n\n{USAGE}");
                ExitCode::FAILURE
            }
        },
        Some(other) => {
            eprintln!("알 수 없는 명령: {other}\n\n{USAGE}");
            ExitCode::FAILURE
        }
        None => {
            eprintln!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}

/// 한 단계: 이름과 `cargo`에 넘길 인자, 더할 환경 변수.
struct Step {
    name: &'static str,
    args: Vec<String>,
    env: Vec<(&'static str, String)>,
}

fn step(name: &'static str, args: &[&str]) -> Step {
    Step {
        name,
        args: args.iter().map(|s| (*s).to_owned()).collect(),
        env: Vec::new(),
    }
}

/// `ci`: 앞 단계들 → 실제 볼륨 테스트 → 코어 벤치마크 → 인덱스 벤치마크.
fn run_ci() -> ExitCode {
    if run_steps("ci", &ci_steps()) != ExitCode::SUCCESS || real_volume_tests() != ExitCode::SUCCESS
    {
        return ExitCode::FAILURE;
    }
    run_steps("ci", &bench_steps())
}

/// 실제 볼륨 테스트를 고르는 이름 앞부분. 이 테스트들은 `#[ignore]`로 기본 `cargo test`에서 빠진다.
const REAL_VOLUME_PREFIX: &str = "real_volume_";
/// 실제 볼륨 테스트에 시험 볼륨의 드라이브 문자를 넘기는 환경 변수.
const REAL_VOLUME_VAR: &str = "FACET_TEST_VOLUME";

/// 관리자면 시험 VHD를 만들어 붙이고 실제 볼륨 테스트(`real_volume_`로 시작하는 `#[ignore]` 테스트)만
/// 돌린 뒤 뗀다. 관리자가 아니거나 Windows가 아니면 건너뛴다고 출력한다.
fn real_volume_tests() -> ExitCode {
    const NAME: &str = "실제 볼륨 테스트";
    if !cfg!(windows) {
        println!(
            "
=== xtask ci: {NAME}: Windows가 아니므로 건너뜀 ==="
        );
        return ExitCode::SUCCESS;
    }
    if !vhd::is_admin() {
        println!(
            "
=== xtask ci: {NAME}: 관리자 권한이 아니므로 건너뜀 ==="
        );
        return ExitCode::SUCCESS;
    }
    println!(
        "
=== xtask ci: {NAME}: 시험 VHD 만들기 ==="
    );
    let vhd = match vhd::TestVhd::create() {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "
xtask ci: 시험 VHD를 만들지 못함: {e}"
            );
            return ExitCode::FAILURE;
        }
    };
    let mut test = step(
        NAME,
        &[
            "test",
            "--workspace",
            "--locked",
            "--",
            "--ignored",
            REAL_VOLUME_PREFIX,
        ],
    );
    test.env.push((REAL_VOLUME_VAR, format!("{}:", vhd.letter)));
    // vhd는 이 함수가 끝날 때 떼어진다.
    run_steps("ci", &[test])
}

fn ci_steps() -> Vec<Step> {
    vec![
        step("형식 검사", &["fmt", "--all", "--check"]),
        step(
            "lint",
            &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--locked",
                "--",
                "-D",
                "warnings",
            ],
        ),
        step(
            "빌드",
            &["build", "--workspace", "--all-targets", "--locked"],
        ),
        test_step(),
    ]
}

/// 다른 볼륨 이동 테스트(`crates/facet-win/tests/file_ops.rs`의 이름 바꾸기, `crates/facet-core/tests/moves.rs`의
/// 볼륨을 넘는 이동 작업)에 두 번째 볼륨의 폴더를 넘기는 환경 변수.
const OTHER_VOLUME_VAR: &str = "FACET_TEST_OTHER_VOLUME_DIR";

/// `cargo test --workspace`. 임시 폴더와 작업 공간이 다른 드라이브면(GitHub 러너는 임시 폴더가 `C:`, 저장소가
/// `D:`) 작업 공간의 `target/other-volume`을 두 번째 볼륨으로 넘겨 다른 볼륨 이동도 확인한다. 이미 정해 두었으면
/// 그대로 둔다.
fn test_step() -> Step {
    let mut test = step("테스트", &["test", "--workspace", "--locked"]);
    if env::var_os(OTHER_VOLUME_VAR).is_none()
        && let Some(dir) = other_volume_dir()
    {
        test.env.push((OTHER_VOLUME_VAR, dir.display().to_string()));
    }
    test
}

/// 임시 폴더와 다른 드라이브에 있는 작업 공간의 `target/other-volume`. 같은 드라이브거나 만들지 못하면 `None`.
fn other_volume_dir() -> Option<PathBuf> {
    let workspace = std::path::absolute(env!("CARGO_MANIFEST_DIR"))
        .ok()?
        .parent()?
        .to_path_buf();
    let drive = |path: &std::path::Path| match path.components().next() {
        Some(std::path::Component::Prefix(prefix)) => {
            Some(prefix.as_os_str().to_string_lossy().to_uppercase())
        }
        _ => None,
    };
    let temp = std::path::absolute(env::temp_dir()).ok()?;
    if drive(&workspace)? == drive(&temp)? {
        return None;
    }
    let dir = workspace.join("target").join("other-volume");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn bench_steps() -> Vec<Step> {
    vec![
        bench_step(),
        index_bench_step(),
        mft_bench_step(),
        session_bench_step(),
        clipboard_bench_step(),
        menu_bench_step(),
        batch_rename_bench_step(),
        filter_bench_step(),
        query_syntax_bench_step(),
        search_bench_step(),
        content_bench_step(),
        copy_bench_step(),
        delete_bench_step(),
    ]
}

/// 릴리스 빌드한 `facet-bench`를 `cargo run`으로 부르는 인자.
fn facet_bench(command: &str) -> Vec<String> {
    [
        "run",
        "--release",
        "--locked",
        "--quiet",
        "-p",
        "facet-bench",
        "--",
        command,
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect()
}

/// 시험 폴더의 기본 위치: 임시 폴더 아래 `facet-test-folders/items-N`.
///
/// `target/` 밑에 두지 않는다. CI의 rust-cache가 `target/`을 캐시하므로
/// 10만 개 파일이 캐시에 섞여 들어가 저장·복원이 느려진다.
fn default_folder(count: &str) -> PathBuf {
    env::temp_dir()
        .join("facet-test-folders")
        .join(format!("items-{count}"))
}

fn bench_step() -> Step {
    let mut args = facet_bench("list");
    args.push(default_folder(BENCH_COUNT).display().to_string());
    args.extend(["--count".to_owned(), BENCH_COUNT.to_owned()]);
    Step {
        name: "코어 벤치마크",
        args,
        env: Vec::new(),
    }
}

/// 합성 인덱스 100만 개의 메모리와 부분 일치 질의를 잰다. 수와 상한은 `facet-bench`의 기본값이다.
fn index_bench_step() -> Step {
    Step {
        name: "인덱스 벤치마크",
        args: facet_bench("index"),
        env: Vec::new(),
    }
}

/// MFT 조각을 되풀이한 레코드 100만 개를 인덱스로 만드는 시간을 잰다. 수와 상한은 `facet-bench`의 기본값이다.
fn mft_bench_step() -> Step {
    Step {
        name: "MFT 파싱 벤치마크",
        args: facet_bench("mft"),
        env: Vec::new(),
    }
}

/// 탭 100개 세션 파일을 읽고 해석하는 시간을 잰다. 상한은 `facet-bench`의 기본값이다.
fn session_bench_step() -> Step {
    Step {
        name: "세션 읽기·해석 벤치마크",
        args: facet_bench("session"),
        env: Vec::new(),
    }
}

/// 파일 1만 개 목록의 클립보드 형식(`CF_HDROP`) 만들기·읽기 시간을 잰다. 상한은 `facet-bench`의 기본값이다.
fn clipboard_bench_step() -> Step {
    Step {
        name: "클립보드 형식 벤치마크",
        args: facet_bench("clipboard"),
        env: Vec::new(),
    }
}

/// 선택 1만 개에서 오른쪽 메뉴(항목 메뉴·빈 곳 메뉴)를 만드는 시간을 잰다. 상한은 `facet-bench`의 기본값이다.
fn menu_bench_step() -> Step {
    Step {
        name: "오른쪽 메뉴 구성 벤치마크",
        args: facet_bench("menu"),
        env: Vec::new(),
    }
}

/// 선택 1만 개의 일괄 이름 바꾸기 미리 보기 계산 시간을 잰다. 상한은 `facet-bench`의 기본값이다.
fn batch_rename_bench_step() -> Step {
    Step {
        name: "일괄 이름 바꾸기 미리 보기",
        args: facet_bench("batch-rename"),
        env: Vec::new(),
    }
}

/// 10만 항목 목록에서 필터 한 글자·두 글자를 정하는 시간을 잰다. 상한은 `facet-bench`의 기본값이다.
fn filter_bench_step() -> Step {
    Step {
        name: "필터 벤치마크",
        args: facet_bench("filter"),
        env: Vec::new(),
    }
}

/// 연산자·조건을 고루 쓴 긴 질의 하나를 해석하는 시간을 잰다. 상한은 `facet-bench`의 기본값이다.
fn query_syntax_bench_step() -> Step {
    Step {
        name: "질의 문법 해석 벤치마크",
        args: facet_bench("query-syntax"),
        env: Vec::new(),
    }
}

/// 10만 항목 시험 나무(임시 폴더의 `facet-test-folders/tree-100000`)를 만들거나 확인하고 실시간 탐색 전체 시간을
/// 잰다. 수와 상한은 `facet-bench`의 기본값이다.
fn search_bench_step() -> Step {
    Step {
        name: "실시간 탐색 벤치마크",
        args: facet_bench("search"),
        env: Vec::new(),
    }
}

/// 임시 폴더의 본문 128MB 시험 나무(`facet-test-folders/content-search`)를 만들거나 확인하고 내용 검색 처리량을
/// 잰다. 하한은 `facet-bench`의 기본값이다.
fn content_bench_step() -> Step {
    Step {
        name: "내용 검색 벤치마크",
        args: facet_bench("content"),
        env: Vec::new(),
    }
}

/// 임시 폴더에서 256MiB 파일 하나의 복사 처리량과 4KiB 파일 2천 개 폴더의 복사 시간을 잰다(작업 대기열의 복사
/// 작업). 크기·수·한도는 `facet-bench`의 기본값이다.
fn copy_bench_step() -> Step {
    Step {
        name: "복사 벤치마크",
        args: facet_bench("copy"),
        env: Vec::new(),
    }
}

/// 임시 폴더에서 4KiB 파일 1만 개 폴더의 영구 삭제 시간(작업 대기열의 영구 삭제 작업)과 작업 기록 100개에서
/// 실행취소 대상을 찾는 시간을 잰다. 수와 상한은 `facet-bench`의 기본값이다.
fn delete_bench_step() -> Step {
    Step {
        name: "삭제 벤치마크",
        args: facet_bench("delete"),
        env: Vec::new(),
    }
}

fn test_folder_step(rest: &[String]) -> Result<Step, String> {
    let mut path = None;
    let mut count = BENCH_COUNT.to_owned();
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--count" => {
                let v = it.next().ok_or("--count 뒤에 값이 없습니다")?;
                v.parse::<usize>()
                    .map_err(|_| format!("--count의 값이 수가 아닙니다: {v}"))?;
                count = v.clone();
            }
            _ if path.is_none() && !arg.starts_with("--") => path = Some(PathBuf::from(arg)),
            _ => return Err(format!("알 수 없는 인자: {arg}")),
        }
    }
    // 상대 경로는 명령을 부른 폴더 기준이다(단계는 작업 공간 루트에서 돌므로 여기서 푼다).
    let path = match path {
        Some(p) if p.is_relative() => env::current_dir().map_err(|e| e.to_string())?.join(p),
        Some(p) => p,
        None => default_folder(&count),
    };
    let mut args = facet_bench("make-folder");
    args.push(path.display().to_string());
    args.extend(["--count".to_owned(), count]);
    Ok(Step {
        name: "시험 폴더 생성",
        args,
        env: Vec::new(),
    })
}

/// `dist` 뒤의 인자에서 label 하나를 꺼내 검사한다.
fn dist_label(rest: &[String]) -> Result<&str, String> {
    match rest {
        [label] => dist::validate_label(label).map(|()| label.as_str()),
        [] => Err("label이 없습니다".to_owned()),
        [_, extra, ..] => Err(format!("알 수 없는 인자: {extra}")),
    }
}

/// `target/dist/`를 비우고, facet을 release로 빌드하고, zip과 체크섬을 만든다.
///
/// 폴더를 먼저 비우므로 빌드가 실패해도 앞선 산출물이나 반쪽 파일이 남지 않는다.
fn run_dist(label: &str) -> ExitCode {
    let root = workspace_root();
    let dist_dir = root.join("target").join("dist");
    if let Err(e) = dist::clean_dir(&dist_dir) {
        eprintln!("xtask dist: {}를 비우지 못함: {e}", dist_dir.display());
        return ExitCode::FAILURE;
    }

    let build = step(
        "release 빌드",
        &["build", "--release", "--locked", "-p", "facet-app"],
    );
    if run_steps("dist", &[build]) != ExitCode::SUCCESS {
        return ExitCode::FAILURE;
    }

    let target_dir = target_dir();
    let entries = [
        dist::Entry {
            name: "facet.exe",
            source: target_dir
                .join("release")
                .join(format!("facet{}", env::consts::EXE_SUFFIX)),
        },
        dist::Entry {
            name: "LICENSE",
            source: root.join("LICENSE"),
        },
        dist::Entry {
            name: "NOTICE.txt",
            source: root.join("NOTICE.txt"),
        },
    ];
    match dist::package(&dist_dir, label, &entries) {
        Ok(zip) => {
            println!("xtask dist: {} 와 .sha256을 만듦", zip.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("xtask dist: 묶기 실패: {e}");
            ExitCode::FAILURE
        }
    }
}

/// 빌드 산출물 폴더(`CARGO_TARGET_DIR`, 없으면 `target/`).
fn target_dir() -> PathBuf {
    let root = workspace_root();
    env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .map(|p| if p.is_relative() { root.join(p) } else { p })
        .unwrap_or_else(|| root.join("target"))
}

/// `indexer`의 인자: `install` 또는 `uninstall` 하나.
fn indexer_action(rest: &[String]) -> Result<&str, String> {
    match rest {
        [a] if a == "install" || a == "uninstall" => Ok(a.as_str()),
        [other] => Err(format!(
            "indexer 뒤에는 install이나 uninstall이 옵니다: {other}"
        )),
        [] => Err("indexer 뒤에 install이나 uninstall이 필요합니다".to_owned()),
        [_, extra, ..] => Err(format!("알 수 없는 인자: {extra}")),
    }
}

/// `indexer install`: facet-indexer를 release로 빌드하고 그 실행 파일의 `install`을 부른다.
/// `indexer uninstall`: release 실행 파일의 `uninstall`을 부른다(실행 파일이 없을 때만 빌드한다.
/// 서비스가 도는 동안에는 실행 파일이 잠겨 다시 빌드하지 못한다).
fn run_indexer(action: &str) -> ExitCode {
    let exe = target_dir()
        .join("release")
        .join(format!("facet-indexer{}", env::consts::EXE_SUFFIX));
    if action == "install" || !exe.exists() {
        let build = step(
            "release 빌드",
            &["build", "--release", "--locked", "-p", "facet-indexer"],
        );
        if run_steps("indexer", &[build]) != ExitCode::SUCCESS {
            eprintln!(
                "xtask indexer: 서비스가 이미 설치돼 돌고 있으면 실행 파일이 잠겨 빌드하지 못합니다. \
                 cargo xtask indexer uninstall을 먼저 하십시오"
            );
            return ExitCode::FAILURE;
        }
    }
    println!("\n=== xtask indexer: {} {action} ===", exe.display());
    match Command::new(&exe).arg(action).status() {
        Ok(s) if s.success() => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("xtask indexer: {}를 실행하지 못함: {e}", exe.display());
            ExitCode::FAILURE
        }
    }
}

fn run_steps(task: &str, steps: &[Step]) -> ExitCode {
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let root = workspace_root();

    for step in steps {
        println!(
            "\n=== xtask {task}: {} (cargo {}) ===",
            step.name,
            step.args.join(" ")
        );
        let status = Command::new(&cargo)
            .args(&step.args)
            .envs(step.env.iter().map(|(k, v)| (k, v)))
            .current_dir(&root)
            .status();
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => {
                eprintln!("\nxtask {task}: {} 단계 실패 ({s})", step.name);
                return ExitCode::FAILURE;
            }
            Err(e) => {
                eprintln!("\nxtask {task}: {} 단계를 실행하지 못함: {e}", step.name);
                return ExitCode::FAILURE;
            }
        }
    }

    println!("\nxtask {task}: 모든 단계 통과");
    ExitCode::SUCCESS
}

/// 작업 공간 루트. xtask 크레이트의 부모 폴더다.
fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .map(PathBuf::from)
        .unwrap_or(manifest_dir)
}

/// 앱과 측정 도구를 릴리스 빌드하고 `facet-bench screen`을 돌린다.
///
/// 받은 인자는 그대로 넘기되, 경로·`--count`·`--json`을 비우면 기본값(10만 항목 시험 폴더,
/// `target/measure.json`)을 채운다. 상대 경로는 명령을 부른 폴더 기준이다.
fn run_measure(rest: &[String]) -> ExitCode {
    let root = workspace_root();
    let build = step(
        "release 빌드",
        &[
            "build",
            "--release",
            "--locked",
            "-p",
            "facet-app",
            "-p",
            "facet-bench",
        ],
    );
    if run_steps("measure", &[build]) != ExitCode::SUCCESS {
        return ExitCode::FAILURE;
    }
    let args = match measure_args(rest, &root) {
        Ok(a) => a,
        Err(e) => {
            eprintln!(
                "{e}

{USAGE}"
            );
            return ExitCode::FAILURE;
        }
    };
    let target_dir = env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .map(|p| if p.is_relative() { root.join(p) } else { p })
        .unwrap_or_else(|| root.join("target"));
    let release = target_dir.join("release");
    let status = Command::new(release.join("facet-bench.exe"))
        .arg("screen")
        .args(&args)
        .arg("--app")
        .arg(release.join("facet.exe"))
        .status();
    match status {
        Ok(s) if s.success() => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("xtask measure: facet-bench를 실행하지 못함: {e}");
            ExitCode::FAILURE
        }
    }
}

/// 측정 도구를 릴리스 빌드하고 `facet-bench compare-copy`를 돌린다(탐색기 비교 측정).
fn run_measure_copy(rest: &[String]) -> ExitCode {
    let root = workspace_root();
    let args = match measure_copy_args(rest, &root) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    let build = step(
        "release 빌드",
        &["build", "--release", "--locked", "-p", "facet-bench"],
    );
    if run_steps("measure-copy", &[build]) != ExitCode::SUCCESS {
        return ExitCode::FAILURE;
    }
    let bench = target_dir()
        .join("release")
        .join(format!("facet-bench{}", env::consts::EXE_SUFFIX));
    match Command::new(&bench)
        .arg("compare-copy")
        .args(&args)
        .status()
    {
        Ok(s) if s.success() => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("xtask measure-copy: facet-bench를 실행하지 못함: {e}");
            ExitCode::FAILURE
        }
    }
}

/// 측정 도구를 릴리스 빌드하고 `facet-bench compare-recycle`을 돌린다(휴지통 보내기 비교 측정).
fn run_measure_recycle(rest: &[String]) -> ExitCode {
    let root = workspace_root();
    let args = match measure_recycle_args(rest, &root) {
        Ok(a) => a,
        Err(e) => {
            eprintln!(
                "{e}

{USAGE}"
            );
            return ExitCode::FAILURE;
        }
    };
    let build = step(
        "release 빌드",
        &["build", "--release", "--locked", "-p", "facet-bench"],
    );
    if run_steps("measure-recycle", &[build]) != ExitCode::SUCCESS {
        return ExitCode::FAILURE;
    }
    let bench = target_dir()
        .join("release")
        .join(format!("facet-bench{}", env::consts::EXE_SUFFIX));
    match Command::new(&bench)
        .arg("compare-recycle")
        .args(&args)
        .status()
    {
        Ok(s) if s.success() => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("xtask measure-recycle: facet-bench를 실행하지 못함: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `measure-recycle`의 인자: 폴더 하나(상대 경로는 명령을 부른 폴더 기준)와 넘길 선택 인자. `--json`을 빼면
/// `target/measure-recycle.json`.
fn measure_recycle_args(rest: &[String], root: &std::path::Path) -> Result<Vec<String>, String> {
    let cwd = env::current_dir().map_err(|e| e.to_string())?;
    let absolute = |p: &str| {
        let p = PathBuf::from(p);
        if p.is_relative() { cwd.join(p) } else { p }
    };
    let mut folder = None;
    let mut json = None;
    let mut passthrough = Vec::new();
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--json" | "--count" | "--runs" => {
                let v = it.next().ok_or(format!("{arg} 뒤에 값이 없습니다"))?;
                if arg == "--json" {
                    json = Some(absolute(v));
                } else {
                    passthrough.extend([arg.clone(), v.clone()]);
                }
            }
            _ if folder.is_none() && !arg.starts_with("--") => folder = Some(absolute(arg)),
            _ => return Err(format!("알 수 없는 인자: {arg}")),
        }
    }
    let folder = folder.ok_or("폴더를 주어야 합니다")?;
    let json = json.unwrap_or_else(|| root.join("target").join("measure-recycle.json"));
    let mut args = vec![folder.display().to_string()];
    args.extend(["--json".to_owned(), json.display().to_string()]);
    args.extend(passthrough);
    Ok(args)
}

/// `measure-copy`의 인자: 원본·대상 폴더 둘(상대 경로는 명령을 부른 폴더 기준)과 넘길 선택 인자. `--json`을
/// 빼면 `target/measure-copy.json`.
fn measure_copy_args(rest: &[String], root: &std::path::Path) -> Result<Vec<String>, String> {
    let cwd = env::current_dir().map_err(|e| e.to_string())?;
    let absolute = |p: &str| {
        let p = PathBuf::from(p);
        if p.is_relative() { cwd.join(p) } else { p }
    };
    let mut folders = Vec::new();
    let mut json = None;
    let mut passthrough = Vec::new();
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--json" | "--size-mb" | "--count" | "--runs" => {
                let v = it.next().ok_or(format!("{arg} 뒤에 값이 없습니다"))?;
                if arg == "--json" {
                    json = Some(absolute(v));
                } else {
                    passthrough.extend([arg.clone(), v.clone()]);
                }
            }
            _ if folders.len() < 2 && !arg.starts_with("--") => folders.push(absolute(arg)),
            _ => return Err(format!("알 수 없는 인자: {arg}")),
        }
    }
    let [source, target] = <[PathBuf; 2]>::try_from(folders)
        .map_err(|_| "원본 폴더와 대상 폴더를 주어야 합니다".to_owned())?;
    let json = json.unwrap_or_else(|| root.join("target").join("measure-copy.json"));
    let mut args = vec![source.display().to_string(), target.display().to_string()];
    args.extend(["--json".to_owned(), json.display().to_string()]);
    args.extend(passthrough);
    Ok(args)
}

/// `measure`의 인자에 빠진 기본값(경로, `--count`, `--json`)을 채운다.
fn measure_args(rest: &[String], root: &std::path::Path) -> Result<Vec<String>, String> {
    let cwd = env::current_dir().map_err(|e| e.to_string())?;
    let absolute = |p: &str| {
        let p = PathBuf::from(p);
        if p.is_relative() { cwd.join(p) } else { p }
    };
    let mut path = None;
    let mut count = BENCH_COUNT.to_owned();
    let mut json = None;
    let mut passthrough = Vec::new();
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--count" | "--json" | "--runs" | "--scroll-runs" | "--scroll-rows"
            | "--refresh-hz" => {
                let v = it.next().ok_or(format!("{arg} 뒤에 값이 없습니다"))?;
                match arg.as_str() {
                    "--count" => count = v.clone(),
                    "--json" => json = Some(absolute(v)),
                    _ => passthrough.extend([arg.clone(), v.clone()]),
                }
            }
            _ if path.is_none() && !arg.starts_with("--") => path = Some(absolute(arg)),
            _ => return Err(format!("알 수 없는 인자: {arg}")),
        }
    }
    let path = path.unwrap_or_else(|| default_folder(&count));
    let json = json.unwrap_or_else(|| root.join("target").join("measure.json"));
    let mut args = vec![path.display().to_string()];
    args.extend(["--count".to_owned(), count]);
    args.extend(["--json".to_owned(), json.display().to_string()]);
    args.extend(passthrough);
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::{dist_label, indexer_action, measure_copy_args, measure_recycle_args};

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn dist_takes_exactly_one_safe_label() {
        assert_eq!(dist_label(&args(&["nightly"])), Ok("nightly"));
        assert_eq!(dist_label(&args(&["v0.1.0"])), Ok("v0.1.0"));
        assert!(dist_label(&args(&[])).is_err());
        assert!(dist_label(&args(&["a/b"])).is_err());
        assert!(dist_label(&args(&["a b"])).is_err());
        assert!(dist_label(&args(&["nightly", "extra"])).is_err());
    }

    #[test]
    fn measure_copy_needs_two_folders_and_fills_json() {
        let root = std::path::Path::new(r"C:\repo");
        let got = measure_copy_args(&args(&[r"C:\a", r"D:\b", "--runs", "3"]), root).unwrap();
        assert_eq!(got[0], r"C:\a");
        assert_eq!(got[1], r"D:\b");
        assert_eq!(got[2], "--json");
        assert!(got[3].ends_with("measure-copy.json"), "{got:?}");
        assert_eq!(&got[4..], ["--runs", "3"]);
        assert!(measure_copy_args(&args(&[r"C:\a"]), root).is_err());
        assert!(measure_copy_args(&args(&[r"C:\a", r"D:\b", r"E:\c"]), root).is_err());
        assert!(measure_copy_args(&args(&[r"C:\a", r"D:\b", "--limit-ms", "1"]), root).is_err());
    }

    #[test]
    fn measure_recycle_needs_one_folder_and_fills_json() {
        let root = std::path::Path::new(r"C:\repo");
        let got = measure_recycle_args(&args(&[r"C:\a", "--count", "50"]), root).unwrap();
        assert_eq!(got[0], r"C:\a");
        assert_eq!(got[1], "--json");
        assert!(got[2].ends_with("measure-recycle.json"), "{got:?}");
        assert_eq!(&got[3..], ["--count", "50"]);
        assert!(measure_recycle_args(&args(&[]), root).is_err());
        assert!(measure_recycle_args(&args(&[r"C:\a", r"D:\b"]), root).is_err());
    }

    #[test]
    fn indexer_takes_install_or_uninstall() {
        assert_eq!(indexer_action(&args(&["install"])), Ok("install"));
        assert_eq!(indexer_action(&args(&["uninstall"])), Ok("uninstall"));
        assert!(indexer_action(&args(&[])).is_err());
        assert!(indexer_action(&args(&["start"])).is_err());
        assert!(indexer_action(&args(&["install", "x"])).is_err());
    }
}
