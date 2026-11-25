use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

fn mi_merge_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_mi-merge"))
}

fn find_tool(name: &str) -> PathBuf {
    let tools = mi_merge::ffmpeg_bin::MediaTools::locate(None, None)
        .expect("ffmpeg and ffprobe must be installed to run integration tests");
    if name == "ffmpeg" {
        tools.ffmpeg
    } else {
        tools.ffprobe
    }
}

fn run_ffmpeg(args: &[&str]) {
    let ffmpeg = find_tool("ffmpeg");
    let output = Command::new(&ffmpeg)
        .args(args)
        .output()
        .expect("failed to execute ffmpeg");
    assert!(
        output.status.success(),
        "ffmpeg failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[derive(Debug, Deserialize)]
struct ProbeResult {
    #[serde(default)]
    streams: Vec<ProbeStream>,
    #[serde(default)]
    chapters: Vec<ProbeChapter>,
    format: ProbeFormat,
}

#[derive(Debug, Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u32>,
    pix_fmt: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProbeChapter {
    #[serde(default)]
    tags: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct ProbeFormat {
    duration: Option<String>,
}

fn probe_output(path: &Path) -> ProbeResult {
    let ffprobe = find_tool("ffprobe");
    let output = Command::new(&ffprobe)
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
            "-show_chapters",
        ])
        .arg(path)
        .output()
        .expect("failed to execute ffprobe");
    assert!(
        output.status.success(),
        "ffprobe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("failed to parse ffprobe json")
}

#[test]
fn fast_path_folder_natural_sort_and_chapters() {
    let dir = tempdir().unwrap();
    let clips_dir = dir.path().join("clips");
    std::fs::create_dir(&clips_dir).unwrap();

    let clip2 = clips_dir.join("clip2.mp4");
    let clip10 = clips_dir.join("clip10.mp4");

    for (path, freq) in [(&clip2, "440"), (&clip10, "660")] {
        let p = path.to_string_lossy().into_owned();
        let audio_src = format!("sine=frequency={freq}:sample_rate=48000:duration=1");
        run_ffmpeg(&[
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=640x360:rate=30:duration=1",
            "-f",
            "lavfi",
            "-i",
            &audio_src,
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-ar",
            "48000",
            "-ac",
            "2",
            &p,
        ]);
    }

    let out = dir.path().join("merged_fast.mp4");

    // Verify dry-run reports fast-copy and does not create output
    let dry = Command::new(mi_merge_bin())
        .arg(&clips_dir)
        .arg("-o")
        .arg(&out)
        .arg("--dry-run")
        .output()
        .unwrap();
    assert!(dry.status.success());
    let dry_stdout = String::from_utf8_lossy(&dry.stdout);
    assert!(
        dry_stdout.contains("fast-copy"),
        "expected fast-copy in dry-run output, got:\n{dry_stdout}"
    );
    assert!(!out.exists());

    // Default run (no --chapters): should produce 0 chapters
    let run_default = Command::new(mi_merge_bin())
        .arg(&clips_dir)
        .arg("-o")
        .arg(&out)
        .arg("-y")
        .env_remove("MI_MERGE_CHAPTERS")
        .output()
        .unwrap();
    assert!(
        run_default.status.success(),
        "mi-merge default failed:\n{}",
        String::from_utf8_lossy(&run_default.stderr)
    );
    assert!(out.exists());
    let probed_default = probe_output(&out);
    assert!(
        probed_default.chapters.is_empty(),
        "expected 0 chapters by default, got {:?}",
        probed_default.chapters
    );

    // Execute merge with MI_MERGE_CHAPTERS=1 env var
    let run = Command::new(mi_merge_bin())
        .arg(&clips_dir)
        .arg("-o")
        .arg(&out)
        .arg("-y")
        .env("MI_MERGE_CHAPTERS", "1")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "mi-merge failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(out.exists());

    let probed = probe_output(&out);
    let v = probed
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("video"))
        .unwrap();
    assert_eq!(v.width, Some(640));
    assert_eq!(v.height, Some(360));
    assert_eq!(v.pix_fmt.as_deref(), Some("yuv420p"));

    let a = probed
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("audio"))
        .unwrap();
    assert_eq!(a.sample_rate.as_deref(), Some("48000"));
    assert_eq!(a.channels, Some(2));

    let duration: f64 = probed.format.duration.unwrap().parse().unwrap();
    assert!(
        (duration - 2.0).abs() < 0.25,
        "expected ~2.0s duration, got {duration}s"
    );

    let titles: Vec<_> = probed
        .chapters
        .iter()
        .map(|c| c.tags.get("title").cloned().unwrap_or_default())
        .collect();
    assert_eq!(titles, vec!["clip2", "clip10"]);

    // Verify overwrite protection without -y
    let no_overwrite = Command::new(mi_merge_bin())
        .arg(&clips_dir)
        .arg("-o")
        .arg(&out)
        .output()
        .unwrap();
    assert!(!no_overwrite.status.success());
    let err_text = String::from_utf8_lossy(&no_overwrite.stderr);
    assert!(err_text.contains("-y"), "expected -y hint, got: {err_text}");
}

