//! CubeLauncher core: everything needed to install and start Minecraft Java
//! Edition without a web view. The Tauri layer is a thin adapter over this crate,
//! which keeps the logic testable and the UI replaceable.

pub mod download;
pub mod forge;
pub mod install;
pub mod instance;
pub mod java;
pub mod launch;
pub mod meta;
pub mod rules;
pub mod types;

pub use download::{DownloadItem, Downloader, Progress};
pub use types::*;

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct CorePaths {
    pub root: PathBuf,
    pub instances: PathBuf,
    pub libraries: PathBuf,
    pub assets: PathBuf,
    pub runtimes: PathBuf,
    pub metadata: PathBuf,
    pub logs: PathBuf,
    pub downloads: PathBuf,
}

impl CorePaths {
    pub fn from_settings(settings: &AppSettings) -> Self {
        let root = settings.data_dir.clone();
        Self {
            instances: root.join("instances"),
            libraries: root.join("libraries"),
            assets: root.join("assets"),
            runtimes: root.join("runtimes"),
            metadata: root.join("metadata"),
            logs: root.join("logs"),
            downloads: root.join("downloads"),
            root,
        }
    }
    pub async fn ensure(&self) -> Result<()> {
        for path in [
            &self.root,
            &self.instances,
            &self.libraries,
            &self.assets,
            &self.runtimes,
            &self.metadata,
            &self.logs,
            &self.downloads,
        ] {
            tokio::fs::create_dir_all(path).await?;
        }
        Ok(())
    }
    pub fn instance_dir(&self, id: &str) -> PathBuf {
        self.instances.join(id)
    }
    pub fn game_dir(&self, id: &str) -> PathBuf {
        self.instance_dir(id).join(".minecraft")
    }
    pub fn natives_dir(&self, id: &str) -> PathBuf {
        self.instance_dir(id).join("natives")
    }
    pub fn versions_dir(&self) -> PathBuf {
        self.metadata.join("versions")
    }
}

pub struct LauncherCore {
    pub paths: CorePaths,
    pub settings: AppSettings,
    pub downloader: Downloader,
    pub processes: std::sync::Arc<launch::ProcessRegistry>,
}

impl LauncherCore {
    pub async fn new(settings: AppSettings) -> Result<Self> {
        let paths = CorePaths::from_settings(&settings);
        paths.ensure().await?;
        let downloader = Downloader::new(
            settings.download_concurrency,
            settings.offline_mode,
            settings.mirror_base_url.clone(),
        )?;
        Ok(Self {
            paths,
            settings,
            downloader,
            processes: std::sync::Arc::new(launch::ProcessRegistry::default()),
        })
    }

    /// Load persisted settings, falling back to defaults when absent or unreadable.
    pub async fn load_settings(default: AppSettings) -> AppSettings {
        let path = CorePaths::from_settings(&default)
            .root
            .join("settings.json");
        let Ok(bytes) = tokio::fs::read(&path).await else {
            return default;
        };
        match serde_json::from_slice::<AppSettings>(&bytes) {
            Ok(settings) => settings,
            Err(error) => {
                tracing::warn!("设置文件无法解析，已回退到默认设置：{error}");
                let backup = path.with_extension("json.broken");
                let _ = tokio::fs::rename(&path, backup).await;
                default
            }
        }
    }

    pub async fn save_settings(&self) -> Result<()> {
        self.paths.ensure().await?;
        atomic_write(
            &self.paths.root.join("settings.json"),
            &serde_json::to_vec_pretty(&self.settings)?,
        )
        .await
    }

    /// Replace settings and rebuild everything derived from them.
    pub async fn apply_settings(&mut self, settings: AppSettings) -> Result<()> {
        self.paths = CorePaths::from_settings(&settings);
        self.paths.ensure().await?;
        self.downloader = Downloader::new(
            settings.download_concurrency,
            settings.offline_mode,
            settings.mirror_base_url.clone(),
        )?;
        self.settings = settings;
        self.save_settings().await
    }

    pub async fn instance(&self, id: &str) -> Result<Instance> {
        let path = self.paths.instance_dir(id).join("instance.json");
        let bytes = tokio::fs::read(&path)
            .await
            .with_context(|| format!("找不到实例 {id}"))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub async fn install_lock(&self, id: &str) -> Option<InstallLock> {
        let path = self.paths.instance_dir(id).join("install.lock");
        let bytes = tokio::fs::read(&path).await.ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Log file for one instance, appended by the launcher for each run.
    pub async fn append_log(&self, id: &str, line: &str) {
        let path = self.paths.logs.join(format!("{id}.log"));
        if let Ok(mut file) = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
        {
            use tokio::io::AsyncWriteExt;
            let _ = file.write_all(line.as_bytes()).await;
            let _ = file.write_all(b"\n").await;
        }
    }

    pub async fn read_log(&self, id: &str, max_lines: usize) -> Result<Vec<String>> {
        let path = self.paths.logs.join(format!("{id}.log"));
        let content = match tokio::fs::read_to_string(&path).await {
            Ok(content) => content,
            Err(_) => return Ok(Vec::new()),
        };
        let lines: Vec<String> = content.lines().map(|line| line.to_string()).collect();
        let start = lines.len().saturating_sub(max_lines);
        Ok(lines[start..].to_vec())
    }
}

/// Write a file atomically so an interrupted write cannot corrupt state.
pub async fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let temp = temp_sibling(path);
    tokio::fs::write(&temp, data).await?;
    tokio::fs::rename(&temp, path).await?;
    Ok(())
}

fn temp_sibling(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    name.push_str(&format!(".tmp{}", std::process::id()));
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_derived_from_data_dir() {
        let settings = AppSettings {
            data_dir: PathBuf::from("/tmp/cube-test"),
            ..Default::default()
        };
        let paths = CorePaths::from_settings(&settings);
        assert_eq!(paths.libraries, PathBuf::from("/tmp/cube-test/libraries"));
        assert_eq!(
            paths.game_dir("abc"),
            PathBuf::from("/tmp/cube-test/instances/abc/.minecraft")
        );
        assert_eq!(
            paths.natives_dir("abc"),
            PathBuf::from("/tmp/cube-test/instances/abc/natives")
        );
    }

    #[tokio::test]
    async fn atomic_write_replaces_contents() {
        let dir = std::env::temp_dir().join(format!("cube-atomic-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.expect("dir");
        let path = dir.join("state.json");
        atomic_write(&path, b"first").await.expect("write");
        atomic_write(&path, b"second").await.expect("rewrite");
        let content = tokio::fs::read_to_string(&path).await.expect("read");
        assert_eq!(content, "second");
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
