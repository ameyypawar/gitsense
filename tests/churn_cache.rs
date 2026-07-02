use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gitsense::git::history::file_churn;
use gitsense::index::SymbolIndex;

/// #7 gating test: `AppState::churn_cache` is only safe to populate once
/// per process because `file_churn` is assumed to be a pure function of
/// (HEAD, file set) — HEAD never moves for the process lifetime, so the
/// same inputs must always produce the same per-file counts.
///
/// Compared as a `HashMap` rather than asserting `Vec` equality directly:
/// `file_churn` sorts by count descending, and ties are broken by the
/// iteration order of an internal `HashMap` whose hasher is freshly seeded
/// per call — two independently-computed `Vec`s can legitimately differ in
/// tie order while representing identical counts. What caching must
/// preserve is the counts themselves, not incidental tie order.
#[test]
fn file_churn_is_stable_across_repeated_calls() -> anyhow::Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let idx = SymbolIndex::build(repo)?;
    let files: Vec<PathBuf> = idx.file_paths().into_iter().cloned().collect();

    let first = file_churn(repo, &files)?;
    let second = file_churn(repo, &files)?;

    assert!(
        !first.is_empty(),
        "expected at least one churned file in gitsense's own history"
    );

    let first_counts: HashMap<&PathBuf, usize> = first.iter().map(|(p, n)| (p, *n)).collect();
    let second_counts: HashMap<&PathBuf, usize> = second.iter().map(|(p, n)| (p, *n)).collect();

    assert_eq!(
        first_counts, second_counts,
        "file_churn must report identical per-file counts across repeated \
         calls for a fixed HEAD — this is the invariant AppState::churn_cache \
         relies on to cache the result for the process lifetime"
    );

    Ok(())
}
