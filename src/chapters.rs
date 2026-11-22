use crate::probe::ClipInfo;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chapter {
    pub title: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

pub fn build_chapters(clips: &[ClipInfo]) -> Vec<Chapter> {
    let mut chapters = Vec::with_capacity(clips.len());
    let mut cursor_secs = 0.0_f64;

    for clip in clips {
        let start_ms = (cursor_secs * 1000.0).round() as u64;
        cursor_secs += clip.duration_secs;
        let raw_end_ms = (cursor_secs * 1000.0).round() as u64;
        let end_ms = raw_end_ms.max(start_ms + 1);

        chapters.push(Chapter {
            title: clip.title.clone(),
            start_ms,
            end_ms,
        });
    }

    chapters
}

pub fn render_ffmetadata(chapters: &[Chapter]) -> String {
    let mut out = String::from(";FFMETADATA1\n");
    for ch in chapters {
        out.push_str("[CHAPTER]\n");
        out.push_str("TIMEBASE=1/1000\n");
        out.push_str(&format!("START={}\n", ch.start_ms));
        out.push_str(&format!("END={}\n", ch.end_ms));
        out.push_str(&format!("title={}\n", escape_ffmetadata(&ch.title)));
    }
    out
}

/// Escape characters that have special meaning in FFMETADATA1 values:
/// `=`, `;`, `#`, `\`, and newlines.
pub fn escape_ffmetadata(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' | '=' | ';' | '#' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            '\n' => escaped.push_str("\\\n"),
            '\r' => {}
            _ => escaped.push(ch),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Fps;
    use crate::probe::{Sar, VideoStreamInfo};
    use std::path::PathBuf;

    fn dummy_clip(title: &str, duration_secs: f64) -> ClipInfo {
        ClipInfo {
            path: PathBuf::from(format!("{title}.mp4")),
            title: title.to_string(),
            video: VideoStreamInfo {
                codec_name: "h264".into(),
                pix_fmt: Some("yuv420p".into()),
                coded_width: 1920,
                coded_height: 1080,
                sar: Sar::square(),
                rotation: 0,
                fps: Fps { num: 30, den: 1 },
            },
            audio: None,
            duration_secs,
        }
    }

    #[test]
    fn chapters_accumulate_and_escape_special_chars() {
        let clips = vec![dummy_clip("01_intro", 2.5), dummy_clip("02_a=b;c#d\\e", 3.25)];
        let chapters = build_chapters(&clips);
        assert_eq!(
            chapters,
            vec![
                Chapter {
                    title: "01_intro".into(),
                    start_ms: 0,
                    end_ms: 2500
                },
                Chapter {
                    title: "02_a=b;c#d\\e".into(),
                    start_ms: 2500,
                    end_ms: 5750
                },
            ]
        );

        let meta = render_ffmetadata(&chapters);
        assert!(meta.starts_with(";FFMETADATA1\n"));
        assert!(meta.contains("START=0\nEND=2500\ntitle=01_intro\n"));
        assert!(meta.contains("START=2500\nEND=5750\ntitle=02_a\\=b\\;c\\#d\\\\e\n"));
    }
}
