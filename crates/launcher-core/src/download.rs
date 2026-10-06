use crate::types::{DownloadInfo, InstallTask, RESOURCES_URL};
use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use reqwest::header::RANGE;
use sha1::{Digest as _, Sha1};
use sha2::Sha256;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, Semaphore};

/// A single file to place on disk, with whatever integrity data upstream provides.
#[derive(Debug, Clone)]
pub struct DownloadItem {
    pub url: String,
    pub dest: PathBuf,
    pub sha1: Option<String>,
    pub sha256: Option<String>,
    pub size: Option<u64>,
    pub label: String,
}

impl DownloadItem {
    pub fn new(url: impl Into<String>, dest: PathBuf) -> Self {
        Self {
            url: url.into(),
            dest,
            sha1: None,
            sha256: None,
            size: None,
            label: String::new(),
        }
    }
    pub fn with_sha1(mut self, sha1: Option<String>) -> Self {
        self.sha1 = sha1;
        self
    }
    pub fn with_size(mut self, size: Option<u64>) -> Self {
        self.size = size;
        self
    }
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }
    /// True when the destination already matches every recorded integrity value.
    pub fn is_satisfied(&self) -> bool {
        let Ok(meta) = std::fs::metadata(&self.dest) else {
            return false;
        };
        if !meta.is_file() {
            return false;
        }
        if let Some(size) = self.size {
            if meta.len() != size {
                return false;
            }
        }
        if self.sha1.is_none() && self.sha256.is_none() && self.size.is_none() {
            // Nothing to verify against: treat as missing so it is re-fetched.
            return false;
        }
        verify_sync(&self.dest, self.sha1.as_deref(), self.sha256.as_deref()).unwrap_or(false)
    }
}

