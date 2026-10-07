use super::{
    clean_legacy_partials, display_name, item_key, temp_path, verify_async, DownloadItem, Progress,
};
use anyhow::{anyhow, bail, Context, Result};
use futures_util::{stream, StreamExt};
use reqwest::header::{
    ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE, ETAG, IF_RANGE,
    LAST_MODIFIED, RANGE,
};
use reqwest::{Response, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const SEGMENT_THRESHOLD: u64 = 8 * 1024 * 1024;
const SEGMENTS: u64 = 4;
const ATTEMPTS: u32 = 3;

/// Clones share the connection budget, including metadata and individual segments.
#[derive(Clone)]
pub struct Downloader {
    client: reqwest::Client,
    concurrency: usize,
    connections: Arc<Semaphore>,
    offline: bool,
    mirror: Option<String>,
}

impl Downloader {
    pub fn new(concurrency: usize, offline: bool, mirror: Option<String>) -> Result<Self> {
        let concurrency = concurrency.clamp(1, 16);
        let mirror = crate::mirror::normalize_base(mirror.as_deref())?;
        let client = reqwest::Client::builder()
            .user_agent(format!(
                "{}/{}",
                crate::types::LAUNCHER_NAME,
                crate::types::LAUNCHER_VERSION
            ))
            .connect_timeout(Duration::from_secs(20))
            .read_timeout(Duration::from_secs(90))
            .pool_idle_timeout(Duration::from_secs(30))
            // Byte ranges and hashes describe the identity representation.
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .build()?;
        Ok(Self {
            client,
            concurrency,
            connections: Arc::new(Semaphore::new(concurrency)),
            offline,
            mirror,
        })
    }

    pub fn offline(&self) -> bool {
        self.offline
    }
    pub fn map_url(&self, url: &str) -> String {
        crate::mirror::map_url(self.mirror.as_deref(), url)
    }

    fn sources(&self, url: &str) -> Vec<String> {
        let mapped = self.map_url(url);
        if mapped == url {
            vec![mapped]
        } else {
            vec![mapped, url.to_owned()]
        }
    }

    pub async fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>> {
        Ok(self.fetch_bytes_with_source(url).await?.0)
    }

    /// The flag records whether the successful response came from the mirror.
    pub async fn fetch_bytes_with_source(&self, url: &str) -> Result<(Vec<u8>, bool)> {
        if self.offline {
            bail!("严格离线模式下不会访问网络：{url}");
        }
        self.fetch_bytes_candidates(url, &self.sources(url)).await
    }

    pub(super) async fn fetch_bytes_candidates(
        &self,
        url: &str,
        sources: &[String],
    ) -> Result<(Vec<u8>, bool)> {
        let mut last_error = anyhow!("请求失败：{url}");
        for source in sources {
            for attempt in 0..ATTEMPTS {
                let result = async {
                    let _permit = self.connections.acquire().await?;
                    let response = self
                        .client
                        .get(source)
                        .header(ACCEPT_ENCODING, "identity")
                        .send()
                        .await?
                        .error_for_status()?;
                    validate_encoding(&response)?;
                    if response.status() != StatusCode::OK {
                        bail!("元数据响应状态异常：{}", response.status());
                    }
                    let expected = content_length(&response)?;
                    let bytes = response.bytes().await?;
                    if expected.is_some_and(|length| length != bytes.len() as u64) {
                        bail!("元数据长度不匹配");
                    }
                    Ok::<_, anyhow::Error>(bytes.to_vec())
                }
                .await;
                match result {
                    Ok(bytes) => return Ok((bytes, source != url)),
                    Err(error) => {
                        let retry = retryable(&error);
                        last_error = error;
                        // Move to the origin immediately when the mirror fails.
                        if source != url || !retry || attempt + 1 == ATTEMPTS {
                            break;
                        }
                        tokio::time::sleep(backoff(attempt)).await;
                    }
                }
            }
        }
        Err(last_error).with_context(|| format!("下载 {url} 失败"))
    }

    pub async fn fetch_text(&self, url: &str) -> Result<String> {
        Ok(String::from_utf8_lossy(&self.fetch_bytes(url).await?).into_owned())
    }

    pub async fn fetch_json<T: for<'a> serde::Deserialize<'a>>(&self, url: &str) -> Result<T> {
        if self.offline {
            bail!("严格离线模式下不会访问网络：{url}");
        }
        let mut last_error = anyhow!("解析 {url} 返回的 JSON 失败");
        // A mirror's HTTP 200 error page also triggers origin fallback.
        for source in self.sources(url) {
            match self
                .fetch_bytes_candidates(url, std::slice::from_ref(&source))
                .await
            {
                Ok((bytes, _)) => match serde_json::from_slice(&bytes) {
                    Ok(value) => return Ok(value),
                    Err(error) => last_error = error.into(),
                },
                Err(error) => last_error = error,
            }
        }
        Err(last_error).with_context(|| format!("解析 {url} 返回的 JSON 失败"))
    }

    pub async fn download(&self, item: &DownloadItem, progress: &Progress) -> Result<()> {
        self.download_candidates(item, progress, &self.sources(&item.url))
            .await
    }

    pub(super) async fn download_candidates(
        &self,
        item: &DownloadItem,
        progress: &Progress,
        sources: &[String],
    ) -> Result<()> {
        progress.check_cancelled()?;
        // Shared paths (libraries/assets) can be requested by independent installers.
        let lock = destination_lock(&item.dest)?;
        let _guard = cancellable(progress, lock.lock()).await?;
        progress.check_cancelled()?;
        if item.is_satisfied() {
            // The file became valid between planning and its turn. It still counts
            // toward the batch, or the byte bar could never reach its total.
            progress.settle_satisfied(item);
            return Ok(());
        }
        let key = item_key(item);
        progress.begin_file(item);
        let result = self.transfer(item, &key, progress, sources).await;
        progress.finish_file(&key, result.is_ok()).await;
        result
    }

    /// Transfer one file into place, retrying sources and attempts.
    async fn transfer(
        &self,
        item: &DownloadItem,
        key: &str,
        progress: &Progress,
        sources: &[String],
    ) -> Result<()> {
        if self.offline {
            bail!(
                "严格离线模式缺少文件，且不允许下载：{}",
                item.dest.display()
            );
        }
        if let Some(parent) = item.dest.parent().filter(|p| !p.as_os_str().is_empty()) {
            tokio::fs::create_dir_all(parent).await?;
        }
        clean_legacy_partials(&item.dest).await;
        let temp = temp_path(&item.dest);
        let mut last_error = anyhow!("下载失败：{}", item.url);
        for source in sources {
            let mut use_segments = (item.sha1.is_some() || item.sha256.is_some())
                && item.size.is_some_and(|size| size >= SEGMENT_THRESHOLD)
                && file_len(&temp).await? == 0;
            for attempt in 0..ATTEMPTS {
                progress.check_cancelled()?;
                let result = async {
                    if use_segments {
                        if !self.segmented(item, source, &temp, key, progress).await? {
                            use_segments = false;
                            self.single(item, source, &temp, key, progress).await?;
                        }
                    } else {
                        self.single(item, source, &temp, key, progress).await?;
                    }
                    if !cancellable(progress, verify_async(&temp, item)).await?? {
                        // Corrupt complete data cannot be resumed, unlike transport failures.
                        truncate(&temp).await?;
                        clear_segments(item).await?;
                        // The discarded bytes are no longer on disk, so the visible
                        // progress for this file must not claim them.
                        progress.credit_file(key, 0);
                        bail!("校验失败：{}", item.dest.display());
                    }
                    progress.check_cancelled()?;
                    tokio::fs::rename(&temp, &item.dest).await?;
                    let _ = remove_partial_meta(&temp).await;
                    // Cleanup must not turn an already-published success into a retry.
                    let _ = clear_segments(item).await;
                    Ok::<_, anyhow::Error>(())
                }
                .await;
                match result {
                    Ok(()) => return Ok(()),
                    Err(error) => {
                        progress.check_cancelled()?;
                        let retry = retryable(&error);
                        last_error = error;
                        if source != &item.url || !retry || attempt + 1 == ATTEMPTS {
                            break;
                        }
                        cancellable(progress, tokio::time::sleep(backoff(attempt))).await?;
                    }
                }
            }
        }
        Err(last_error).with_context(|| format!("下载 {}", item.dest.display()))
    }

    async fn request(
        &self,
        url: &str,
        range: Option<String>,
        if_range: Option<&str>,
        progress: &Progress,
    ) -> Result<(Response, OwnedSemaphorePermit)> {
        let permit = cancellable(progress, self.connections.clone().acquire_owned()).await??;
        let mut request = self.client.get(url).header(ACCEPT_ENCODING, "identity");
        if let Some(range) = range {
            request = request.header(RANGE, range);
        }
        if let Some(if_range) = if_range {
            request = request.header(IF_RANGE, if_range);
        }
        let response = cancellable(progress, request.send()).await??;
        validate_encoding(&response)?;
        Ok((response, permit))
    }

    async fn single(
        &self,
        item: &DownloadItem,
        url: &str,
        temp: &Path,
        key: &str,
        progress: &Progress,
    ) -> Result<()> {
        let hash_verified = item.sha1.is_some() || item.sha256.is_some();
        loop {
            let mut existing = file_len(temp).await?;
            let mut saved = None;
            if !hash_verified && existing > 0 {
                saved = load_partial_meta(temp).await?;
                if saved.as_ref().is_none_or(|meta| meta.source != url) {
                    // A size-only partial has no identity unless its validator and
                    // source are retained. Preserve the file for diagnostics, but
                    // never append another representation to it.
                    truncate(temp).await?;
                    remove_partial_meta(temp).await?;
                    existing = 0;
                    saved = None;
                }
            }
            let offset = if item.size.is_some_and(|size| existing > size) {
                0
            } else {
                existing
            };
            // The kept bytes are on disk, so they are progress the user already has.
            progress.credit_file(key, offset);
            let range = (offset > 0).then(|| format!("bytes={offset}-"));
            let if_range = saved.as_ref().map(|meta| meta.validator.as_str());
            let (response, _permit) = self.request(url, range, if_range, progress).await?;
            let validator = response_validator(&response);
            if response.status() == StatusCode::RANGE_NOT_SATISFIABLE {
                let total = unsatisfied_total(&response)?;
                if offset == 0 || total != offset || item.size.is_some_and(|size| size != total) {
                    bail!("416 响应与已有文件长度不匹配");
                }
                if !hash_verified {
                    let matches = saved
                        .as_ref()
                        .zip(validator.as_ref())
                        .is_some_and(|(saved, current)| saved.validator == *current);
                    if !matches {
                        truncate(temp).await?;
                        remove_partial_meta(temp).await?;
                        continue;
                    }
                }
                // The caller verifies size and all hashes before publishing.
                return Ok(());
            }
            let response = response.error_for_status()?;
            let (append, expected) = match response.status() {
                StatusCode::PARTIAL_CONTENT => {
                    let (start, end, total) = content_range(&response)?;
                    if offset == 0
                        || start != offset
                        || end + 1 != total
                        || item.size.is_some_and(|size| size != total)
                    {
                        bail!("Content-Range 与续传请求不匹配");
                    }
                    if !hash_verified {
                        let saved = saved.as_ref().context("无哈希续传缺少 partial 验证器")?;
                        if validator.as_ref() != Some(&saved.validator) {
                            truncate(temp).await?;
                            remove_partial_meta(temp).await?;
                            continue;
                        }
                    }
                    let length = end - start + 1;
                    check_length(&response, length)?;
                    (true, Some(length))
                }
                StatusCode::OK => {
                    if response.headers().contains_key(CONTENT_RANGE) {
                        bail!("200 响应含有异常 Content-Range");
                    }
                    let length = content_length(&response)?;
                    if let (Some(actual), Some(expected)) = (length, item.size) {
                        if actual != expected {
                            bail!("Content-Length 与预期大小不匹配");
                        }
                    }
                    (false, item.size.or(length))
                }
                status => bail!("不支持的下载响应：{status}"),
            };
            if !hash_verified {
                if let Some(value) = validator {
                    save_partial_meta(temp, url, &value).await?;
                } else {
                    // A complete response without a validator cannot be safely
                    // resumed on a later process, so clear stale identity data.
                    remove_partial_meta(temp).await?;
                }
            }
            return write_response(response, temp, append, expected, key, progress).await;
        }
    }

    async fn segmented(
        &self,
        item: &DownloadItem,
        url: &str,
        temp: &Path,
        key: &str,
        progress: &Progress,
    ) -> Result<bool> {
        let total = item.size.context("分段下载缺少大小")?;
        // Each attempt rewrites every segment from its own offset, so the file's
        // byte counter restarts here and each worker credits what it already has.
        progress.credit_file(key, 0);
        // Probe rather than trusting Accept-Ranges; drop ignored bodies before fallback.
        {
            let (response, _permit) = self
                .request(url, Some("bytes=0-0".into()), None, progress)
                .await?;
            if matches!(
                response.status(),
                StatusCode::OK | StatusCode::RANGE_NOT_SATISFIABLE
            ) {
                return Ok(false);
            }
            let response = response.error_for_status()?;
            if response.status() != StatusCode::PARTIAL_CONTENT
                || content_range(&response)? != (0, 0, total)
            {
                bail!("服务器返回错误的分段探测 Content-Range");
            }
            check_length(&response, 1)?;
            let bytes = cancellable(progress, response.bytes()).await??;
            if bytes.len() != 1 {
                bail!("分段探测长度不匹配");
            }
        }
        let mut work = stream::iter(0..SEGMENTS)
            .map(|index| async move {
                let start = total * index / SEGMENTS;
                let end = total * (index + 1) / SEGMENTS - 1;
                let path = segment_path(item, index);
                let mut existing = file_len(&path).await?;
                let length = end - start + 1;
                if existing > length {
                    truncate(&path).await?;
                    existing = 0;
                }
                // Bytes kept from an earlier run are already downloaded, including
                // a segment that is already complete.
                progress.add_bytes(key, existing);
                if existing == length {
                    return Ok(true);
                }
                let offset = start + existing;
                let (response, _permit) = self
                    .request(url, Some(format!("bytes={offset}-{end}")), None, progress)
                    .await?;
                if matches!(
                    response.status(),
                    StatusCode::OK | StatusCode::RANGE_NOT_SATISFIABLE
                ) {
                    return Ok(false);
                }
                let response = response.error_for_status()?;
                if response.status() != StatusCode::PARTIAL_CONTENT
                    || content_range(&response)? != (offset, end, total)
                {
                    bail!("Content-Range 与分段请求不匹配");
                }
                check_length(&response, end - offset + 1)?;
                write_response(
                    response,
                    &path,
                    existing > 0,
                    Some(end - offset + 1),
                    key,
                    progress,
                )
                .await?;
                Ok::<_, anyhow::Error>(true)
            })
            .buffer_unordered(SEGMENTS as usize);
        let mut failure = None;
        let mut supported = true;
        // Drain every segment so all outstanding writes are flushed on cancellation.
        while let Some(result) = work.next().await {
            match result {
                Ok(value) => supported &= value,
                Err(error) => {
                    if failure.is_none() {
                        failure = Some(error);
                    }
                }
            }
        }
        progress.check_cancelled()?;
        if !supported {
            return Ok(false);
        }
        if let Some(error) = failure {
            return Err(error);
        }
        let mut output = tokio::fs::File::create(temp).await?;
        let result = async {
            let mut buffer = vec![0; 64 * 1024];
            for index in 0..SEGMENTS {
                let mut input = tokio::fs::File::open(segment_path(item, index)).await?;
                loop {
                    progress.check_cancelled()?;
                    let read = input.read(&mut buffer).await?;
                    if read == 0 {
                        break;
                    }
                    output.write_all(&buffer[..read]).await?;
                }
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        output.flush().await?;
        result?;
        Ok(true)
    }

    pub async fn download_all(&self, items: Vec<DownloadItem>, progress: &Progress) -> Result<()> {
        progress.check_cancelled()?;
        // The byte bar's denominator covers every planned file, including the ones
        // that turn out to be on disk already.
        progress.begin_batch(&items);
        let mut pending = Vec::new();
        for item in items {
            progress.check_cancelled()?;
            if item.is_satisfied() {
                progress.settle_satisfied(&item);
                progress.step("download", display_name(&item)).await;
            } else {
                pending.push(item);
            }
        }
        if pending.is_empty() {
            return Ok(());
        }
        if self.offline {
            bail!(
                "严格离线模式缺少 {} 个文件，例如：{}",
                pending.len(),
                pending[0].dest.display()
            );
        }
        let mut work = stream::iter(pending)
            .map(|item| async move {
                self.download(&item, progress).await?;
                cancellable(progress, progress.step("download", display_name(&item))).await?;
                Ok::<_, anyhow::Error>(())
            })
            .buffer_unordered(self.concurrency);
        let mut failure = None;
        while let Some(result) = work.next().await {
            if let Err(error) = result {
                if failure.is_none() {
                    failure = Some(error);
                }
            }
        }
        progress.check_cancelled()?;
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub async fn download_one(
        &self,
        item: DownloadItem,
        progress: &Progress,
        phase: &str,
    ) -> Result<()> {
        progress.check_cancelled()?;
        progress.begin_batch(std::slice::from_ref(&item));
        if item.is_satisfied() {
            progress.settle_satisfied(&item);
            progress.step(phase, display_name(&item)).await;
            return Ok(());
        }
        self.download(&item, progress).await?;
        cancellable(progress, progress.step(phase, display_name(&item))).await?;
        Ok(())
    }
}

// Weak entries avoid retaining every asset path forever. Independent Downloader
// instances also share these locks when installers overlap in the same process.
fn destination_lock(path: &Path) -> Result<Arc<tokio::sync::Mutex<()>>> {
    type Locks = Mutex<HashMap<PathBuf, Weak<tokio::sync::Mutex<()>>>>;
    static LOCKS: OnceLock<Locks> = OnceLock::new();
    let absolute = std::path::absolute(path)?;
    let mut locks = LOCKS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(lock) = locks.get(&absolute).and_then(Weak::upgrade) {
        return Ok(lock);
    }
    locks.retain(|_, lock| lock.strong_count() > 0);
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    locks.insert(absolute, Arc::downgrade(&lock));
    Ok(lock)
}

async fn cancellable<T>(progress: &Progress, future: impl Future<Output = T>) -> Result<T> {
    tokio::select! {
        biased;
        _ = async { loop { if progress.cancelled() { break; } tokio::time::sleep(Duration::from_millis(20)).await; } } => bail!("任务已取消"),
        result = future => Ok(result),
    }
}
fn backoff(attempt: u32) -> Duration {
    Duration::from_millis(150 * (attempt + 1) as u64)
}
fn retryable(error: &anyhow::Error) -> bool {
    !error
        .downcast_ref::<reqwest::Error>()
        .and_then(|e| e.status())
        .is_some_and(|s| {
            s.is_client_error()
                && s != StatusCode::REQUEST_TIMEOUT
                && s != StatusCode::TOO_MANY_REQUESTS
        })
}
async fn file_len(path: &Path) -> Result<u64> {
    match tokio::fs::metadata(path).await {
        Ok(metadata) if metadata.is_file() => Ok(metadata.len()),
        Ok(_) => bail!("下载临时路径不是文件：{}", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error.into()),
    }
}
async fn truncate(path: &Path) -> Result<()> {
    tokio::fs::File::create(path).await?.flush().await?;
    Ok(())
}
pub(super) fn segment_path(item: &DownloadItem, index: u64) -> PathBuf {
    // Stable boundaries survive concurrency changes; identity separates manifests.
    let identity = format!(
        "{}\n{:?}\n{:?}\n{:?}",
        item.url, item.size, item.sha1, item.sha256
    );
    let digest = hex::encode(Sha256::digest(identity.as_bytes()));
    let mut name = temp_path(&item.dest).file_name().unwrap().to_os_string();
    name.push(format!(".{digest}.segment-{index}"));
    item.dest.with_file_name(name)
}
async fn clear_segments(item: &DownloadItem) -> Result<()> {
    if item.size.is_none_or(|size| size < SEGMENT_THRESHOLD) {
        return Ok(());
    }
    for index in 0..SEGMENTS {
        match tokio::fs::remove_file(segment_path(item, index)).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
struct PartialMeta {
    source: String,
    validator: String,
}

fn partial_meta_path(temp: &Path) -> PathBuf {
    let mut name = temp.file_name().unwrap_or_default().to_os_string();
    name.push(".meta");
    temp.with_file_name(name)
}

async fn load_partial_meta(temp: &Path) -> Result<Option<PartialMeta>> {
    let path = partial_meta_path(temp);
    let bytes = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let meta = serde_json::from_slice(&bytes).ok();
    Ok(meta.filter(|meta: &PartialMeta| !meta.source.is_empty() && !meta.validator.is_empty()))
}

async fn save_partial_meta(temp: &Path, source: &str, validator: &str) -> Result<()> {
    let path = partial_meta_path(temp);
    let mut temporary = path.clone();
    temporary.set_extension("meta.tmp");
    let bytes = serde_json::to_vec(&PartialMeta {
        source: source.to_owned(),
        validator: validator.to_owned(),
    })?;
    tokio::fs::write(&temporary, bytes).await?;
    tokio::fs::rename(temporary, path).await?;
    Ok(())
}

async fn remove_partial_meta(temp: &Path) -> Result<()> {
    match tokio::fs::remove_file(partial_meta_path(temp)).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn response_validator(response: &Response) -> Option<String> {
    let etag = response
        .headers()
        .get(ETAG)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim_start().starts_with("W/"))
        .map(str::to_owned);
    etag.or_else(|| {
        response
            .headers()
            .get(LAST_MODIFIED)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    })
}

fn validate_encoding(response: &Response) -> Result<()> {
    if let Some(encoding) = response.headers().get(CONTENT_ENCODING) {
        if !encoding.to_str()?.eq_ignore_ascii_case("identity") {
            bail!("服务器未返回 identity 编码");
        }
    }
    Ok(())
}
fn number(value: &str) -> Result<u64> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        bail!("无效的 HTTP 长度或范围");
    }
    Ok(value.parse()?)
}
fn content_length(response: &Response) -> Result<Option<u64>> {
    response
        .headers()
        .get(CONTENT_LENGTH)
        .map(|value| number(value.to_str()?))
        .transpose()
}
fn check_length(response: &Response, expected: u64) -> Result<()> {
    if content_length(response)?.is_some_and(|length| length != expected) {
        bail!("Content-Length 与 Content-Range 不匹配");
    }
    Ok(())
}
fn content_range(response: &Response) -> Result<(u64, u64, u64)> {
    let value = response
        .headers()
        .get(CONTENT_RANGE)
        .context("缺少 Content-Range")?
        .to_str()?;
    let value = value
        .strip_prefix("bytes ")
        .context("无效 Content-Range 单位")?;
    let (range, total) = value.split_once('/').context("无效 Content-Range")?;
    let (start, end) = range.split_once('-').context("无效 Content-Range")?;
    let (start, end, total) = (number(start)?, number(end)?, number(total)?);
    if start > end || end >= total {
        bail!("无效 Content-Range 边界");
    }
    Ok((start, end, total))
}
fn unsatisfied_total(response: &Response) -> Result<u64> {
    let value = response
        .headers()
        .get(CONTENT_RANGE)
        .context("416 缺少 Content-Range")?
        .to_str()?;
    number(
        value
            .strip_prefix("bytes */")
            .context("无效的 416 Content-Range")?,
    )
}
async fn write_response(
    response: Response,
    path: &Path,
    append: bool,
    expected: Option<u64>,
    key: &str,
    progress: &Progress,
) -> Result<()> {
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(path)
        .await?;
    let mut body = response.bytes_stream();
    let result = async {
        let mut received = 0u64;
        while let Some(chunk) = cancellable(progress, body.next()).await? {
            let chunk = chunk?;
            let length = chunk.len() as u64;
            if expected.is_some_and(|expected| length > expected.saturating_sub(received)) {
                bail!("响应体超过声明长度");
            }
            file.write_all(&chunk).await?;
            received += length;
            // Flushed bytes are real progress; the counter drives the UI panel.
            progress.add_bytes(key, length);
        }
        if expected.is_some_and(|expected| expected != received) {
            bail!("响应体短于声明长度");
        }
        Ok::<_, anyhow::Error>(())
    }
    .await;
    // Flush even after the next network read fails or cancellation interrupts it.
    file.flush().await?;
    result
}
