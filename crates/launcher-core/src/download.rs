use crate::types::{ActiveDownload, DownloadInfo, InstallTask, RESOURCES_URL};
use anyhow::{bail, Result};
use sha1::{Digest as _, Sha1};
use sha2::Sha256;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

mod engine;
pub use engine::Downloader;

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

/// Mid-transfer updates are coalesced to this interval. Stage, step and terminal
/// events are never throttled, so every finished file still reports.
const REPORT_INTERVAL: Duration = Duration::from_millis(150);

/// A file that is transferring right now.
#[derive(Default)]
struct ActiveFile {
    name: String,
    size: u64,
    received: u64,
}

/// The phase the last stage/step event belonged to. Byte updates reuse it so the
/// headline does not jump between a file name and a phase name.
#[derive(Default, Clone)]
struct Context {
    phase: String,
    message: String,
}

/// Byte-level accounting for the batch of files one phase transfers.
///
/// Settled files are already valid or finished; active files are in flight, so
/// `done` is derived instead of repaired with signed deltas when a transfer
/// restarts. Segment workers share one entry per destination.
#[derive(Default)]
struct Tracker {
    total_bytes: AtomicU64,
    settled_bytes: AtomicU64,
    active: Mutex<BTreeMap<String, ActiveFile>>,
    context: Mutex<Context>,
    last_report: Mutex<Option<Instant>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

impl Tracker {
    /// Start a fresh batch: known sizes are the denominator, and nothing of it
    /// is on disk yet (satisfied files are settled as the caller checks them).
    fn begin_batch(&self, items: &[DownloadItem]) {
        let total: u64 = items.iter().filter_map(|item| item.size).sum();
        self.total_bytes.store(total, Ordering::Relaxed);
        self.settled_bytes.store(0, Ordering::Relaxed);
        self.active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        *lock(&self.last_report) = None;
    }
    /// Count one known size that is not part of the batch total yet.
    fn plan(&self, item: &DownloadItem) {
        if let Some(size) = item.size {
            self.total_bytes.fetch_add(size, Ordering::Relaxed);
        }
    }
    /// A file whose destination already validates counts as finished.
    fn settle(&self, size: Option<u64>) {
        if let Some(size) = size {
            self.settled_bytes.fetch_add(size, Ordering::Relaxed);
        }
    }
    fn begin_file(&self, key: &str, name: String, size: Option<u64>) {
        lock(&self.active).insert(
            key.to_owned(),
            ActiveFile {
                name,
                size: size.unwrap_or(0),
                received: 0,
            },
        );
    }
    /// Absolute progress for one file, used where the engine learns how much of
    /// a partial file it keeps.
    fn credit_file(&self, key: &str, bytes: u64) {
        if let Some(file) = lock(&self.active).get_mut(key) {
            file.received = bytes;
        }
    }
    fn add_bytes(&self, key: &str, bytes: u64) {
        if let Some(file) = lock(&self.active).get_mut(key) {
            file.received += bytes;
        }
    }
    /// Stop listing a file. A finished file counts its full size even if some
    /// bytes were never observed, so a completed batch reaches its total.
    fn finish_file(&self, key: &str, complete: bool) -> bool {
        let mut active = lock(&self.active);
        if let Some(file) = active.remove(key) {
            let counted = if complete {
                file.received.max(file.size)
            } else {
                file.received
            };
            self.settled_bytes.fetch_add(counted, Ordering::Relaxed);
        }
        active.is_empty()
    }
    /// Forget every in-flight file: used when a phase ends, so a terminal event
    /// never advertises transfers that have already stopped.
    fn clear(&self) {
        self.total_bytes.store(0, Ordering::Relaxed);
        self.settled_bytes.store(0, Ordering::Relaxed);
        lock(&self.active).clear();
        *lock(&self.last_report) = None;
    }
    fn clear_active(&self) {
        let mut active = lock(&self.active);
        for file in active.values() {
            self.settled_bytes
                .fetch_add(file.received, Ordering::Relaxed);
        }
        active.clear();
    }
    fn set_context(&self, phase: &str, message: &str) {
        let mut context = lock(&self.context);
        context.phase = phase.to_owned();
        context.message = message.to_owned();
    }
    fn context(&self) -> Context {
        lock(&self.context).clone()
    }
    fn snapshot(&self) -> (u64, u64, Vec<ActiveDownload>) {
        let active = lock(&self.active);
        let mut files: Vec<ActiveDownload> = active
            .values()
            .map(|file| ActiveDownload {
                name: file.name.clone(),
                downloaded: file.received,
                total: file.size,
            })
            .collect();
        // Largest first: the files worth watching are the ones that take time.
        files.sort_by(|a, b| b.total.cmp(&a.total).then_with(|| a.name.cmp(&b.name)));
        let downloaded = self.settled_bytes.load(Ordering::Relaxed)
            + active.values().map(|file| file.received).sum::<u64>();
        (downloaded, self.total_bytes.load(Ordering::Relaxed), files)
    }
    fn take_report_slot(&self) -> bool {
        let mut last = lock(&self.last_report);
        let now = Instant::now();
        if last.is_some_and(|last| now.duration_since(last) < REPORT_INTERVAL) {
            return false;
        }
        *last = Some(now);
        true
    }
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
    tracker: Arc<Tracker>,
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
            tracker: Arc::new(Tracker::default()),
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
    /// Counters describe the phase that follows; byte totals belong to the batch
    /// the downloader is about to announce, so they start empty here.
    pub fn set_total(&self, total: u64) {
        self.done.store(0, Ordering::Relaxed);
        self.total.store(total, Ordering::Relaxed);
        self.tracker.clear();
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
        self.tracker.clear_active();
        self.send("done", message.into(), true, None).await;
    }
    pub async fn fail(&self, message: impl Into<String>) {
        self.tracker.clear_active();
        let message = message.into();
        self.send("error", message.clone(), true, Some(message))
            .await;
    }

