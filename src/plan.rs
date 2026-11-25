use crate::chapters::{build_chapters, render_ffmetadata, Chapter};
use crate::cli::{Encoder, Fps, Resolution};
use crate::probe::ClipInfo;
use anyhow::{bail, Result};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeMode {
    FastCopy,
    Transcode,
}

#[derive(Debug, Clone)]
pub struct MergePlan {
    pub mode: MergeMode,
    pub target_resolution: Resolution,
    pub target_fps: Fps,
    pub encoder: Encoder,
    pub clips: Vec<ClipInfo>,
    pub chapters: Vec<Chapter>,
    pub ffmetadata: Option<String>,
    pub filter_complex: Option<String>,
}

impl MergePlan {
    pub fn build(
        clips: Vec<ClipInfo>,
        resolution_override: Option<Resolution>,
        fps_override: Option<Fps>,
        encoder: Encoder,
        include_chapters: bool,
    ) -> Result<Self> {
        if clips.is_empty() {
            bail!("cannot build merge plan with zero clips");
        }

        // Output size/fps come from the first landscape clip, else the first clip.
        let reference = clips
            .iter()
            .find(|c| c.video.is_landscape())
            .unwrap_or(&clips[0]);

        let target_resolution = match resolution_override {
            Some(r) => r,
            None => {
                let (w, h) = reference.video.display_size();
                Resolution::make_even(w, h)
            }
        };

        let target_fps = fps_override.unwrap_or(reference.video.fps);

        let mode = if can_use_fast_path(&clips, target_resolution, target_fps) {
            MergeMode::FastCopy
        } else {
            MergeMode::Transcode
        };

        let (chapters, ffmetadata) = if include_chapters {
            let ch = build_chapters(&clips);
            let meta = render_ffmetadata(&ch);
            (ch, Some(meta))
        } else {
            (Vec::new(), None)
        };

        let filter_complex = match mode {
            MergeMode::FastCopy => None,
            MergeMode::Transcode => {
                Some(build_filter_complex(&clips, target_resolution, target_fps))
            }
        };

        Ok(Self {
            mode,
            target_resolution,
            target_fps,
            encoder,
            clips,
            chapters,
            ffmetadata,
            filter_complex,
        })
    }

    pub fn build_ffmpeg_args(
        &self,
        concat_list_path: Option<&Path>,
        metadata_path: Option<&Path>,
        output_path: &Path,
        overwrite: bool,
    ) -> Result<Vec<String>> {
        let mut args = Vec::new();
        args.push(if overwrite { "-y" } else { "-n" }.to_string());

        match self.mode {
            MergeMode::FastCopy => {
                let list_path = concat_list_path.ok_or_else(|| {
                    anyhow::anyhow!("internal error: concat list path missing for fast copy mode")
                })?;
                args.extend([
                    "-f".into(),
                    "concat".into(),
                    "-safe".into(),
                    "0".into(),
                    "-i".into(),
                    list_path.to_string_lossy().into_owned(),
                ]);
                if let Some(meta_path) = metadata_path {
                    args.extend(["-i".into(), meta_path.to_string_lossy().into_owned()]);
                }
                args.extend([
                    "-map".into(),
                    "0:v:0".into(),
                    "-map".into(),
                    "0:a:0?".into(),
                ]);
                if metadata_path.is_some() {
                    args.extend([
                        "-map_metadata".into(),
                        "1".into(),
                        "-map_chapters".into(),
                        "1".into(),
                    ]);
                }
                args.extend([
                    "-c".into(),
                    "copy".into(),
                    "-movflags".into(),
                    "+faststart".into(),
                    output_path.to_string_lossy().into_owned(),
                ]);
            }
            MergeMode::Transcode => {
                for clip in &self.clips {
                    args.push("-i".into());
                    args.push(clip.path.to_string_lossy().into_owned());
                }
                let meta_idx = self.clips.len();
                if let Some(meta_path) = metadata_path {
                    args.push("-i".into());
                    args.push(meta_path.to_string_lossy().into_owned());
                }

                let fc = self.filter_complex.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("internal error: filter_complex missing for transcode mode")
                })?;

                args.extend([
                    "-filter_complex".into(),
                    fc.clone(),
                    "-map".into(),
                    "[vout]".into(),
                    "-map".into(),
                    "[aout]".into(),
                ]);
                if metadata_path.is_some() {
                    args.extend([
                        "-map_metadata".into(),
                        meta_idx.to_string(),
                        "-map_chapters".into(),
                        meta_idx.to_string(),
                    ]);
                }

                args.extend(self.encoder.ffmpeg_args());

                args.extend([
                    "-c:a".into(),
                    "aac".into(),
                    "-b:a".into(),
                    "192k".into(),
                    "-ar".into(),
                    "48000".into(),
                    "-ac".into(),
                    "2".into(),
                    "-movflags".into(),
                    "+faststart".into(),
                    output_path.to_string_lossy().into_owned(),
                ]);
            }
        }

        Ok(args)
    }
}

