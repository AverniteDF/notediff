//! Ranking of baselines that could be linked to a document, free of any
//! toolkit types.
//!
//! Ranking only orders suggestions. The user explicitly chooses Link or Copy;
//! suggestions are never applied automatically.

use std::path::Path;
use std::time::{Duration, Instant};

use similar::TextDiffConfig;

use super::LARGE_FILE_WARNING_BYTES;
use super::fsutil::read_utf8_limited;
use super::snapshot_store::SnapshotEntry;

/// A snapshot that could be linked, with the signals used to order it.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub entry: SnapshotEntry,
    /// The live file name matches the target document's file name.
    pub same_name: bool,
    /// Line-based similarity between the snapshot and the target's current
    /// text, from 0.0 to 1.0. `None` when it was not measured (empty or very
    /// large text, or an unreadable snapshot).
    pub similarity: Option<f64>,
}

impl Candidate {
    /// A candidate that has not been ranked (no badges are shown for it).
    pub fn unranked(entry: SnapshotEntry) -> Self {
        Self {
            entry,
            same_name: false,
            similarity: None,
        }
    }
}

/// Line-based similarity of two texts, from 0.0 (nothing in common) to 1.0.
pub fn similarity(a: &str, b: &str) -> f64 {
    let mut config = TextDiffConfig::default();
    config.deadline(Instant::now() + Duration::from_millis(250));
    f64::from(config.diff_lines(a, b).ratio())
}

/// Read a snapshot for similarity measurement. Snapshots above the large-file
/// cap are skipped so opening the dialog stays fast.
pub fn read_limited(path: &Path) -> Option<String> {
    read_utf8_limited(path, LARGE_FILE_WARNING_BYTES).ok()
}

/// Order `entries` as link candidates for the document at `target` whose
/// current text is `live_text`. Same file name comes first, then higher
/// similarity, then newer modification time, then path. `read` supplies a
/// snapshot's text (see [`read_limited`]).
pub fn rank(
    entries: &[SnapshotEntry],
    target: &Path,
    live_text: &str,
    read: impl Fn(&Path) -> Option<String>,
) -> Vec<Candidate> {
    let target_name = target.file_name();
    let measure = !live_text.is_empty() && live_text.len() as u64 <= LARGE_FILE_WARNING_BYTES;

    let mut ranked: Vec<Candidate> = entries
        .iter()
        .map(|entry| {
            let similarity = if measure {
                read(&entry.snapshot_path).map(|text| similarity(&text, live_text))
            } else {
                None
            };
            Candidate {
                same_name: target_name.is_some() && entry.live_path.file_name() == target_name,
                similarity,
                entry: entry.clone(),
            }
        })
        .collect();

    ranked.sort_by(|a, b| {
        b.same_name
            .cmp(&a.same_name)
            .then_with(|| {
                b.similarity
                    .unwrap_or(-1.0)
                    .total_cmp(&a.similarity.unwrap_or(-1.0))
            })
            .then_with(|| b.entry.modified.cmp(&a.entry.modified))
            .then_with(|| a.entry.live_path.cmp(&b.entry.live_path))
    });
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::snapshot_store::LivePathState;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::time::{Duration, UNIX_EPOCH};

    fn entry(live: &str) -> SnapshotEntry {
        SnapshotEntry {
            snapshot_path: PathBuf::from(format!("/store{live}")),
            live_path: PathBuf::from(live),
            modified: None,
            state: LivePathState::Orphan,
        }
    }

    fn texts(pairs: &[(&str, &str)]) -> HashMap<PathBuf, String> {
        pairs
            .iter()
            .map(|(live, text)| (PathBuf::from(format!("/store{live}")), text.to_string()))
            .collect()
    }

    fn order(ranked: &[Candidate]) -> Vec<String> {
        ranked
            .iter()
            .map(|c| c.entry.live_path.display().to_string())
            .collect()
    }

    #[test]
    fn similarity_bounds() {
        assert_eq!(similarity("a\nb\n", "a\nb\n"), 1.0);
        assert_eq!(similarity("a\n", "b\n"), 0.0);
        let partial = similarity("a\nb\nc\nd\n", "a\nb\nc\nx\n");
        assert!(partial > 0.0 && partial < 1.0);
    }

    #[test]
    fn same_name_first_then_similarity() {
        let entries = [
            entry("/a/other.md"),
            entry("/b/notes.md"),
            entry("/c/close.md"),
        ];
        let map = texts(&[
            ("/a/other.md", "zzz\n"),
            ("/b/notes.md", "unrelated\n"),
            ("/c/close.md", "one\ntwo\nthree\nfour\n"),
        ]);
        let ranked = rank(
            &entries,
            Path::new("/z/notes.md"),
            "one\ntwo\nthree\n",
            |path| map.get(path).cloned(),
        );
        assert_eq!(
            order(&ranked),
            vec!["/b/notes.md", "/c/close.md", "/a/other.md"]
        );
        assert!(ranked[0].same_name);
        assert!(!ranked[1].same_name);
        assert!(ranked[1].similarity.unwrap() > ranked[2].similarity.unwrap());
    }

    #[test]
    fn empty_live_text_skips_similarity() {
        let entries = [entry("/a/x.md")];
        let map = texts(&[("/a/x.md", "text\n")]);
        let ranked = rank(&entries, Path::new("/z/y.md"), "", |p| map.get(p).cloned());
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].similarity, None);
    }

    #[test]
    fn unreadable_snapshots_rank_last_among_equals() {
        let entries = [entry("/a/gone.md"), entry("/b/there.md")];
        let map = texts(&[("/b/there.md", "same\n")]);
        let ranked = rank(&entries, Path::new("/z/y.md"), "same\n", |p| {
            map.get(p).cloned()
        });
        assert_eq!(order(&ranked), vec!["/b/there.md", "/a/gone.md"]);
        assert_eq!(ranked[1].similarity, None);
    }

    #[test]
    fn newer_snapshots_win_ties() {
        let mut old = entry("/a/old.md");
        old.modified = Some(UNIX_EPOCH + Duration::from_secs(10));
        let mut new = entry("/b/new.md");
        new.modified = Some(UNIX_EPOCH + Duration::from_secs(20));
        let ranked = rank(&[old, new], Path::new("/z/y.md"), "", |_| None);
        assert_eq!(order(&ranked), vec!["/b/new.md", "/a/old.md"]);
    }

    #[test]
    fn unranked_has_no_signals() {
        let candidate = Candidate::unranked(entry("/a/x.md"));
        assert!(!candidate.same_name);
        assert_eq!(candidate.similarity, None);
    }
}
