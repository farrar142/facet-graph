//! 일괄 이름 바꾸기의 규칙과 미리 보기(순수 함수, 화면·파일 시스템·시계를 부르지 않음, ADR-0007).
//!
//! 규칙은 찾아 바꾸기 → 앞·뒤 붙이기 → 번호 매기기를 파일의 줄기(확장자 앞)에만 쓴다.
//! 겹침 검사는 폴더별 해시 집합이라 항목·이름 수에 거의 비례한다.

use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use crate::name_input::{NameError, validate_name};

/// 이름 규칙. 쓰는 차례: 찾아 바꾸기 → 붙이기·번호. 모든 필드가 비면(기본값) 아무 이름도 바꾸지 않는다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RenameRule {
    /// 찾을 글자. 비면 찾아 바꾸기를 하지 않는다.
    pub find: String,
    /// 바꿀 글자. 비면 찾은 글자를 지운다.
    pub replace: String,
    /// 앞에 붙일 글자.
    pub prefix: String,
    /// 뒤에(확장자 앞에) 붙일 글자.
    pub suffix: String,
    /// 번호 매기기. `None`이면 번호를 붙이지 않는다.
    pub numbering: Option<Numbering>,
}

/// 번호 매기기 설정.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Numbering {
    /// 첫 항목의 번호. 항목 목록의 i번째(0부터) 항목은 `start + i`다.
    pub start: u64,
    /// 최소 자릿수. 모자라면 앞을 0으로 채운다(0·1이면 채우지 않음, 넘치면 그대로 길어짐).
    pub digits: usize,
    /// 번호를 줄기의 앞에 둘지 뒤에 둘지.
    pub position: NumberPosition,
    /// 번호와 줄기 사이에 넣을 글자(비면 붙여 씀).
    pub separator: String,
}

/// 번호의 자리.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberPosition {
    Start,
    End,
}

/// 바꿀 항목 하나: 든 폴더, 원래 이름, 폴더인가.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchItem {
    pub folder: PathBuf,
    pub name: OsString,
    pub is_folder: bool,
}

/// 겹침을 볼 때 쓰는 폴더마다의 이름들(바꿀 항목이 아닌 다른 항목 포함, 숨김 항목 포함).
#[derive(Debug, Clone, Default)]
pub struct ExistingNames {
    folders: HashMap<String, HashSet<String>>,
}

impl ExistingNames {
    pub fn new() -> Self {
        Self::default()
    }

    /// `folder`에 `name` 항목이 있다고 적는다. 같은 이름(대소문자 무시)을 여러 번 넣어도 하나다.
    pub fn insert(&mut self, folder: &Path, name: &OsStr) {
        self.folders
            .entry(folder_key(folder))
            .or_default()
            .insert(name_key(&name.to_string_lossy()));
    }
}

/// 규칙을 쓴 결과(아무것도 바꾸지 않는다).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    entries: Vec<PreviewEntry>,
    renames: usize,
    unchanged: usize,
    problems: usize,
}

impl Preview {
    /// 항목마다(넘긴 차례 그대로) 결과.
    pub fn entries(&self) -> &[PreviewEntry] {
        &self.entries
    }

    /// 상태가 `EntryStatus::Rename`인 항목 수.
    pub fn rename_count(&self) -> usize {
        self.renames
    }

    /// 상태가 `EntryStatus::Unchanged`인 항목 수(건너뛸 항목).
    pub fn unchanged_count(&self) -> usize {
        self.unchanged
    }

    /// 상태가 `EntryStatus::Problem`인 항목 수.
    pub fn problem_count(&self) -> usize {
        self.problems
    }

    /// 실행할 수 있는가: 문제가 하나도 없고 바꿀 항목이 하나 이상.
    pub fn can_execute(&self) -> bool {
        self.problems == 0 && self.renames > 0
    }
}

/// 항목 하나의 미리 보기.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewEntry {
    pub folder: PathBuf,
    pub original: OsString,
    /// 바뀔 이름(그대로면 원래 이름과 같다).
    pub new_name: String,
    pub status: EntryStatus,
}

/// 항목의 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryStatus {
    /// 이름이 바뀐다.
    Rename,
    /// 규칙이 이름을 바꾸지 않는다(바뀔 이름이 원래 이름과 글자 그대로 같음). 건너뛴다.
    Unchanged,
    /// 바꿀 수 없다. 하나라도 있으면 실행할 수 없다.
    Problem(BatchProblem),
}

/// 바꿀 수 없는 까닭.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchProblem {
    /// 쓸 수 없는 이름(`validate_name`의 까닭 그대로).
    Invalid(NameError),
    /// 같은 폴더의 다른 바뀌는 항목과 바뀔 이름이 같다(대소문자 무시). 겹치는 항목 모두에 붙는다.
    Duplicate,
    /// 바뀔 이름이 같은 폴더에 남아 있을 다른 항목의 이름과 같다(대소문자 무시).
    NameTaken,
    /// 원래 이름이 유니코드로 읽을 수 없는 이름이라 규칙을 쓸 수 없다.
    NotUnicode,
}

/// 소문자 한 글자로 바뀌면 그 글자, 아니면 그대로(`name_match`와 같은 단순 접기).
fn fold(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_lowercase();
    }
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(one), None) => one,
        _ => c,
    }
}

