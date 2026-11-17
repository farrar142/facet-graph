// target: crates/facet-core/tests/spec_batch_rename_preview.rs
// AC: AC-1, AC-2, AC-3, AC-4, AC-5, AC-6, AC-7, AC-8, AC-9, AC-10
//! 일괄 이름 바꾸기의 규칙과 미리 보기(`batch_rename::preview`, 순수 함수)를 확인한다: 줄기·확장자, 찾아 바꾸기·지우기,
//! 번호 매기기, 항목 차례, 겹침(선택 안·남는 이름)과 쓸 수 없는 이름, 사슬·대소문자만 바꾸기, 그대로인 항목.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use facet_core::batch_rename::{
    BatchItem, BatchProblem, EntryStatus, ExistingNames, NumberPosition, Numbering, Preview,
    RenameRule, preview,
};
use facet_core::name_input::NameError;

fn file(folder: &str, name: &str) -> BatchItem {
    BatchItem {
        folder: PathBuf::from(folder),
        name: OsString::from(name),
        is_folder: false,
    }
}

fn dir(folder: &str, name: &str) -> BatchItem {
    BatchItem {
        is_folder: true,
        ..file(folder, name)
    }
}

fn run(items: &[BatchItem], rule: &RenameRule) -> Preview {
    preview(items, rule, &ExistingNames::new())
}

fn new_names(preview: &Preview) -> Vec<&str> {
    preview
        .entries()
        .iter()
        .map(|entry| entry.new_name.as_str())
        .collect()
}

fn statuses(preview: &Preview) -> Vec<EntryStatus> {
    preview.entries().iter().map(|entry| entry.status).collect()
}

/// 항목 하나에 규칙을 써 본 상태.
fn status_of(item: BatchItem, rule: &RenameRule) -> EntryStatus {
    run(&[item], rule).entries()[0].status
}

fn affix(prefix: &str, suffix: &str) -> RenameRule {
    RenameRule {
        prefix: prefix.to_string(),
        suffix: suffix.to_string(),
        ..RenameRule::default()
    }
}

fn replace(find: &str, with: &str) -> RenameRule {
    RenameRule {
        find: find.to_string(),
        replace: with.to_string(),
        ..RenameRule::default()
    }
}

fn numbering(start: u64, digits: usize, position: NumberPosition, separator: &str) -> Numbering {
    Numbering {
        start,
        digits,
        position,
        separator: separator.to_string(),
    }
}

#[test]
fn affixes_go_around_the_stem_and_keep_the_extension() {
    let items = [
        file("F", "a.txt"),
        file("F", "b.tar.gz"),
        file("F", "README"),
        file("F", ".gitignore"),
        dir("F", "my.dir"),
    ];
    let result = run(&items, &affix("x_", "_y"));
    assert_eq!(
        new_names(&result),
        [
            "x_a_y.txt",
            "x_b.tar_y.gz",
            "x_README_y",
            "x_.gitignore_y",
            "x_my.dir_y"
        ]
    );
    assert!(statuses(&result).iter().all(|s| *s == EntryStatus::Rename));
    assert_eq!(result.rename_count(), 5);
    assert!(result.can_execute());
}

#[test]
fn find_replaces_every_match_ignoring_case_only_in_the_stem() {
    let result = run(&[file("F", "IMG_img_Img.img")], &replace("img", "사진"));
    assert_eq!(new_names(&result), ["사진_사진_사진.img"]);

    let result = run(&[file("F", "보고서 (1).docx")], &replace(" (1)", ""));
    assert_eq!(
        new_names(&result),
        ["보고서.docx"],
        "바꿀 글자가 비면 지운다"
    );

    // 붙인 글자에는 찾아 바꾸기를 쓰지 않는다.
    let rule = RenameRule {
        prefix: "a".to_string(),
        suffix: "a".to_string(),
        ..replace("a", "b")
    };
    let result = run(&[file("F", "a.txt")], &rule);
    assert_eq!(new_names(&result), ["aba.txt"]);
}

