use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// A single post-processing effect applied in the order listed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Effect {
    /// Boost saturation. 1.0 = neutral, 1.3 = moderate pop.
    Saturation(f32),
    /// Sharpen strength passed to unsharp mask. 0.5–2.0 typical.
    Sharpen(f32),
    /// Frame interpolation — target fps, e.g. 120 or 240.
    Interpolate { target_fps: u32 },
    /// Motion blur — shutter angle in degrees (180 = natural, 360 = heavy).
    MotionBlur { shutter_angle: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CompressCodec {
    /// H.264 via NVENC (fast, good compatibility)
    H264Nvenc,
    /// H.265/HEVC via NVENC (smaller files, GPU required)
    HevcNvenc,
    /// H.265/HEVC via CPU (x265, no GPU needed, slower)
    Hevc,
    /// AV1 via CPU (libaom-av1, best compression, very slow)
    Av1,
}

impl CompressCodec {
    pub fn label(&self) -> &'static str {
        match self {
            Self::H264Nvenc => "H.264 NVENC",
            Self::HevcNvenc => "H.265 NVENC",
            Self::Hevc => "H.265 CPU",
            Self::Av1 => "AV1",
        }
    }

    pub fn ffmpeg_codec(&self) -> &'static str {
        match self {
            Self::H264Nvenc => "h264_nvenc",
            Self::HevcNvenc => "hevc_nvenc",
            Self::Hevc => "libx265",
            Self::Av1 => "libaom-av1",
        }
    }

    /// Returns true if this codec uses NVENC hardware encoding.
    pub fn is_nvenc(&self) -> bool {
        matches!(self, Self::H264Nvenc | Self::HevcNvenc)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CompressQuality {
    High,
    Med,
    Low,
}

impl CompressQuality {
    pub fn label(&self) -> &'static str {
        match self {
            Self::High => "High",
            Self::Med => "Med",
            Self::Low => "Low",
        }
    }

    /// CQ value for NVENC or CRF value for software encoders.
    pub fn crf(&self, codec: &CompressCodec) -> u8 {
        match codec {
            CompressCodec::H264Nvenc => match self {
                Self::High => 18,
                Self::Med => 23,
                Self::Low => 28,
            },
            CompressCodec::HevcNvenc => match self {
                Self::High => 20,
                Self::Med => 26,
                Self::Low => 32,
            },
            CompressCodec::Hevc => match self {
                Self::High => 22,
                Self::Med => 28,
                Self::Low => 34,
            },
            CompressCodec::Av1 => match self {
                Self::High => 28,
                Self::Med => 38,
                Self::Low => 50,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompressOptions {
    pub codec: CompressCodec,
    pub quality: CompressQuality,
    /// Optional trim start as HH:MM:SS or SS.sss
    pub trim_start: Option<String>,
    /// Optional trim end as HH:MM:SS or SS.sss
    pub trim_end: Option<String>,
}

impl Default for CompressOptions {
    fn default() -> Self {
        Self {
            codec: CompressCodec::H264Nvenc,
            quality: CompressQuality::High,
            trim_start: None,
            trim_end: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum JobKind {
    /// Fast Lanczos upscale via ffmpeg — runs immediately after buffer save
    Upscale,
    /// User-triggered post-processing pipeline: sat/sharpen/interp/blur
    PostProcess { effects: Vec<Effect> },
    /// User-triggered compression with codec/quality presets + optional trim
    Compress(CompressOptions),
    /// Upload output file to 0x0.st and store the share URL + deletion token
    Share,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Running,
    Done,
    Failed(String),
    Cancelled,
}

impl JobStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done | Self::Failed(_) | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: Uuid,
    pub kind: JobKind,
    pub source: PathBuf,
    pub output: PathBuf,
    pub status: JobStatus,
    /// 0–100, only meaningful when Running
    pub progress: u8,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    /// Populated after a Share job completes successfully.
    pub share_url: Option<String>,
    /// 0x0.st deletion token, populated after a Share job completes.
    pub share_token: Option<String>,
}

impl Job {
    pub fn new(kind: JobKind, source: PathBuf, output: PathBuf) -> Self {
        Self {
            id: Uuid::new_v4(),
            kind,
            source,
            output,
            status: JobStatus::Queued,
            progress: 0,
            created_at: Utc::now(),
            finished_at: None,
            share_url: None,
            share_token: None,
        }
    }

    pub fn display_name(&self) -> String {
        self.source
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    }

    pub fn kind_label(&self) -> &'static str {
        match &self.kind {
            JobKind::Upscale => "Upscale",
            JobKind::PostProcess { .. } => "Post-Process",
            JobKind::Compress(_) => "Compress",
            JobKind::Share => "Share",
        }
    }
}
