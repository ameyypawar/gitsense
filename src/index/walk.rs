use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Recursively collect all `*.rs` files under `repo_root`, skipping
/// `target/`, `.git/`, and any hidden directories (name starts with `.`).
pub fn collect_rust_files(repo_root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    for entry in WalkDir::new(repo_root).into_iter().filter_entry(|e| {
        let name = e.file_name().to_string_lossy();
        // Skip hidden dirs, target/, and .git/
        if e.file_type().is_dir() {
            return name != "target" && name != ".git" && !name.starts_with('.');
        }
        true
    }) {
        let entry = entry?;
        if entry.file_type().is_file() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                files.push(path.to_path_buf());
            }
        }
    }

    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn finds_main_rs_in_src() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let src_dir = Path::new(manifest_dir).join("src");

        let files = collect_rust_files(&src_dir).expect("walk should succeed");

        let names: Vec<&str> = files
            .iter()
            .filter_map(|p| p.file_name()?.to_str())
            .collect();

        assert!(
            names.contains(&"main.rs"),
            "expected main.rs in src/, got: {:?}",
            names
        );
    }
}