pub fn can_use_fast_path(clips: &[ClipInfo], target_res: Resolution, target_fps: Fps) -> bool {
    let Some(first) = clips.first() else {
        return false;
    };

    for clip in clips {
        if clip.video.rotation != 0 {
            return false;
        }
        if !clip.video.sar.is_square() {
            return false;
        }
        if clip.video.coded_width != target_res.width
            || clip.video.coded_height != target_res.height
        {
            return false;
        }
        if !clip.video.fps.approx_eq(target_fps) {
            return false;
        }
        if clip.video.codec_name != first.video.codec_name {
            return false;
        }
        if clip.video.pix_fmt != first.video.pix_fmt {
            return false;
        }

        match (&clip.audio, &first.audio) {
            (None, None) => {}
            (Some(a), Some(fa)) => {
                if a.codec_name != fa.codec_name
                    || a.sample_rate != fa.sample_rate
                    || a.channels != fa.channels
                {
                    return false;
                }
            }
            _ => return false,
        }
    }

    true
}

pub fn build_filter_complex(
    clips: &[ClipInfo],
    target_res: Resolution,
    target_fps: Fps,
) -> String {
    let w = target_res.width;
    let h = target_res.height;
    let fps_expr = target_fps.to_ffmpeg_expr();

    let mut parts = Vec::with_capacity(clips.len() * 2 + 1);
    let mut concat_inputs = String::new();

    for (i, clip) in clips.iter().enumerate() {
        let sar_prefix = if clip.video.sar.is_square() {
            ""
        } else {
            "scale=trunc(iw*sar/2)*2:ih,setsar=1,"
        };

        parts.push(format!(
            "[{i}:v:0]{sar_prefix}scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:black,setsar=1,fps={fps_expr},format=yuv420p,setpts=PTS-STARTPTS[v{i}]"
        ));

        if clip.audio.is_some() {
            parts.push(format!(
                "[{i}:a:0]aresample=48000,aformat=sample_rates=48000:channel_layouts=stereo,asetpts=PTS-STARTPTS[a{i}]"
            ));
        } else {
            parts.push(format!(
                "anullsrc=channel_layout=stereo:sample_rate=48000,atrim=duration={:.6},asetpts=PTS-STARTPTS[a{i}]",
                clip.duration_secs
            ));
        }

        concat_inputs.push_str(&format!("[v{i}][a{i}]"));
    }

    parts.push(format!(
        "{concat_inputs}concat=n={}:v=1:a=1[vout][aout]",
        clips.len()
    ));

    parts.join(";")
}

pub fn render_concat_list(clips: &[ClipInfo]) -> Result<String> {
    let cwd = std::env::current_dir()?;
    let mut out = String::new();

    for clip in clips {
        let abs = if clip.path.is_absolute() {
            clip.path.clone()
        } else {
            cwd.join(&clip.path)
        };
        let normalized = normalize_concat_path(&abs);
        let escaped = normalized.replace('\'', "'\\''");
        out.push_str(&format!("file '{escaped}'\n"));
    }

    Ok(out)
}

fn normalize_concat_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    let stripped = s.strip_prefix(r"\\?\").unwrap_or(&s);
    stripped.replace('\\', "/")
}

pub fn format_command_line(program: &Path, args: &[String]) -> String {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(quote_arg(&program.to_string_lossy()));
    for arg in args {
        parts.push(quote_arg(arg));
    }
    parts.join(" ")
}

