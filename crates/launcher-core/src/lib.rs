//! CubeLauncher core: everything needed to install and start Minecraft Java
//! Edition without a web view. The Tauri layer is a thin adapter over this crate,
//! which keeps the logic testable and the UI replaceable.

pub mod auth;
pub mod download;
pub mod forge;
pub mod gamedir;
pub mod install;
pub mod instance;
pub mod java;
pub mod launch;
pub mod meta;
mod mirror;
pub mod rules;
pub mod types;

pub use auth::MicrosoftAuth;
pub use download::{DownloadItem, Downloader, Progress};
pub use gamedir::InstanceLayout;
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
    /// 正版 sign-in chain, always using the application id compiled into the launcher.
    pub auth: auth::MicrosoftAuth,
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
        let auth = auth::MicrosoftAuth::new()?;
        Ok(Self {
            paths,
            settings,
            downloader,
            auth,
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
    pub async fn apply_settings(&mut self, mut settings: AppSettings) -> Result<()> {
        if !(1..=16).contains(&settings.download_concurrency) {
            anyhow::bail!("下载并发数必须是 1–16 之间的整数");
        }
        settings.mirror_base_url = mirror::normalize_base(settings.mirror_base_url.as_deref())?;
        settings.java_mirror_base_url =
            mirror::normalize_base(settings.java_mirror_base_url.as_deref())?;
        let paths = CorePaths::from_settings(&settings);
        let downloader = Downloader::new(
            settings.download_concurrency,
            settings.offline_mode,
            settings.mirror_base_url.clone(),
        )?;
        let auth = auth::MicrosoftAuth::new()?;
        paths.ensure().await?;
        atomic_write(
            &paths.root.join("settings.json"),
            &serde_json::to_vec_pretty(&settings)?,
        )
        .await?;
        self.paths = paths;
        self.downloader = downloader;
        self.auth = auth;
        self.settings = settings;
        Ok(())
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

/// Like [`atomic_write`], but the file is created owner-only on Unix. Used for
/// `accounts.json`, which holds the Microsoft refresh tokens.
pub async fn atomic_write_private(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let temp = temp_sibling(path);
    tokio::fs::write(&temp, data).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Set the mode before the rename: after it, the secret would already be
        // readable under whatever the umask allowed.
        tokio::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600)).await?;
    }
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
    async fn download_settings_validate_before_changing_state() {
        let root = std::env::temp_dir().join(format!(
            "cube-download-settings-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let original = AppSettings {
            data_dir: root.clone(),
            offline_mode: true,
            ..Default::default()
        };
        let mut core = LauncherCore::new(original.clone()).await.unwrap();
        core.save_settings().await.unwrap();
        let before = tokio::fs::read(root.join("settings.json")).await.unwrap();
        for (count, mirror) in [(0, None), (17, None), (4, Some("file:///tmp/mirror"))] {
            let mut invalid = original.clone();
            invalid.download_concurrency = count;
            invalid.mirror_base_url = mirror.map(str::to_string);
            assert!(core.apply_settings(invalid).await.is_err());
            assert_eq!(core.settings, original);
            assert_eq!(
                tokio::fs::read(root.join("settings.json")).await.unwrap(),
                before
            );
        }
        let mut valid = original.clone();
        valid.download_concurrency = 8;
        valid.mirror_base_url = Some(" https://bmclapi2.bangbang93.com/ ".into());
        valid.java_mirror_base_url = Some(" ".into());
        core.apply_settings(valid).await.unwrap();
        let reloaded = LauncherCore::load_settings(original).await;
        assert_eq!(reloaded.download_concurrency, 8);
        assert_eq!(
            reloaded.mirror_base_url.as_deref(),
            Some("https://bmclapi2.bangbang93.com")
        );
        assert_eq!(reloaded.java_mirror_base_url, None);
        assert_eq!(reloaded, core.settings);
        assert_eq!(root.parent(), Some(std::env::temp_dir().as_path()));
        assert!(root
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("cube-download-settings-"));
        tokio::fs::remove_dir_all(root).await.unwrap();
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
