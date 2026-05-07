use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{Mutex, broadcast};

/// A finished clip on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clip {
    pub path: PathBuf,
    /// Just the filename for display.
    pub filename: String,
    pub size_bytes: u64,
    /// Share URL, if uploaded.
    pub share_url: Option<String>,
    /// Deletion token (unused with litterbox, kept for forward compat).
    pub share_delete_token: Option<String>,
}

impl Clip {
    pub fn from_path(path: PathBuf) -> Result<Self> {
        let metadata = std::fs::metadata(&path)?;
        let filename = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        Ok(Self {
            path,
            filename,
            size_bytes: metadata.len(),
            share_url: None,
            share_delete_token: None,
        })
    }

    /// Human-readable file size (e.g. "234 MB").
    pub fn size_label(&self) -> String {
        let mb = self.size_bytes as f64 / 1_048_576.0;
        if mb >= 1024.0 {
            format!("{:.1} GB", mb / 1024.0)
        } else {
            format!("{:.0} MB", mb)
        }
    }


}

// ── ShareStore ─────────────────────────────────────────────────────────────────

/// Persisted share token store: maps absolute path string → (url, token).
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ShareStore(pub HashMap<String, ShareEntry>);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareEntry {
    pub url: String,
    pub token: String,
}

impl ShareStore {
    fn path(data_dir: &Path) -> PathBuf {
        data_dir.join("shares.json")
    }

    pub fn load(data_dir: &Path) -> Self {
        let p = Self::path(data_dir);
        if let Ok(text) = std::fs::read_to_string(&p) {
            serde_json::from_str(&text).unwrap_or_default()
        } else {
            Self::default()
        }
    }

    pub fn save(&self, data_dir: &Path) -> Result<()> {
        let p = Self::path(data_dir);
        std::fs::create_dir_all(data_dir)?;
        std::fs::write(p, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn set(&mut self, path: &Path, url: String, token: String) {
        self.0.insert(
            path.to_string_lossy().into_owned(),
            ShareEntry { url, token },
        );
    }

    pub fn remove(&mut self, path: &Path) {
        self.0.remove(&path.to_string_lossy().into_owned());
    }

    pub fn get(&self, path: &Path) -> Option<&ShareEntry> {
        self.0.get(&path.to_string_lossy().into_owned())
    }
}

// ── ClipStore ──────────────────────────────────────────────────────────────────

/// Thread-safe, event-broadcasting clip library.
pub struct ClipStore {
    clips: Mutex<Vec<Clip>>,
    shares: Mutex<ShareStore>,
    tx: broadcast::Sender<Vec<Clip>>,
    data_dir: PathBuf,
    output_dir: PathBuf,
}

impl ClipStore {
    pub fn new(data_dir: PathBuf, output_dir: PathBuf) -> Arc<Self> {
        let (tx, _) = broadcast::channel(16);
        let store = Arc::new(Self {
            clips: Mutex::new(Vec::new()),
            shares: Mutex::new(ShareStore::default()),
            tx,
            data_dir,
            output_dir,
        });
        store
    }

    /// Load share store from disk and scan output dir for video files.
    pub async fn init(&self) {
        let share_store = ShareStore::load(&self.data_dir);
        let clips = scan_dir(&self.output_dir, &share_store);
        *self.shares.lock().await = share_store;
        *self.clips.lock().await = clips;
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Vec<Clip>> {
        self.tx.subscribe()
    }

    pub async fn snapshot(&self) -> Vec<Clip> {
        self.clips.lock().await.clone()
    }

    /// Re-scan the output directory, merging existing share data.
    pub async fn refresh(&self) {
        let shares = self.shares.lock().await;
        let clips = scan_dir(&self.output_dir, &shares);
        drop(shares);
        *self.clips.lock().await = clips.clone();
        let _ = self.tx.send(clips);
    }

    /// Called when a clip file is freshly produced (after an upscale job).
    /// Adds the clip to the library if not already present.
    pub async fn add_if_new(&self, path: &Path) {
        let mut clips = self.clips.lock().await;
        if clips.iter().any(|c| c.path == path) {
            return;
        }
        if let Ok(clip) = Clip::from_path(path.to_path_buf()) {
            clips.push(clip);
            clips.sort_by(|a, b| b.filename.cmp(&a.filename));
            let snapshot = clips.clone();
            drop(clips);
            let _ = self.tx.send(snapshot);
        }
    }

    /// Update a clip's share URL and token after a successful upload.
    pub async fn set_share(&self, path: &Path, url: String, token: String) {
        {
            let mut shares = self.shares.lock().await;
            shares.set(path, url.clone(), token.clone());
            let _ = shares.save(&self.data_dir);
        }
        let mut clips = self.clips.lock().await;
        if let Some(clip) = clips.iter_mut().find(|c| c.path == path) {
            clip.share_url = Some(url);
            clip.share_delete_token = Some(token);
        }
        let snapshot = clips.clone();
        drop(clips);
        let _ = self.tx.send(snapshot);
    }

    /// Remove share data for a clip (after deletion).
    pub async fn clear_share(&self, path: &Path) {
        {
            let mut shares = self.shares.lock().await;
            shares.remove(path);
            let _ = shares.save(&self.data_dir);
        }
        let mut clips = self.clips.lock().await;
        if let Some(clip) = clips.iter_mut().find(|c| c.path == path) {
            clip.share_url = None;
            clip.share_delete_token = None;
        }
        let snapshot = clips.clone();
        drop(clips);
        let _ = self.tx.send(snapshot);
    }

    /// Return the share token for a clip, if one exists.
    pub async fn get_share_token(&self, path: &Path) -> Option<String> {
        let shares = self.shares.lock().await;
        shares.get(path).map(|e| e.token.clone())
    }
}

// ── helpers ────────────────────────────────────────────────────────────────────

fn scan_dir(output_dir: &Path, store: &ShareStore) -> Vec<Clip> {
    let extensions = ["mp4", "mkv", "mov", "avi", "webm"];
    let mut clips: Vec<Clip> = Vec::new();

    let entries = match std::fs::read_dir(output_dir) {
        Ok(e) => e,
        Err(_) => return clips,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if !extensions.contains(&ext.as_str()) {
            continue;
        }
        if let Ok(mut clip) = Clip::from_path(path.clone()) {
            if let Some(entry) = store.get(&path) {
                clip.share_url = Some(entry.url.clone());
                clip.share_delete_token = Some(entry.token.clone());
            }
            clips.push(clip);
        }
    }

    clips.sort_by(|a, b| b.filename.cmp(&a.filename));
    clips
}
