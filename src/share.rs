use anyhow::{Context, Result, bail};
use reqwest::header::HeaderMap;
use std::path::Path;
use tokio::fs::File;
use tokio_util::io::ReaderStream;

pub struct UploadResult {
    pub url: String,
    pub delete_token: String,
}

/// Upload a file to 0x0.st.
/// Returns the public URL and the deletion token.
pub async fn upload(path: &Path) -> Result<UploadResult> {
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

    let part = reqwest::multipart::Part::stream(body).file_name(filename);
    let form = reqwest::multipart::Form::new().part("file", part);

    let client = reqwest::Client::new();
    let response = client
        .post("https://0x0.st")
        .multipart(form)
        .send()
        .await
        .context("sending upload request")?;

    if !response.status().is_success() {
        bail!("0x0.st returned HTTP {}", response.status());
    }

    let headers: HeaderMap = response.headers().clone();
    let url = response
        .text()
        .await
        .context("reading response body")?
        .trim()
        .to_owned();

    if url.is_empty() || !url.starts_with("http") {
        bail!("unexpected response from 0x0.st: {:?}", url);
    }

    let delete_token = headers
        .get("X-Token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();

    Ok(UploadResult { url, delete_token })
}

/// Delete a previously uploaded file using its deletion token.
pub async fn delete(url: &str, token: &str) -> Result<()> {
    let client = reqwest::Client::new();
    let response = client
        .post(url)
        .form(&[("token", token), ("delete", "")])
        .send()
        .await
        .context("sending delete request")?;

    if !response.status().is_success() {
        bail!("0x0.st delete returned HTTP {}", response.status());
    }

    Ok(())
}
