//! facet의 코어. 화면에 의존하지 않는 라이브러리다.
//!
//! 앱과 앞으로 들어올 다른 실행 파일(인덱서 서비스, 셸 확장 도우미)은
//! 이 크레이트를 부르기만 한다.

pub mod batch_rename;
pub mod clipboard;
pub mod command;
pub mod condition_match;
pub mod content_search;
pub mod details;
pub mod dragdrop;
pub mod edit;
pub mod file_list;
mod file_system;
pub mod history;
pub mod indexer_client;
pub mod instant_search;
pub mod job;
pub mod launch;
pub mod live_search;
mod location;
pub mod measure;
pub mod menu;
pub mod name_input;
pub mod name_match;
pub mod placement;
mod provider;
pub mod query_syntax;
pub mod search_results;
pub mod selection;
pub mod session;
pub mod tab;
pub mod watch;
pub mod window;
pub mod work_area;

pub use facet_win::OpenError;
pub use file_system::FileSystemProvider;
pub use location::{Attributes, Item, ListError, Location, SearchKind, SearchScope};
pub use provider::{Candidate, PathSegment, Provider, ResolveError, Resolved, provider_for};

/// 코어 크레이트의 버전.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_matches_package() {
        assert_eq!(version(), "0.1.0");
    }
}