fn name_key(name: &str) -> String {
    name.chars().map(fold).collect()
}

fn folder_key(folder: &Path) -> String {
    let text = folder.to_string_lossy();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text.into_owned()
    }
}

/// 줄기와 확장자(점 포함)로 가른다. `name_input::extension_changed`와 같은 규칙.
fn split_stem(name: &str, is_folder: bool) -> (&str, &str) {
    if is_folder {
        return (name, "");
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 && dot + 1 < name.len() => name.split_at(dot),
        _ => (name, ""),
    }
}

/// 줄기에서 `find`(이미 접은 글자들)를 왼쪽부터 겹치지 않게 모두 `replace`로 바꾼다.
fn replace_folded(stem: &str, find: &[char], replace: &str) -> String {
    let chars: Vec<char> = stem.chars().collect();
    let folded: Vec<char> = chars.iter().copied().map(fold).collect();
    let mut out = String::with_capacity(stem.len());
    let mut i = 0;
    while i < chars.len() {
        if i + find.len() <= chars.len() && folded[i..i + find.len()] == *find {
            out.push_str(replace);
            i += find.len();
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn new_name_for(
    item_name: &str,
    is_folder: bool,
    index: usize,
    rule: &RenameRule,
    find: &[char],
) -> String {
    let (stem, ext) = split_stem(item_name, is_folder);
    let replaced;
    let stem = if find.is_empty() {
        stem
    } else {
        replaced = replace_folded(stem, find, &rule.replace);
        &replaced
    };
    let mut out = String::with_capacity(item_name.len() + rule.prefix.len() + rule.suffix.len());
    out.push_str(&rule.prefix);
    match &rule.numbering {
        None => out.push_str(stem),
        Some(n) => {
            let number = n.start.saturating_add(index as u64);
            let number = format!("{number:0>width$}", width = n.digits);
            match n.position {
                NumberPosition::Start => {
                    out.push_str(&number);
                    out.push_str(&n.separator);
                    out.push_str(stem);
                }
                NumberPosition::End => {
                    out.push_str(stem);
                    out.push_str(&n.separator);
                    out.push_str(&number);
                }
            }
        }
    }
    out.push_str(&rule.suffix);
    out.push_str(ext);
    out
}

/// 폴더마다: 이름을 비우는 항목, 그대로 남는 항목, 바뀔 이름의 개수.
#[derive(Default)]
struct FolderNames {
    removed: HashSet<String>,
    kept: HashSet<String>,
    targets: HashMap<String, usize>,
}

/// 규칙을 항목들에 써 본다(아무것도 바꾸지 않는다).
pub fn preview(items: &[BatchItem], rule: &RenameRule, existing: &ExistingNames) -> Preview {
    let find: Vec<char> = rule.find.chars().map(fold).collect();

    let mut entries: Vec<PreviewEntry> = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let (new_name, status) = match item.name.to_str() {
            None => (
                item.name.to_string_lossy().into_owned(),
                EntryStatus::Problem(BatchProblem::NotUnicode),
            ),
            Some(original) => {
                let new_name = new_name_for(original, item.is_folder, index, rule, &find);
                let status = if new_name == original {
                    EntryStatus::Unchanged
                } else if let Err(reason) = validate_name(&new_name) {
                    EntryStatus::Problem(BatchProblem::Invalid(reason))
                } else {
                    EntryStatus::Rename
                };
                (new_name, status)
            }
        };
        entries.push(PreviewEntry {
            folder: item.folder.clone(),
            original: item.name.clone(),
            new_name,
            status,
        });
    }

    let mut folders: HashMap<String, FolderNames> = HashMap::new();
    let mut keys: Vec<(String, String)> = Vec::with_capacity(entries.len());
    for entry in &entries {
        let fkey = folder_key(&entry.folder);
        let original = name_key(&entry.original.to_string_lossy());
        let folder = folders.entry(fkey.clone()).or_default();
        match entry.status {
            EntryStatus::Unchanged | EntryStatus::Problem(BatchProblem::NotUnicode) => {
                folder.kept.insert(original);
            }
            EntryStatus::Rename | EntryStatus::Problem(_) => {
                folder.removed.insert(original);
            }
        }
        let target = name_key(&entry.new_name);
        if entry.status == EntryStatus::Rename {
            *folder.targets.entry(target.clone()).or_insert(0) += 1;
        }
        keys.push((fkey, target));
    }

    let (mut renames, mut unchanged, mut problems) = (0, 0, 0);
    for (entry, (fkey, target)) in entries.iter_mut().zip(&keys) {
        if entry.status == EntryStatus::Rename {
            let folder = &folders[fkey];
            if folder.targets[target] > 1 {
                entry.status = EntryStatus::Problem(BatchProblem::Duplicate);
            } else {
                let existing_here = existing.folders.get(fkey).is_some_and(|names| {
                    names.contains(target) && !folder.removed.contains(target)
                });
                if existing_here || folder.kept.contains(target) {
                    entry.status = EntryStatus::Problem(BatchProblem::NameTaken);
                }
            }
        }
        match entry.status {
            EntryStatus::Rename => renames += 1,
            EntryStatus::Unchanged => unchanged += 1,
            EntryStatus::Problem(_) => problems += 1,
        }
    }

    Preview {
        entries,
        renames,
        unchanged,
        problems,
    }
}
