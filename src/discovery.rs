use anyhow::{bail, Context, Result};
use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};

const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "mkv", "avi", "webm", "m4v", "mts", "m2ts", "ts", "wmv", "flv", "3gp", "mpg",
    "mpeg", "ogv",
];

pub fn is_video_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| VIDEO_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Resolve a list of CLI input paths (files or folders) into an ordered list of video files.
/// Explicit files preserve their CLI order. Folders are expanded (top-level video files)
/// and sorted using natural sort order.
pub fn resolve_inputs(inputs: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    for input in inputs {
        if !input.exists() {
            bail!("input path does not exist: {}", input.display());
        }
        if input.is_dir() {
            let mut dir_files = Vec::new();
            let entries = fs::read_dir(input)
                .with_context(|| format!("failed to read directory: {}", input.display()))?;
            for entry in entries {
                let entry = entry.with_context(|| {
                    format!("failed to read entry in directory: {}", input.display())
                })?;
                let path = entry.path();
                if path.is_file() && is_video_file(&path) {
                    dir_files.push(path);
                }
            }
            dir_files.sort_by(|a, b| {
                let a_name = a.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                let b_name = b.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                natural_cmp(a_name, b_name)
            });
            files.extend(dir_files);
        } else if input.is_file() {
            files.push(input.clone());
        } else {
            bail!("unsupported input path: {}", input.display());
        }
    }

    if files.is_empty() {
        bail!("no video clips found in the specified inputs");
    }

    Ok(files)
}

/// Natural sort comparator (case-insensitive for letters, numeric for digit runs).
/// E.g., "clip2.mp4" < "clip10.mp4", "IMG_001.mp4" < "IMG_2.mp4".
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut a_bytes = a.as_bytes();
    let mut b_bytes = b.as_bytes();

    while !a_bytes.is_empty() && !b_bytes.is_empty() {
        let a_is_digit = a_bytes[0].is_ascii_digit();
        let b_is_digit = b_bytes[0].is_ascii_digit();

        if a_is_digit && b_is_digit {
            let a_len = a_bytes.iter().take_while(|c| c.is_ascii_digit()).count();
            let b_len = b_bytes.iter().take_while(|c| c.is_ascii_digit()).count();

            let a_num = &a_bytes[..a_len];
            let b_num = &b_bytes[..b_len];

            let a_trimmed = trim_leading_zeros(a_num);
            let b_trimmed = trim_leading_zeros(b_num);

            let ord = a_trimmed
                .len()
                .cmp(&b_trimmed.len())
                .then_with(|| a_trimmed.cmp(b_trimmed))
                .then_with(|| a_len.cmp(&b_len));
            if ord != Ordering::Equal {
                return ord;
            }

            a_bytes = &a_bytes[a_len..];
            b_bytes = &b_bytes[b_len..];
        } else {
            let ca = a_bytes[0].to_ascii_lowercase();
            let cb = b_bytes[0].to_ascii_lowercase();
            let ord = ca.cmp(&cb);
            if ord != Ordering::Equal {
                return ord;
            }
            a_bytes = &a_bytes[1..];
            b_bytes = &b_bytes[1..];
        }
    }

    a_bytes.len().cmp(&b_bytes.len()).then_with(|| a.cmp(b))
}

fn trim_leading_zeros(digits: &[u8]) -> &[u8] {
    let first_nonzero = digits.iter().position(|&b| b != b'0');
    match first_nonzero {
        Some(idx) => &digits[idx..],
        None => &digits[digits.len().saturating_sub(1)..],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn natural_sort_orders_numbers_humanly() {
        let mut names = vec![
            "clip10.mp4",
            "clip2.mp4",
            "clip1.mp4",
            "clip20.mp4",
            "Clip3.mp4",
            "clip01.mp4",
        ];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            names,
            vec![
                "clip1.mp4",
                "clip01.mp4",
                "clip2.mp4",
                "Clip3.mp4",
                "clip10.mp4",
                "clip20.mp4"
            ]
        );
    }

    #[test]
    fn resolve_inputs_expands_folder_and_ignores_non_video() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("part10.mp4"), b"").unwrap();
        fs::write(dir.path().join("part2.mp4"), b"").unwrap();
        fs::write(dir.path().join("notes.txt"), b"").unwrap();
        fs::write(dir.path().join("part1.MOV"), b"").unwrap();

        let resolved = resolve_inputs(&[dir.path().to_path_buf()]).unwrap();
        let names: Vec<_> = resolved
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, vec!["part1.MOV", "part2.mp4", "part10.mp4"]);
    }
}
