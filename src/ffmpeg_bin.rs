use anyhow::{bail, Result};
use std::env;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct MediaTools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

impl MediaTools {
    /// Locate ffmpeg and ffprobe in order:
    /// 1. Explicit CLI flag (--ffmpeg / --ffprobe; also checks sibling of --ffmpeg for ffprobe)
    /// 2. Next to the mi-merge executable
    /// 3. On PATH
    pub fn locate(ffmpeg_flag: Option<&Path>, ffprobe_flag: Option<&Path>) -> Result<Self> {
        let exe_dir = env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()));

        let ffmpeg = locate_tool("ffmpeg", ffmpeg_flag, None, exe_dir.as_deref())?;

        // If --ffmpeg was explicitly provided, check its parent directory for ffprobe before exe_dir/PATH.
        let ffmpeg_sibling_dir = ffmpeg_flag.and_then(|p| {
            if p.is_dir() {
                Some(p)
            } else {
                p.parent()
            }
        });

        let ffprobe = locate_tool(
            "ffprobe",
            ffprobe_flag,
            ffmpeg_sibling_dir,
            exe_dir.as_deref(),
        )?;

        Ok(Self { ffmpeg, ffprobe })
    }
}

fn exe_names(base: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![format!("{base}.exe"), base.to_string()]
    } else {
        vec![base.to_string(), format!("{base}.exe")]
    }
}

fn find_in_dir(dir: &Path, base: &str) -> Option<PathBuf> {
    for name in exe_names(base) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn find_in_path(base: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    for dir in env::split_paths(&path_var) {
        if let Some(found) = find_in_dir(&dir, base) {
            return Some(found);
        }
    }
    None
}

fn locate_tool(
    base: &str,
    explicit: Option<&Path>,
    secondary_dir: Option<&Path>,
    exe_dir: Option<&Path>,
) -> Result<PathBuf> {
    if let Some(flag_path) = explicit {
        if flag_path.is_dir() {
            if let Some(found) = find_in_dir(flag_path, base) {
                return Ok(found);
            }
            bail!(
                "{base} was not found inside directory: {}",
                flag_path.display()
            );
        } else if flag_path.is_file() {
            return Ok(flag_path.to_path_buf());
        } else {
            bail!(
                "specified --{base} path does not exist: {}",
                flag_path.display()
            );
        }
    }

    if let Some(dir) = secondary_dir {
        if let Some(found) = find_in_dir(dir, base) {
            return Ok(found);
        }
    }

    if let Some(dir) = exe_dir {
        if let Some(found) = find_in_dir(dir, base) {
            return Ok(found);
        }
    }

    if let Some(found) = find_in_path(base) {
        return Ok(found);
    }

    bail!(
        "could not find '{base}'. Pass --{base} <PATH>, place {base}.exe next to mi-merge.exe, or add FFmpeg to PATH (e.g. `winget install Gyan.FFmpeg`)."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn locate_tool_prefers_explicit_flag_file_and_dir() {
        let dir = tempdir().unwrap();
        let fake_ffmpeg = dir.path().join(if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        });
        let fake_ffprobe = dir.path().join(if cfg!(windows) {
            "ffprobe.exe"
        } else {
            "ffprobe"
        });
        fs::write(&fake_ffmpeg, b"").unwrap();
        fs::write(&fake_ffprobe, b"").unwrap();

        // Pass --ffmpeg as file -> ffprobe found as sibling
        let tools = MediaTools::locate(Some(&fake_ffmpeg), None).unwrap();
        assert_eq!(tools.ffmpeg, fake_ffmpeg);
        assert_eq!(tools.ffprobe, fake_ffprobe);

        // Pass --ffmpeg as directory -> both found in directory
        let tools_dir = MediaTools::locate(Some(dir.path()), None).unwrap();
        assert_eq!(tools_dir.ffmpeg, fake_ffmpeg);
        assert_eq!(tools_dir.ffprobe, fake_ffprobe);
    }
}