pub fn verify_sync(path: &Path, sha1: Option<&str>, sha256: Option<&str>) -> Result<bool> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut sha1_hasher = Sha1::new();
    let mut sha256_hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        if sha1.is_some() {
            sha1_hasher.update(&buffer[..read]);
        }
        if sha256.is_some() {
            sha256_hasher.update(&buffer[..read]);
        }
        if sha1.is_none() && sha256.is_none() {
            break;
        }
    }
    if let Some(expected) = sha1 {
        if hex::encode(sha1_hasher.finalize()) != expected.to_lowercase() {
            return Ok(false);
        }
    }
    if let Some(expected) = sha256 {
        if hex::encode(sha256_hasher.finalize()) != expected.to_lowercase() {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Progress channel wrapper shared by installers and downloaders.
#[derive(Clone)]
pub struct Progress {
    tx: Option<mpsc::Sender<InstallTask>>,
    pub task_id: String,
    pub instance_id: String,
    pub cancel: Arc<AtomicBool>,
    done: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
}

impl Progress {
    pub fn new(tx: Option<mpsc::Sender<InstallTask>>, instance_id: impl Into<String>) -> Self {
        Self {
            tx,
            task_id: String::new(),
            instance_id: instance_id.into(),
            cancel: Arc::new(AtomicBool::new(false)),
            done: Arc::new(AtomicU64::new(0)),
            total: Arc::new(AtomicU64::new(0)),
        }
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    pub fn check_cancelled(&self) -> Result<()> {
        if self.cancelled() {
            bail!("任务已取消");
        }
        Ok(())
    }
    pub fn set_total(&self, total: u64) {
        self.total.store(total, Ordering::Relaxed);
    }
    pub async fn stage(&self, phase: &str, message: impl Into<String>) {
        self.send(phase, message.into(), false, None).await;
    }
    pub async fn step(&self, phase: &str, message: impl Into<String>) {
        let current = self.done.fetch_add(1, Ordering::Relaxed) + 1;
        let total = self.total.load(Ordering::Relaxed);
        let _ = total;
        self.send_counts(phase, message.into(), current, total)
            .await;
    }
    pub async fn finish(&self, message: impl Into<String>) {
        self.send("done", message.into(), true, None).await;
    }
    pub async fn fail(&self, message: impl Into<String>) {
        let message = message.into();
        self.send("error", message.clone(), true, Some(message))
            .await;
    }
    async fn send(&self, phase: &str, message: String, done: bool, error: Option<String>) {
        let total = self.total.load(Ordering::Relaxed);
        let current = self.done.load(Ordering::Relaxed);
        self.send_counts_with(phase, message, current, total, done, error)
            .await;
    }
    async fn send_counts(&self, phase: &str, message: String, current: u64, total: u64) {
        self.send_counts_with(phase, message, current, total, false, None)
            .await;
    }
    async fn send_counts_with(
        &self,
        phase: &str,
        message: String,
        current: u64,
        total: u64,
        done: bool,
        error: Option<String>,
    ) {
        if let Some(tx) = &self.tx {
            let _ = tx
                .send(InstallTask {
                    id: self.task_id.clone(),
                    instance_id: self.instance_id.clone(),
                    phase: phase.to_string(),
                    current,
                    total,
                    message,
                    done,
                    error,
                })
                .await;
        }
    }
}

/// Upstream host → path prefix on the mirror. Verified against BMCLAPI:
/// version metadata keeps its path, libraries and loader Maven live under
/// `/maven`, and asset objects under `/assets`.
const MIRROR_ROUTES: &[(&str, &str)] = &[
    ("https://piston-meta.mojang.com", ""),
    ("https://libraries.minecraft.net", "/maven"),
    ("https://maven.minecraftforge.net", "/maven"),
    ("https://maven.neoforged.net/releases", "/maven"),
    ("https://maven.fabricmc.net", "/maven"),
    ("https://resources.download.minecraft.net", "/assets"),
];

pub struct Downloader {
    client: reqwest::Client,
    concurrency: usize,
    offline: bool,
    mirror: Option<String>,
}

impl Downloader {
    pub fn new(concurrency: usize, offline: bool, mirror: Option<String>) -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(format!(
                "{}/{}",
                crate::types::LAUNCHER_NAME,
                crate::types::LAUNCHER_VERSION
            ))
            .connect_timeout(std::time::Duration::from_secs(20))
            // A stall timeout, not a total budget: large files (client jar, assets,
            // Java runtimes) may legitimately take many minutes to transfer.
            .read_timeout(std::time::Duration::from_secs(90))
            .pool_idle_timeout(std::time::Duration::from_secs(30))
            .build()?;
        Ok(Self {
            client,
            concurrency: concurrency.clamp(1, 16),
            offline,
            mirror,
        })
    }

    pub fn offline(&self) -> bool {
        self.offline
    }

    /// Rewrite an official URL onto the configured mirror.
    ///
    /// The mirror is a host that serves the same files under slightly different
    /// paths, so each upstream host gets an explicit path prefix. Only mappings
    /// that have been checked against a live mirror are listed; anything else is
    /// left untouched rather than guessed at.
    pub fn map_url(&self, url: &str) -> String {
        let Some(mirror) = &self.mirror else {
            return url.to_string();
        };
        let mirror = mirror.trim_end_matches('/');
        let mirror_host = mirror
            .split("://")
            .nth(1)
            .unwrap_or(mirror)
            .split('/')
            .next()
            .unwrap_or("")
            .to_string();
        for (upstream, mirror_prefix) in MIRROR_ROUTES {
            // Never rewrite a URL that already points at the mirror host.
            if url
                .split("://")
                .nth(1)
                .map(|rest| rest.starts_with(&mirror_host))
                .unwrap_or(false)
            {
                return url.to_string();
            }
            if let Some(path) = url.strip_prefix(upstream) {
                return format!("{mirror}{mirror_prefix}{path}");
            }
        }
        url.to_string()
    }

    /// Fetch a small resource into memory. Used for metadata documents.
    pub async fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>> {
        if self.offline {
            bail!("严格离线模式下不会访问网络：{url}");
        }
        let mapped = self.map_url(url);
        let mut last_error = None;
        for attempt in 0..3 {
            match self.client.get(&mapped).send().await {
                Ok(response) => match response.error_for_status() {
                    Ok(ok) => match ok.bytes().await {
                        Ok(bytes) => return Ok(bytes.to_vec()),
                        Err(error) => last_error = Some(anyhow!(error)),
                    },
                    Err(error) => {
                        if error.status().map(|s| s.is_client_error()).unwrap_or(false) {
                            return Err(anyhow!(
                                "请求失败（{}）：{url}",
                                error.status().unwrap().as_u16()
                            ));
                        }
                        last_error = Some(anyhow!(error));
                    }
                },
                Err(error) => last_error = Some(anyhow!(error)),
            }
            tokio::time::sleep(std::time::Duration::from_millis(400 * (attempt + 1) as u64)).await;
        }
        Err(last_error.unwrap_or_else(|| anyhow!("请求失败：{url}")))
            .with_context(|| format!("下载 {url} 失败"))
    }

    pub async fn fetch_text(&self, url: &str) -> Result<String> {
        Ok(String::from_utf8_lossy(&self.fetch_bytes(url).await?).to_string())
    }

    pub async fn fetch_json<T: for<'a> serde::Deserialize<'a>>(&self, url: &str) -> Result<T> {
        let bytes = self.fetch_bytes(url).await?;
        serde_json::from_slice(&bytes).with_context(|| format!("解析 {url} 返回的 JSON 失败"))
    }

    /// Download one file with resume, retry, and integrity verification.
    pub async fn download(&self, item: &DownloadItem, progress: &Progress) -> Result<()> {
        if item.is_satisfied() {
            return Ok(());
        }
        if self.offline {
            bail!(
                "严格离线模式缺少文件，且不允许下载：{}",
                item.dest.display()
            );
        }
        progress.check_cancelled()?;
        if let Some(parent) = item.dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        clean_legacy_partials(&item.dest).await;
        let temp = temp_path(&item.dest);
        let mut last_error = None;
        for attempt in 0..3u32 {
            progress.check_cancelled()?;
            match self.attempt(item, &temp).await {
                Ok(()) => {
                    if !verify_async(&temp, item).await? {
                        let _ = tokio::fs::remove_file(&temp).await;
                        last_error = Some(anyhow!("校验失败：{}", item.dest.display()));
                        continue;
                    }
                    tokio::fs::rename(&temp, &item.dest).await?;
                    return Ok(());
                }
                Err(error) => {
                    last_error = Some(error);
                    tokio::time::sleep(std::time::Duration::from_millis(
                        500 * (attempt + 1) as u64,
                    ))
                    .await;
                }
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow!("下载失败：{}", item.url)))
            .with_context(|| format!("下载 {}", item.dest.display()))
    }

    async fn attempt(&self, item: &DownloadItem, temp: &Path) -> Result<()> {
        let existing = tokio::fs::metadata(temp)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        // Only resume when the partial file is smaller than the expected size.
        let resume_from = match item.size {
            Some(size) if existing > 0 && existing < size => existing,
            Some(_) => {
                let _ = tokio::fs::remove_file(temp).await;
                0
            }
            None => existing,
        };
        let url = self.map_url(&item.url);
        let mut request = self.client.get(&url);
        if resume_from > 0 {
            request = request.header(RANGE, format!("bytes={resume_from}-"));
        }
        let response = request.send().await?;
        if !response.status().is_success()
            && response.status() != reqwest::StatusCode::PARTIAL_CONTENT
        {
            bail!("服务器返回 {}：{url}", response.status().as_u16());
        }
        let resumed = resume_from > 0 && response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
        let mut file = if resumed {
            tokio::fs::OpenOptions::new()
                .append(true)
                .open(temp)
                .await?
        } else {
            tokio::fs::File::create(temp).await?
        };
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        Ok(())
    }

    /// Download many files with bounded concurrency, reporting each completion.
    pub async fn download_all(&self, items: Vec<DownloadItem>, progress: &Progress) -> Result<()> {
        let pending: Vec<DownloadItem> = items
            .into_iter()
            .filter(|item| !item.is_satisfied())
            .collect();
        if pending.is_empty() {
            return Ok(());
        }
        if self.offline {
            let missing = pending
                .iter()
                .take(5)
                .map(|item| item.dest.display().to_string())
                .collect::<Vec<_>>()
                .join("、");
            bail!("严格离线模式缺少 {} 个文件，例如：{missing}", pending.len());
        }
        let semaphore = Arc::new(Semaphore::new(self.concurrency));
        let mut set = tokio::task::JoinSet::new();
        for item in pending {
            if progress.cancelled() {
                break;
            }
            let permit = semaphore.clone().acquire_owned().await?;
            let client = self.client.clone();
            let progress = progress.clone();
            let mirror = self.mirror.clone();
            set.spawn(async move {
                let _permit = permit;
                let downloader = Downloader {
                    client,
                    concurrency: 1,
                    offline: false,
                    mirror,
                };
                let label = if item.label.is_empty() {
                    item.dest
                        .file_name()
                        .map(|v| v.to_string_lossy().to_string())
                        .unwrap_or_default()
                } else {
                    item.label.clone()
                };
                let result = downloader.download(&item, &progress).await;
                if result.is_ok() {
                    progress.step("download", label).await;
                }
                result
            });
        }
        let mut failure: Option<anyhow::Error> = None;
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    if failure.is_none() {
                        failure = Some(error);
                    }
                }
                Err(error) => {
                    if failure.is_none() {
                        failure = Some(anyhow!(error));
                    }
                }
            }
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Download one file and report progress at file granularity.
    pub async fn download_one(
        &self,
        item: DownloadItem,
        progress: &Progress,
        phase: &str,
    ) -> Result<()> {
        if item.is_satisfied() {
            return Ok(());
        }
        self.download(&item, progress).await?;
        progress
            .step(
                phase,
                if item.label.is_empty() {
                    item.dest.display().to_string()
                } else {
                    item.label.clone()
                },
            )
            .await;
        Ok(())
    }
}

/// Partial downloads use a deterministic name so an interrupted run can be
/// resumed after the launcher restarts. Correctness does not depend on it:
/// the finished file is verified against upstream hashes before it is renamed
/// into place, and a mismatch deletes it and retries.
fn temp_path(dest: &Path) -> PathBuf {
    let mut name = dest
        .file_name()
        .map(|v| v.to_string_lossy().to_string())
        .unwrap_or_else(|| "download".to_string());
    name.push_str(".part");
    dest.with_file_name(name)
}

/// Remove partial files left by older launcher versions, which suffixed the
/// process id and therefore could never be resumed.
async fn clean_legacy_partials(dest: &Path) {
    let (Some(parent), Some(file_name)) = (dest.parent(), dest.file_name()) else {
        return;
    };
    let prefix = format!("{}.", file_name.to_string_lossy());
    let Ok(mut entries) = tokio::fs::read_dir(parent).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(&prefix) && name.ends_with(".part") {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
}

pub async fn verify_async(path: &Path, item: &DownloadItem) -> Result<bool> {
    let metadata = match tokio::fs::metadata(path).await {
        Ok(meta) => meta,
        Err(_) => return Ok(false),
    };
    if let Some(size) = item.size {
        if metadata.len() != size {
            return Ok(false);
        }
    }
    if item.sha1.is_none() && item.sha256.is_none() {
        return Ok(true);
    }
    let path = path.to_path_buf();
    let sha1 = item.sha1.clone();
    let sha256 = item.sha256.clone();
    tokio::task::spawn_blocking(move || verify_sync(&path, sha1.as_deref(), sha256.as_deref()))
        .await?
}

pub fn resource_url(hash: &str) -> String {
    format!("{RESOURCES_URL}/{}/{}", &hash[..2], hash)
}

pub fn download_item_from(info: &DownloadInfo, dest: PathBuf) -> DownloadItem {
    DownloadItem {
        url: info.url.clone(),
        dest,
        sha1: info.sha1.clone(),
        sha256: info.sha256.clone(),
        size: info.size,
        label: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsatisfied_without_metadata() {
        let item = DownloadItem::new("https://example.invalid/a", PathBuf::from("/tmp/nope-a"));
        assert!(!item.is_satisfied());
    }

    #[test]
    fn mirror_rewrite_keeps_path() {
        let downloader = Downloader::new(2, false, Some("https://bmclapi2.bangbang93.com".into()))
            .expect("client");
        // Version metadata keeps its path.
        assert_eq!(
            downloader.map_url("https://piston-meta.mojang.com/mc/game/version_manifest_v2.json"),
            "https://bmclapi2.bangbang93.com/mc/game/version_manifest_v2.json"
        );
        // Libraries move under /maven.
        assert_eq!(
            downloader.map_url("https://libraries.minecraft.net/org/ow2/asm/asm/9.6/asm-9.6.jar"),
            "https://bmclapi2.bangbang93.com/maven/org/ow2/asm/asm/9.6/asm-9.6.jar"
        );
        // Loader Maven uses the same prefix, with the /releases segment dropped.
        assert_eq!(
            downloader.map_url(
                "https://maven.neoforged.net/releases/net/neoforged/neoforge/21.1.255/neoforge-21.1.255-installer.jar"
            ),
            "https://bmclapi2.bangbang93.com/maven/net/neoforged/neoforge/21.1.255/neoforge-21.1.255-installer.jar"
        );
        // Asset objects move under /assets.
        assert_eq!(
            downloader.map_url("https://resources.download.minecraft.net/ab/abcdef"),
            "https://bmclapi2.bangbang93.com/assets/ab/abcdef"
        );
        // Hosts without a verified mapping are left alone, including piston-data.
        assert_eq!(
            downloader.map_url("https://meta.fabricmc.net/v2/versions/game"),
            "https://meta.fabricmc.net/v2/versions/game"
        );
        assert_eq!(
            downloader.map_url("https://piston-data.mojang.com/v1/objects/aa/bb/client.jar"),
            "https://piston-data.mojang.com/v1/objects/aa/bb/client.jar"
        );
        // A URL already on the mirror is never rewritten twice.
        assert_eq!(
            downloader.map_url("https://bmclapi2.bangbang93.com/mc/game/x.json"),
            "https://bmclapi2.bangbang93.com/mc/game/x.json"
        );
    }

    #[test]
    fn no_mirror_leaves_urls_untouched() {
        let downloader = Downloader::new(2, false, None).expect("client");
        assert_eq!(
            downloader.map_url("https://libraries.minecraft.net/a/b.jar"),
            "https://libraries.minecraft.net/a/b.jar"
        );
    }

    #[test]
    fn resource_url_uses_hash_prefix() {
        assert_eq!(
            resource_url("abcdef1234567890"),
            format!("{RESOURCES_URL}/ab/abcdef1234567890")
        );
    }

    #[test]
    fn partial_name_is_stable_across_runs() {
        // A deterministic partial name is what makes resume-after-restart work.
        let dest = PathBuf::from("/tmp/libraries/foo.jar");
        assert_eq!(
            temp_path(&dest),
            PathBuf::from("/tmp/libraries/foo.jar.part")
        );
        assert_eq!(temp_path(&dest), temp_path(&dest));
    }
}
