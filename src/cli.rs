use anyhow::{anyhow, bail, Result};
use clap::{Parser, ValueEnum};
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

#[derive(Debug, Clone, Parser)]
#[command(
    name = "mi-merge",
    version,
    about = "Merge video clips end to end with chapters and automatic pillarboxing.",
    after_help = "Examples:\n  mi-merge clip1.mp4 clip2.mp4 -o merged.mp4\n  mi-merge ./clips -o merged.mp4 -y\n  mi-merge ./clips --resolution 1920x1080 --fps 60 --encoder nvenc -o merged.mp4\n  mi-merge ./clips --dry-run -o merged.mp4"
)]
pub struct Cli {
    /// Input video files or folders (folders are expanded in natural sort order)
    #[arg(required = true, value_name = "INPUT")]
    pub inputs: Vec<PathBuf>,

    /// Output video file path
    #[arg(short = 'o', long = "output", required = true, value_name = "FILE")]
    pub output: PathBuf,

    /// Overwrite output file if it already exists
    #[arg(short = 'y', long = "overwrite")]
    pub overwrite: bool,

    /// Print the ffmpeg command and merge plan without executing
    #[arg(long = "dry-run")]
    pub dry_run: bool,

    /// Override output resolution (e.g. 1920x1080)
    #[arg(long = "resolution", value_name = "WxH")]
    pub resolution: Option<Resolution>,

    /// Override output frame rate (e.g. 30, 60, 29.97, 30000/1001)
    #[arg(long = "fps", value_name = "FPS")]
    pub fps: Option<Fps>,

    /// Video encoder to use when transcoding
    #[arg(long = "encoder", value_enum, default_value_t = Encoder::X264)]
    pub encoder: Encoder,

    /// Path to ffmpeg executable (or directory containing ffmpeg and ffprobe)
    #[arg(long = "ffmpeg", value_name = "PATH")]
    pub ffmpeg: Option<PathBuf>,

    /// Path to ffprobe executable
    #[arg(long = "ffprobe", value_name = "PATH")]
    pub ffprobe: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Encoder {
    /// Software H.264 encoder (libx264, veryfast, CRF 20)
    X264,
    /// NVIDIA hardware H.264 encoder (h264_nvenc)
    Nvenc,
    /// Intel Quick Sync Video H.264 encoder (h264_qsv)
    Qsv,
    /// AMD AMF H.264 encoder (h264_amf)
    Amf,
}

impl Encoder {
    pub fn ffmpeg_args(self) -> Vec<String> {
        match self {
            Encoder::X264 => vec![
                "-c:v".into(),
                "libx264".into(),
                "-preset".into(),
                "veryfast".into(),
                "-crf".into(),
                "20".into(),
            ],
            Encoder::Nvenc => vec![
                "-c:v".into(),
                "h264_nvenc".into(),
                "-preset".into(),
                "p4".into(),
                "-rc".into(),
                "vbr".into(),
                "-cq".into(),
                "20".into(),
            ],
            Encoder::Qsv => vec![
                "-c:v".into(),
                "h264_qsv".into(),
                "-preset".into(),
                "veryfast".into(),
                "-global_quality".into(),
                "20".into(),
            ],
            Encoder::Amf => vec![
                "-c:v".into(),
                "h264_amf".into(),
                "-quality".into(),
                "balanced".into(),
                "-rc".into(),
                "cqp".into(),
                "-qp_i".into(),
                "20".into(),
                "-qp_p".into(),
                "20".into(),
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}

impl Resolution {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        if width == 0 || height == 0 {
            bail!("resolution dimensions must be greater than zero");
        }
        if width % 2 != 0 || height % 2 != 0 {
            bail!(
                "resolution {}x{} must have even width and height for yuv420p compatibility",
                width,
                height
            );
        }
        Ok(Self { width, height })
    }

    pub fn make_even(width: u32, height: u32) -> Self {
        let w = (width.max(2) + 1) & !1;
        let h = (height.max(2) + 1) & !1;
        Self {
            width: w,
            height: h,
        }
    }
}

impl fmt::Display for Resolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{}", self.width, self.height)
    }
}

impl FromStr for Resolution {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let trimmed = s.trim();
        let (w_str, h_str) = trimmed
            .split_once(['x', 'X'])
            .ok_or_else(|| anyhow!("invalid resolution '{s}': expected WxH (e.g. 1920x1080)"))?;
        let width: u32 = w_str
            .trim()
            .parse()
            .map_err(|_| anyhow!("invalid width in resolution '{s}'"))?;
        let height: u32 = h_str
            .trim()
            .parse()
            .map_err(|_| anyhow!("invalid height in resolution '{s}'"))?;
        Resolution::new(width, height)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fps {
    pub num: u64,
    pub den: u64,
}

impl Fps {
    pub fn from_ratio(num: u64, den: u64) -> Result<Self> {
        if num == 0 || den == 0 {
            bail!("fps must be greater than zero");
        }
        let g = gcd(num, den);
        Ok(Self {
            num: num / g,
            den: den / g,
        })
    }

    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    pub fn to_ffmpeg_expr(self) -> String {
        if self.den == 1 {
            format!("{}", self.num)
        } else {
            format!("{}/{}", self.num, self.den)
        }
    }

