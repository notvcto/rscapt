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
/// Returns the public URL. Retries up to 3 times on transient server errors.
pub async fn upload(path: &Path, expiry: &str) -> Result<String> {
    let file_size = tokio::fs::metadata(path)
        .await
        .with_context(|| format!("stat {}", path.display()))?
        .len();

    if file_size == 0 {
        bail!("file is empty (0 bytes) — nothing to upload");
    }

    let client = reqwest::Client::builder()
        .user_agent(format!("rscapt/{}", env!("CARGO_PKG_VERSION")))
        .build()?;

    let mut attempts = 0u32;
    let response = loop {
        attempts += 1;
        let form = reqwest::multipart::Form::new()
            .text("reqtype", "fileupload")
            .text("time", expiry.to_owned())
            .part("fileToUpload", build_file_part(path, file_size).await?);

        match client
            .post("https://litterbox.catbox.moe/resources/internals/api.php")
            .multipart(form)
            .send()
            .await
        {
            Ok(r) if r.status().is_server_error() && attempts < 3 => {
                let status = r.status();
                tracing::warn!(attempt = attempts, %status, "litterbox transient error, retrying");
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
            Ok(r) => break r,
            Err(e) if attempts < 3 => {
                tracing::warn!(attempt = attempts, error = %e, "litterbox request failed, retrying");
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
            Err(e) => bail!("upload failed after {attempts} attempts: {e}"),
        }
    };

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

async fn build_file_part(path: &Path, file_size: u64) -> Result<reqwest::multipart::Part> {
    let filename = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let file = File::open(path)
        .await
        .with_context(|| format!("opening {}", path.display()))?;
    let stream = ReaderStream::new(file);
    let body = reqwest::Body::wrap_stream(stream);
    Ok(reqwest::multipart::Part::stream_with_length(body, file_size)
        .file_name(filename))
}