#[test]
fn numbering_follows_the_item_order_with_padding_and_separator() {
    let items = [file("F", "a.jpg"), file("F", "b.jpg"), file("F", "c.jpg")];
    let rule = RenameRule {
        numbering: Some(numbering(1, 3, NumberPosition::End, "_")),
        ..RenameRule::default()
    };
    assert_eq!(
        new_names(&run(&items, &rule)),
        ["a_001.jpg", "b_002.jpg", "c_003.jpg"]
    );

    let rule = RenameRule {
        prefix: "휴가 ".to_string(),
        numbering: Some(numbering(9, 2, NumberPosition::Start, "-")),
        ..RenameRule::default()
    };
    assert_eq!(
        new_names(&run(&items[..2], &rule)),
        ["휴가 09-a.jpg", "휴가 10-b.jpg"],
        "번호는 앞에 붙인 글자 뒤, 줄기 앞이다"
    );

    let rule = RenameRule {
        suffix: "!".to_string(),
        numbering: Some(numbering(99, 2, NumberPosition::End, "")),
        ..RenameRule::default()
    };
    assert_eq!(
        new_names(&run(&[file("F", "a"), file("F", "b")], &rule)),
        ["a99!", "b100!"],
        "자릿수를 넘으면 그대로 길어지고 번호는 뒤에 붙인 글자 앞이다"
    );

    let rule = RenameRule {
        numbering: Some(numbering(7, 0, NumberPosition::End, " ")),
        ..RenameRule::default()
    };
    assert_eq!(new_names(&run(&[dir("F", "폴더")], &rule)), ["폴더 7"]);
}

#[test]
fn entries_follow_the_items_with_folder_and_original_name() {
    let items = [file("F", "b.txt"), file("G", "a.txt"), dir("F", "c")];
    let result = run(&items, &affix("새 ", ""));
    let entries = result.entries();
    assert_eq!(entries.len(), 3);
    for (entry, item) in entries.iter().zip(&items) {
        assert_eq!(entry.folder, item.folder);
        assert_eq!(entry.original, item.name);
    }
    assert_eq!(new_names(&result), ["새 b.txt", "새 a.txt", "새 c"]);
}

#[test]
fn same_new_name_in_one_folder_is_a_duplicate_but_not_across_folders() {
    let items = [
        file("F", "ax.txt"),
        file("F", "xA.TXT"),
        file("G", "ax.txt"),
    ];
    let result = run(&items, &replace("x", ""));
    assert_eq!(new_names(&result), ["a.txt", "A.TXT", "a.txt"]);
    assert_eq!(
        statuses(&result),
        [
            EntryStatus::Problem(BatchProblem::Duplicate),
            EntryStatus::Problem(BatchProblem::Duplicate),
            EntryStatus::Rename,
        ],
        "대소문자만 다른 이름도 같은 폴더면 겹친다. 다른 폴더는 따로다"
    );
    assert_eq!(result.problem_count(), 2);
    assert!(!result.can_execute());
}

#[test]
fn a_name_that_stays_in_the_folder_is_taken() {
    // 선택하지 않은 항목의 이름.
    let mut existing = ExistingNames::new();
    existing.insert(Path::new("F"), OsStr::new("b.txt"));
    existing.insert(Path::new("F"), OsStr::new("a.txt"));
    let result = preview(&[file("F", "a.txt")], &replace("a", "B"), &existing);
    assert_eq!(new_names(&result), ["B.txt"]);
    assert_eq!(
        statuses(&result),
        [EntryStatus::Problem(BatchProblem::NameTaken)]
    );
    assert!(!result.can_execute());

    // 다른 폴더의 같은 이름은 상관없다.
    let result = preview(&[file("G", "a.txt")], &replace("a", "b"), &existing);
    assert_eq!(statuses(&result), [EntryStatus::Rename]);

    // 규칙이 바꾸지 않아 그대로 남는 선택 항목의 이름.
    let items = [file("F", "보고서.txt"), file("F", "보고서 (1).txt")];
    let result = run(&items, &replace(" (1)", ""));
    assert_eq!(
        statuses(&result),
        [
            EntryStatus::Unchanged,
            EntryStatus::Problem(BatchProblem::NameTaken)
        ]
    );
    assert_eq!(result.problem_count(), 1);
    assert!(!result.can_execute());
}

