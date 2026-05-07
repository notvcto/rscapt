use anyhow::{Context, Result, bail};
use std::path::Path;
use tokio::fs::File;
use tokio_util::io::ReaderStream;

/// Valid litterbox expiry values, in display order.
pub const EXPIRY_OPTIONS: &[(&str, &str)] = &[
    ("1h",  "1 hour"),
    ("12h", "12 hours"),
    ("24h", "24 hours"),
    ("72h", "72 hours"),
    ("1w",  "1 week"),
];

pub fn expiry_idx(s: &str) -> usize {
    EXPIRY_OPTIONS
        .iter()
        .position(|(k, _)| *k == s)
        .unwrap_or(4) // default: 1w
}

/// Upload a file to litterbox.catbox.moe.
/// Returns the public URL.
pub async fn upload(path: &Path, expiry: &str) -> Result<String> {
    let filename = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();

    let file_size = tokio::fs::metadata(path)
        .await
        .with_context(|| format!("stat {}", path.display()))?
        .len();

    let file = File::open(path)
        .await
        .with_context(|| format!("opening {}", path.display()))?;

    let stream = ReaderStream::new(file);
    let body = reqwest::Body::wrap_stream(stream);

    let file_part = reqwest::multipart::Part::stream_with_length(body, file_size)
        .file_name(filename);

    let form = reqwest::multipart::Form::new()
        .text("reqtype", "fileupload")
        .text("time", expiry.to_owned())
        .part("fileToUpload", file_part);

    let client = reqwest::Client::builder()
        .user_agent(format!("rscapt/{}", env!("CARGO_PKG_VERSION")))
        .build()?;

    let response = client
        .post("https://litterbox.catbox.moe/resources/internals/api.php")
        .multipart(form)
        .send()
        .await
        .context("sending upload request")?;

    if !response.status().is_success() {
        bail!("litterbox returned HTTP {}", response.status());
    }

    let url = response
        .text()
        .await
        .context("reading response body")?
        .trim()
        .to_owned();

    if url.is_empty() || !url.starts_with("http") {
        bail!("unexpected response from litterbox: {:?}", url);
    }

    Ok(url)
}
