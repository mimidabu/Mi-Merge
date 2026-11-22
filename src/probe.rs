use crate::cli::Fps;
use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sar {
    pub num: u32,
    pub den: u32,
}

impl Sar {
    pub fn square() -> Self {
        Self { num: 1, den: 1 }
    }

    pub fn is_square(self) -> bool {
        self.num == self.den
    }

    pub fn parse(s: Option<&str>) -> Self {
        let Some(raw) = s else {
            return Self::square();
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("N/A") {
            return Self::square();
        }
        let sep = if trimmed.contains(':') { ':' } else { '/' };
        if let Some((n_str, d_str)) = trimmed.split_once(sep) {
            if let (Ok(n), Ok(d)) = (n_str.trim().parse::<u32>(), d_str.trim().parse::<u32>()) {
                if n > 0 && d > 0 {
                    let g = gcd_u32(n, d);
                    return Self {
                        num: n / g,
                        den: d / g,
                    };
                }
            }
        }
        Self::square()
    }
}

fn gcd_u32(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoStreamInfo {
    pub codec_name: String,
    pub pix_fmt: Option<String>,
    pub coded_width: u32,
    pub coded_height: u32,
    pub sar: Sar,
    /// Rotation normalized to 0, 90, 180, 270 (or other angle mod 360).
    pub rotation: i32,
    pub fps: Fps,
}

impl VideoStreamInfo {
    /// Effective display dimensions after applying SAR and rotation.
    pub fn display_size(&self) -> (u32, u32) {
        let mut w = if self.sar.is_square() {
            self.coded_width
        } else {
            ((self.coded_width as f64) * (self.sar.num as f64) / (self.sar.den as f64)).round()
                as u32
        };
        let mut h = self.coded_height;

        if self.rotation == 90 || self.rotation == 270 {
            std::mem::swap(&mut w, &mut h);
        }

        (w.max(1), h.max(1))
    }

    pub fn is_landscape(&self) -> bool {
        let (w, h) = self.display_size();
        w > h
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioStreamInfo {
    pub codec_name: String,
    pub sample_rate: u32,
    pub channels: u32,
}

#[derive(Debug, Clone)]
pub struct ClipInfo {
    pub path: PathBuf,
    pub title: String,
    pub video: VideoStreamInfo,
    pub audio: Option<AudioStreamInfo>,
    pub duration_secs: f64,
}

#[derive(Debug, Deserialize)]
struct FfprobeOutput {
    #[serde(default)]
    streams: Vec<FfprobeStream>,
    format: Option<FfprobeFormat>,
}

#[derive(Debug, Deserialize)]
struct FfprobeFormat {
    duration: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FfprobeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    pix_fmt: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    sample_aspect_ratio: Option<String>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    duration: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u32>,
    #[serde(default)]
    disposition: HashMap<String, i64>,
    #[serde(default)]
    tags: HashMap<String, String>,
    #[serde(default)]
    side_data_list: Vec<FfprobeSideData>,
}

#[derive(Debug, Deserialize)]
struct FfprobeSideData {
    side_data_type: Option<String>,
    rotation: Option<f64>,
}

pub fn probe_clip(ffprobe_path: &Path, input: &Path) -> Result<ClipInfo> {
    let output = Command::new(ffprobe_path)
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(input)
        .output()
        .with_context(|| {
            format!(
                "failed to execute ffprobe ({}) on {}",
                ffprobe_path.display(),
                input.display()
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "ffprobe failed for {} (exit {}): {}",
            input.display(),
            output.status,
            stderr.trim()
        );
    }

    let json = String::from_utf8(output.stdout)
        .with_context(|| format!("ffprobe returned non-UTF-8 output for {}", input.display()))?;

    parse_ffprobe_json(&json, input)
}

pub fn parse_ffprobe_json(json: &str, input: &Path) -> Result<ClipInfo> {
    let parsed: FfprobeOutput = serde_json::from_str(json)
        .with_context(|| format!("failed to parse ffprobe JSON for {}", input.display()))?;

    let video_stream = parsed
        .streams
        .iter()
        .find(|s| {
            s.codec_type.as_deref() == Some("video")
                && s.disposition.get("attached_pic").copied().unwrap_or(0) != 1
        })
        .ok_or_else(|| anyhow!("no video stream found in {}", input.display()))?;

    let coded_width = video_stream
        .width
        .filter(|&w| w > 0)
        .ok_or_else(|| anyhow!("missing video width in {}", input.display()))?;
    let coded_height = video_stream
        .height
        .filter(|&h| h > 0)
        .ok_or_else(|| anyhow!("missing video height in {}", input.display()))?;

    let codec_name = video_stream
        .codec_name
        .clone()
        .unwrap_or_else(|| "unknown".to_string());

    let sar = Sar::parse(video_stream.sample_aspect_ratio.as_deref());
    let rotation = extract_rotation(video_stream);

    let fps = parse_probe_fps(
        video_stream.avg_frame_rate.as_deref(),
        video_stream.r_frame_rate.as_deref(),
    )
    .ok_or_else(|| anyhow!("could not determine valid FPS for {}", input.display()))?;

    let audio = parsed
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("audio"))
        .map(|a| AudioStreamInfo {
            codec_name: a
                .codec_name
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
            sample_rate: a
                .sample_rate
                .as_deref()
                .and_then(|sr| sr.parse::<u32>().ok())
                .unwrap_or(0),
            channels: a.channels.unwrap_or(0),
        });

    let duration_secs = parsed
        .format
        .as_ref()
        .and_then(|f| parse_positive_f64(f.duration.as_deref()))
        .or_else(|| parse_positive_f64(video_stream.duration.as_deref()))
        .ok_or_else(|| anyhow!("could not determine duration for {}", input.display()))?;

    let title = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("clip")
        .to_string();

    Ok(ClipInfo {
        path: input.to_path_buf(),
        title,
        video: VideoStreamInfo {
            codec_name,
            pix_fmt: video_stream.pix_fmt.clone(),
            coded_width,
            coded_height,
            sar,
            rotation,
            fps,
        },
        audio,
        duration_secs,
    })
}

fn extract_rotation(stream: &FfprobeStream) -> i32 {
    for sd in &stream.side_data_list {
        if let Some(rot) = sd.rotation {
            return normalize_rotation(rot.round() as i32);
        }
        if let Some(sd_type) = &sd.side_data_type {
            if sd_type.to_ascii_lowercase().contains("display matrix") {
                if let Some(rot) = sd.rotation {
                    return normalize_rotation(rot.round() as i32);
                }
            }
        }
    }

    if let Some(rot_str) = stream.tags.get("rotate") {
        if let Ok(rot) = rot_str.trim().parse::<f64>() {
            return normalize_rotation(rot.round() as i32);
        }
    }

    0
}

fn normalize_rotation(deg: i32) -> i32 {
    deg.rem_euclid(360)
}

fn parse_probe_fps(avg: Option<&str>, r_fps: Option<&str>) -> Option<Fps> {
    for candidate in [avg, r_fps].into_iter().flatten() {
        let trimmed = candidate.trim();
        if trimmed.is_empty() || trimmed == "0/0" || trimmed == "N/A" {
            continue;
        }
        if let Ok(fps) = trimmed.parse::<Fps>() {
            if fps.as_f64() > 0.0 && fps.as_f64() <= 1000.0 {
                return Some(fps);
            }
        }
    }
    None
}

fn parse_positive_f64(s: Option<&str>) -> Option<f64> {
    let val = s?.trim().parse::<f64>().ok()?;
    if val.is_finite() && val > 0.0 {
        Some(val)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rotated_vertical_clip_json() {
        let json = r#"{
            "streams": [
                {
                    "codec_type": "video",
                    "codec_name": "h264",
                    "pix_fmt": "yuv420p",
                    "width": 1920,
                    "height": 1080,
                    "sample_aspect_ratio": "1:1",
                    "avg_frame_rate": "30000/1001",
                    "r_frame_rate": "30000/1001",
                    "side_data_list": [
                        {
                            "side_data_type": "Display Matrix",
                            "rotation": -90
                        }
                    ]
                },
                {
                    "codec_type": "audio",
                    "codec_name": "aac",
                    "sample_rate": "48000",
                    "channels": 2
                }
            ],
            "format": {
                "duration": "12.500000"
            }
        }"#;

        let clip = parse_ffprobe_json(json, Path::new("C:/clips/vertical_shot.mp4")).unwrap();
        assert_eq!(clip.title, "vertical_shot");
        assert_eq!(clip.video.rotation, 270);
        assert_eq!(clip.video.display_size(), (1080, 1920));
        assert!(!clip.video.is_landscape());
        assert_eq!(clip.video.fps.to_ffmpeg_expr(), "30000/1001");
        assert_eq!(clip.audio.as_ref().unwrap().sample_rate, 48000);
        assert!((clip.duration_secs - 12.5).abs() < 1e-6);
    }

    #[test]
    fn parse_anamorphic_sar_clip() {
        let json = r#"{
            "streams": [
                {
                    "codec_type": "video",
                    "codec_name": "h264",
                    "pix_fmt": "yuv420p",
                    "width": 1440,
                    "height": 1080,
                    "sample_aspect_ratio": "4:3",
                    "avg_frame_rate": "25/1"
                }
            ],
            "format": {
                "duration": "4.0"
            }
        }"#;

        let clip = parse_ffprobe_json(json, Path::new("hdv.mp4")).unwrap();
        assert_eq!(clip.video.sar, Sar { num: 4, den: 3 });
        assert_eq!(clip.video.display_size(), (1920, 1080));
        assert!(clip.video.is_landscape());
        assert!(clip.audio.is_none());
    }
}