fn quote_arg(arg: &str) -> String {
    if arg.is_empty()
        || arg
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '"' | ';' | '&' | '|' | '<' | '>' | '[' | ']' | '(' | ')'))
    {
        format!("\"{}\"", arg.replace('"', "\\\""))
    } else {
        arg.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::{AudioStreamInfo, Sar, VideoStreamInfo};
    use std::path::PathBuf;

    fn make_clip(
        name: &str,
        w: u32,
        h: u32,
        rotation: i32,
        fps_num: u64,
        has_audio: bool,
    ) -> ClipInfo {
        ClipInfo {
            path: PathBuf::from(format!("{name}.mp4")),
            title: name.to_string(),
            video: VideoStreamInfo {
                codec_name: "h264".into(),
                pix_fmt: Some("yuv420p".into()),
                coded_width: w,
                coded_height: h,
                sar: Sar::square(),
                rotation,
                fps: Fps {
                    num: fps_num,
                    den: 1,
                },
            },
            audio: if has_audio {
                Some(AudioStreamInfo {
                    codec_name: "aac".into(),
                    sample_rate: 48000,
                    channels: 2,
                })
            } else {
                None
            },
            duration_secs: 2.0,
        }
    }

    #[test]
    fn fast_path_selected_when_all_clips_match() {
        let clips = vec![
            make_clip("a", 1920, 1080, 0, 30, true),
            make_clip("b", 1920, 1080, 0, 30, true),
        ];
        let plan = MergePlan::build(clips, None, None, Encoder::X264, true).unwrap();
        assert_eq!(plan.mode, MergeMode::FastCopy);
        assert_eq!(
            plan.target_resolution,
            Resolution {
                width: 1920,
                height: 1080
            }
        );
        assert_eq!(plan.target_fps, Fps { num: 30, den: 1 });
        assert!(plan.filter_complex.is_none());
        assert!(plan.ffmetadata.is_some());
    }

    #[test]
    fn no_chapters_omits_metadata_and_map_chapters() {
        let clips = vec![
            make_clip("a", 1920, 1080, 0, 30, true),
            make_clip("b", 1920, 1080, 0, 30, true),
        ];
        let plan = MergePlan::build(clips, None, None, Encoder::X264, false).unwrap();
        assert!(plan.chapters.is_empty());
        assert!(plan.ffmetadata.is_none());

        let args = plan
            .build_ffmpeg_args(
                Some(Path::new("list.txt")),
                None,
                Path::new("out.mp4"),
                true,
            )
            .unwrap();
        assert!(!args.contains(&"-map_chapters".to_string()));
        assert!(!args.contains(&"-map_metadata".to_string()));
    }

    #[test]
    fn target_comes_from_first_landscape_clip() {
        let clips = vec![
            make_clip("vertical_first", 1080, 1920, 0, 60, true),
            make_clip("landscape_second", 1280, 720, 0, 30, true),
        ];
        let plan = MergePlan::build(clips, None, None, Encoder::X264, true).unwrap();
        assert_eq!(plan.mode, MergeMode::Transcode);
        assert_eq!(
            plan.target_resolution,
            Resolution {
                width: 1280,
                height: 720
            }
        );
        assert_eq!(plan.target_fps, Fps { num: 30, den: 1 });

        let fc = plan.filter_complex.unwrap();
        assert!(fc.contains("scale=1280:720:force_original_aspect_ratio=decrease"));
        assert!(fc.contains("pad=1280:720:(ow-iw)/2:(oh-ih)/2:black"));
        assert!(fc.contains("setsar=1,fps=30,format=yuv420p"));
        assert!(fc.contains("concat=n=2:v=1:a=1[vout][aout]"));
    }

    #[test]
    fn silent_clip_triggers_transcode_with_anullsrc() {
        let clips = vec![
            make_clip("with_audio", 1280, 720, 0, 30, true),
            make_clip("silent", 1280, 720, 0, 30, false),
        ];
        let plan = MergePlan::build(clips, None, None, Encoder::Nvenc, true).unwrap();
        assert_eq!(plan.mode, MergeMode::Transcode);
        let fc = plan.filter_complex.as_ref().unwrap();
        assert!(fc.contains("anullsrc=channel_layout=stereo:sample_rate=48000,atrim=duration=2.000000"));

        let args = plan
            .build_ffmpeg_args(
                None,
                Some(Path::new("meta.txt")),
                Path::new("out.mp4"),
                true,
            )
            .unwrap();
        assert!(args.contains(&"h264_nvenc".to_string()));
        assert!(args.contains(&"-map_chapters".to_string()));
        assert!(args.contains(&"+faststart".to_string()));
    }
}