#[test]
fn names_windows_cannot_use_are_problems_with_the_reason() {
    let invalid = |reason| EntryStatus::Problem(BatchProblem::Invalid(reason));
    assert_eq!(
        status_of(file("F", "f.txt"), &affix("a:", "")),
        invalid(NameError::ForbiddenChar(':'))
    );
    assert_eq!(
        status_of(file("F", "f.txt"), &affix("\u{1}", "")),
        invalid(NameError::ControlChar('\u{1}'))
    );
    assert_eq!(
        status_of(dir("F", "CO"), &affix("", "N")),
        invalid(NameError::Reserved)
    );
    assert_eq!(
        status_of(dir("F", "abc"), &affix("", ".")),
        invalid(NameError::TrailingDot)
    );
    assert_eq!(
        status_of(dir("F", "abc"), &affix("", " ")),
        invalid(NameError::TrailingSpace)
    );
    assert_eq!(
        status_of(dir("F", "abc"), &replace("ABC", "")),
        invalid(NameError::Empty)
    );
    let long = "x".repeat(300);
    assert_eq!(
        status_of(file("F", "a.txt"), &affix(&long, "")),
        invalid(NameError::TooLong { units: 305 })
    );
    let result = run(&[file("F", "f.txt"), file("F", "g.txt")], &affix("a|", ""));
    assert_eq!(result.problem_count(), 2);
    assert_eq!(result.rename_count(), 0);
    assert!(!result.can_execute());
}

#[test]
fn chains_and_case_only_changes_are_not_conflicts() {
    // a → aa 인데 aa는 aaaa로 비워진다.
    let result = run(
        &[file("F", "a.txt"), file("F", "aa.txt")],
        &replace("a", "aa"),
    );
    assert_eq!(new_names(&result), ["aa.txt", "aaaa.txt"]);
    assert_eq!(
        statuses(&result),
        [EntryStatus::Rename, EntryStatus::Rename]
    );
    assert!(result.can_execute());

    // 반대 방향 사슬: aaaa → aa 인데 aa는 a로 비워진다.
    let result = run(
        &[file("F", "aa.txt"), file("F", "aaaa.txt")],
        &replace("aa", "a"),
    );
    assert_eq!(new_names(&result), ["a.txt", "aa.txt"]);
    assert!(result.can_execute());

    // 대소문자만 바꾸기는 같은 항목이다(제 이름이 이미 있다고 적혀 있어도).
    let mut existing = ExistingNames::new();
    existing.insert(Path::new("F"), OsStr::new("readme.md"));
    existing.insert(Path::new("F"), OsStr::new("other.md"));
    let result = preview(
        &[file("F", "readme.md")],
        &replace("readme", "README"),
        &existing,
    );
    assert_eq!(new_names(&result), ["README.md"]);
    assert_eq!(statuses(&result), [EntryStatus::Rename]);
    assert!(result.can_execute());
}

#[test]
fn items_the_rule_does_not_change_are_skipped_and_counted() {
    let items = [file("F", "a.txt"), file("F", "bzzz.txt"), dir("F", "c")];
    let result = run(&items, &replace("ZZZ", "y"));
    assert_eq!(
        statuses(&result),
        [
            EntryStatus::Unchanged,
            EntryStatus::Rename,
            EntryStatus::Unchanged
        ]
    );
    assert_eq!(new_names(&result), ["a.txt", "by.txt", "c"]);
    assert_eq!((result.rename_count(), result.unchanged_count()), (1, 2));
    assert!(result.can_execute());

    let result = run(&items, &RenameRule::default());
    assert_eq!(result.unchanged_count(), 3);
    assert_eq!(result.rename_count(), 0);
    assert!(!result.can_execute(), "바꿀 것이 없으면 실행할 수 없다");
}

#[test]
fn ten_thousand_items_preview_quickly() {
    let items: Vec<BatchItem> = (0..10_000)
        .map(|n| file("F", &format!("사진 {n}.jpg")))
        .collect();
    let mut existing = ExistingNames::new();
    for item in &items {
        existing.insert(&item.folder, &item.name);
    }
    let rule = RenameRule {
        find: "사진".to_string(),
        replace: "휴가".to_string(),
        prefix: "2026 ".to_string(),
        suffix: "".to_string(),
        numbering: Some(numbering(1, 5, NumberPosition::End, "_")),
    };
    let start = Instant::now();
    let result = preview(&items, &rule, &existing);
    // 넉넉한 상한(디버그 빌드에서도). 속도 예산은 벤치마크가 지킨다.
    assert!(start.elapsed() < Duration::from_secs(2));
    assert_eq!(result.rename_count(), 10_000);
    assert_eq!(result.entries()[0].new_name, "2026 휴가 0_00001.jpg");
    assert!(result.can_execute());
}
