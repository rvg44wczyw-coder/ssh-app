//! Phase 0 of the universal agent feed (docs/features/60-universal-agent-feed.md): the behaviour of the
//! parsers for Claude Code and Antigravity, frozen as snapshots. The refactor into registry, `SessionSource`
//! and `DialectParser` must leave every snapshot byte-identical.
//!
//! A missing or different snapshot fails the test. `UPDATE_GOLDEN=1 cargo test -p ssh-core --test golden_feed`
//! rewrites the snapshots; the diff is then reviewed like any other change.

use ssh_core::feed::FeedState;
use ssh_core::screen::parse_screen_feed;
use ssh_core::transcript::parse_transcript_jsonl;
use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn check(name: &str, actual: &str) {
    let path = root()
        .join("tests/golden/snap")
        .join(format!("{name}.snap"));
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, actual).unwrap();
        return;
    }
    let want = fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!("no snapshot {name}: run with UPDATE_GOLDEN=1 and review the file")
    });
    assert!(
        want == actual,
        "snapshot {name} changed (UPDATE_GOLDEN=1 rewrites it)\n--- want\n{want}\n--- got\n{actual}"
    );
}

fn files(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == ext))
        .collect();
    v.sort();
    v
}

fn stem(p: &Path) -> String {
    p.file_stem().unwrap().to_string_lossy().into_owned()
}

#[test]
fn every_screen_fixture_parses_to_its_snapshot() {
    let mut n = 0;
    for dir in ["src/screen/fixtures", "tests/data"] {
        for p in files(&root().join(dir), "txt") {
            let text = fs::read_to_string(&p).unwrap();
            let feed = parse_screen_feed(text);
            check(&format!("screen_{}", stem(&p)), &format!("{feed:#?}\n"));
            n += 1;
        }
    }
    assert!(n >= 15, "fixtures went missing: {n}");
}

#[test]
fn every_transcript_fixture_parses_to_its_snapshot() {
    let mut n = 0;
    for p in files(&root().join("tests/golden/transcript"), "jsonl") {
        let raw = fs::read_to_string(&p).unwrap();
        let turns = parse_transcript_jsonl(raw).unwrap();
        check(
            &format!("transcript_{}", stem(&p)),
            &format!("{turns:#?}\n"),
        );
        n += 1;
    }
    assert_eq!(n, 4);
}

#[test]
fn the_live_feed_in_any_chunking_equals_the_whole_file() {
    for p in files(&root().join("tests/golden/transcript"), "jsonl") {
        let raw = fs::read_to_string(&p).unwrap();
        let whole = parse_transcript_jsonl(raw.clone()).unwrap();
        for size in [1usize, 7, 64, 1000] {
            let state = FeedState::new();
            let bytes = raw.as_bytes();
            let mut at = 0;
            while at < bytes.len() {
                let mut end = (at + size).min(bytes.len());
                while !raw.is_char_boundary(end) {
                    end += 1;
                }
                state.apply_chunk(raw[at..end].to_string());
                at = end;
            }
            assert_eq!(state.turns(), whole, "{} in chunks of {size}", stem(&p));
        }
    }
}