    /* ------------------------------------------------------- download batches */

    /// Announce the files a batch will transfer, so the byte bar has a denominator
    /// before the first response arrives.
    pub fn begin_batch(&self, items: &[DownloadItem]) {
        self.tracker.begin_batch(items);
        self.report();
    }
    /// A batch that was never announced still reports bytes: the first file to
    /// start claims the batch.
    fn ensure_batch(&self, item: &DownloadItem) {
        let (downloaded, total, active) = self.tracker.snapshot();
        if downloaded == 0 && total == 0 && active.is_empty() {
            self.tracker.plan(item);
        }
    }
    /// A destination that already validates counts toward the byte total.
    pub fn settle_satisfied(&self, item: &DownloadItem) {
        self.tracker.settle(item.size);
    }
    pub fn begin_file(&self, item: &DownloadItem) {
        self.ensure_batch(item);
        self.tracker
            .begin_file(&item_key(item), display_name(item), item.size);
        self.report();
    }
    /// Bytes of a partial file that this run keeps and will not fetch again.
    pub fn credit_file(&self, key: &str, bytes: u64) {
        self.tracker.credit_file(key, bytes);
        self.report();
    }
    pub fn add_bytes(&self, key: &str, bytes: u64) {
        if bytes > 0 {
            self.tracker.add_bytes(key, bytes);
            self.report();
        }
    }
    /// Stop listing a file. The last file of a batch reports immediately so the
    /// bar lands on its total instead of the last throttled sample.
    pub async fn finish_file(&self, key: &str, complete: bool) {
        if self.tracker.finish_file(key, complete) {
            self.send_context().await;
        } else {
            self.report();
        }
    }

    /// One coalesced update while bytes are moving. Dropped when the channel is
    /// full: a slow reader must never stall a transfer.
    fn report(&self) {
        let Some(tx) = &self.tx else { return };
        if !self.tracker.take_report_slot() {
            return;
        }
        let context = self.tracker.context();
        let current = self.done.load(Ordering::Relaxed);
        let total = self.total.load(Ordering::Relaxed);
        let _ =
            tx.try_send(self.build(context.phase, context.message, current, total, false, None));
    }
    /// Unthrottled update that keeps the current stage headline.
    async fn send_context(&self) {
        let context = self.tracker.context();
        let current = self.done.load(Ordering::Relaxed);
        let total = self.total.load(Ordering::Relaxed);
        self.send_counts_with(&context.phase, context.message, current, total, false, None)
            .await;
    }

    /* ------------------------------------------------------------- reporting */

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
        self.tracker.set_context(phase, &message);
        let task = self.build(phase.to_string(), message, current, total, done, error);
        if let Some(tx) = &self.tx {
            let _ = tx.send(task).await;
        }
    }
    fn build(
        &self,
        phase: String,
        message: String,
        current: u64,
        total: u64,
        done: bool,
        error: Option<String>,
    ) -> InstallTask {
        let (downloaded_bytes, total_bytes, active) = self.tracker.snapshot();
        InstallTask {
            id: self.task_id.clone(),
            instance_id: self.instance_id.clone(),
            phase,
            current,
            total,
            message,
            done,
            error,
            downloaded_bytes,
            total_bytes,
            active,
        }
    }
}

/// One entry per destination, so the segment workers of a file merge instead of
/// racing each other into separate rows.
pub(super) fn item_key(item: &DownloadItem) -> String {
    item.dest.to_string_lossy().into_owned()
}

/// What the UI shows for a file: the plan label, or its file name when the plan
/// never gave one (the full path stays available through the destination).
pub(super) fn display_name(item: &DownloadItem) -> String {
    if !item.label.is_empty() {
        return item.label.clone();
    }
    item.dest
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| item.dest.display().to_string())
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
        let pid = name
            .strip_prefix(&prefix)
            .and_then(|s| s.strip_suffix(".part"));
        if pid.is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())) {
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
mod tests;