#[test]
fn transcode_path_vertical_pillarbox_and_silent_clip() {
    let dir = tempdir().unwrap();

    let clip1 = dir.path().join("01_vertical_silent.mp4");
    let clip2 = dir.path().join("02_landscape_main.mp4");
    let clip3 = dir.path().join("03_rotated_phone.mp4");

    // Clip 1: vertical 360x640, 24fps, no audio
    run_ffmpeg(&[
        "-y",
        "-f",
        "lavfi",
        "-i",
        "color=c=red:size=360x640:rate=24:duration=1",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
        &clip1.to_string_lossy(),
    ]);

    // Clip 2: landscape 640x360, 30fps, mono 44100Hz audio
    run_ffmpeg(&[
        "-y",
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=640x360:rate=30:duration=1",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=520:sample_rate=44100:duration=1",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-ar",
        "44100",
        "-ac",
        "1",
        &clip2.to_string_lossy(),
    ]);

    // Clip 3: coded 640x360 with 90-deg display rotation (vertical display), 30fps, stereo 48kHz
    run_ffmpeg(&[
        "-y",
        "-f",
        "lavfi",
        "-i",
        "color=c=blue:size=640x360:rate=30:duration=1",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=700:sample_rate=48000:duration=1",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-ar",
        "48000",
        "-ac",
        "2",
        "-metadata:s:v:0",
        "rotate=90",
        &clip3.to_string_lossy(),
    ]);

    let out = dir.path().join("merged_transcode.mp4");

    let run = Command::new(mi_merge_bin())
        .args([
            clip1.to_str().unwrap(),
            clip2.to_str().unwrap(),
            clip3.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--chapters",
            "-y",
        ])
        .env_remove("MI_MERGE_CHAPTERS")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "mi-merge transcode failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );

    let probed = probe_output(&out);

    let v = probed
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("video"))
        .unwrap();
    // Output size/fps must come from first landscape clip (02_landscape_main: 640x360 @ 30fps)
    assert_eq!(v.width, Some(640));
    assert_eq!(v.height, Some(360));
    assert_eq!(v.pix_fmt.as_deref(), Some("yuv420p"));
    assert_eq!(v.avg_frame_rate.as_deref(), Some("30/1"));

    let a = probed
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("audio"))
        .unwrap();
    assert_eq!(a.sample_rate.as_deref(), Some("48000"));
    assert_eq!(a.channels, Some(2));

    let duration: f64 = probed.format.duration.unwrap().parse().unwrap();
    assert!(
        (duration - 3.0).abs() < 0.3,
        "expected ~3.0s duration, got {duration}s"
    );

    let titles: Vec<_> = probed
        .chapters
        .iter()
        .map(|c| c.tags.get("title").cloned().unwrap_or_default())
        .collect();
    assert_eq!(
        titles,
        vec![
            "01_vertical_silent",
            "02_landscape_main",
            "03_rotated_phone"
        ]
    );
}