    pub fn approx_eq(self, other: Self) -> bool {
        (self.as_f64() - other.as_f64()).abs() < 0.01
    }
}

impl fmt::Display for Fps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{:.3} ({}/{})", self.as_f64(), self.num, self.den)
        }
    }
}

impl FromStr for Fps {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let trimmed = s.trim();
        if let Some((n_str, d_str)) = trimmed.split_once('/') {
            let num: u64 = n_str
                .trim()
                .parse()
                .map_err(|_| anyhow!("invalid fps numerator in '{s}'"))?;
            let den: u64 = d_str
                .trim()
                .parse()
                .map_err(|_| anyhow!("invalid fps denominator in '{s}'"))?;
            return Fps::from_ratio(num, den);
        }

        if let Some((whole_str, frac_str)) = trimmed.split_once('.') {
            if frac_str.is_empty() || !frac_str.chars().all(|c| c.is_ascii_digit()) {
                bail!("invalid decimal fps '{s}'");
            }
            let scale = 10u64
                .checked_pow(frac_str.len() as u32)
                .ok_or_else(|| anyhow!("too many decimal places in fps '{s}'"))?;
            let whole: u64 = if whole_str.is_empty() {
                0
            } else {
                whole_str
                    .parse()
                    .map_err(|_| anyhow!("invalid fps '{s}'"))?
            };
            let frac: u64 = frac_str
                .parse()
                .map_err(|_| anyhow!("invalid fps '{s}'"))?;
            let num = whole
                .checked_mul(scale)
                .and_then(|v| v.checked_add(frac))
                .ok_or_else(|| anyhow!("fps overflow in '{s}'"))?;
            return Fps::from_ratio(num, scale);
        }

        let num: u64 = trimmed.parse().map_err(|_| anyhow!("invalid fps '{s}'"))?;
        Fps::from_ratio(num, 1)
    }
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_resolution_valid() {
        assert_eq!(
            "1920x1080".parse::<Resolution>().unwrap(),
            Resolution {
                width: 1920,
                height: 1080
            }
        );
        assert_eq!(
            "1280X720".parse::<Resolution>().unwrap(),
            Resolution {
                width: 1280,
                height: 720
            }
        );
    }

    #[test]
    fn parse_resolution_invalid() {
        assert!("1920".parse::<Resolution>().is_err());
        assert!("0x1080".parse::<Resolution>().is_err());
        assert!("1921x1080".parse::<Resolution>().is_err());
    }

    #[test]
    fn parse_fps_formats() {
        let f1: Fps = "30".parse().unwrap();
        assert_eq!(f1, Fps { num: 30, den: 1 });
        assert_eq!(f1.to_ffmpeg_expr(), "30");

        let f2: Fps = "30000/1001".parse().unwrap();
        assert_eq!(
            f2,
            Fps {
                num: 30000,
                den: 1001
            }
        );
        assert_eq!(f2.to_ffmpeg_expr(), "30000/1001");

        let f3: Fps = "29.97".parse().unwrap();
        assert_eq!(
            f3,
            Fps {
                num: 2997,
                den: 100
            }
        );
        assert!(f2.approx_eq(f3));

        assert!("0".parse::<Fps>().is_err());
        assert!("30/0".parse::<Fps>().is_err());
    }
}
