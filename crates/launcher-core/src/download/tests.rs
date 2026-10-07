use super::engine::segment_path;
use super::*;
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const NORMAL: usize = 0;
const IGNORE_RANGE: usize = 1;
const WRONG_RANGE: usize = 2;
const DISCONNECT: usize = 3;
const STALL: usize = 4;
const FAIL: usize = 5;
const WRONG_LENGTH: usize = 6;
const WRONG_TOTAL: usize = 7;
const OVERSIZE: usize = 8;
const ENCODED: usize = 9;

struct State {
    bytes: Vec<u8>,
    mode: AtomicUsize,
    requests: Mutex<Vec<(String, Option<String>)>>,
    active: AtomicUsize,
    peak: AtomicUsize,
    delay_ms: AtomicU64,
    drip_ms: AtomicU64,
    validator: AtomicUsize,
}
struct Server {
    base: String,
    state: Arc<State>,
    task: tokio::task::JoinHandle<()>,
}
impl Server {
    async fn new(size: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(State {
            bytes: (0..size).map(|i| (i % 251) as u8).collect(),
            mode: AtomicUsize::new(NORMAL),
            requests: Mutex::new(Vec::new()),
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            delay_ms: AtomicU64::new(0),
            drip_ms: AtomicU64::new(0),
            validator: AtomicUsize::new(0),
        });
        let shared = state.clone();
        let task = tokio::spawn(async move {
            let mut clients = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((socket, _)) = accepted else { break; };
                        let state = shared.clone();
                        clients.spawn(async move { let _ = serve(socket, state).await; });
                    }
                    _ = clients.join_next(), if !clients.is_empty() => {},
                }
            }
        });
        Self { base, state, task }
    }
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }
    fn mode(&self, mode: usize) {
        self.state.mode.store(mode, Ordering::SeqCst);
    }
    fn validator(&self, version: usize) {
        self.state.validator.store(version, Ordering::SeqCst);
    }
    fn ranges(&self) -> Vec<Option<String>> {
        self.state
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|(_, range)| range.clone())
            .collect()
    }
    fn clear_requests(&self) {
        self.state.requests.lock().unwrap().clear();
    }
    fn item(&self, dir: &TestDir, name: &str) -> DownloadItem {
        let mut item = DownloadItem::new(self.url(&format!("/{name}")), dir.0.join(name));
        item.size = Some(self.state.bytes.len() as u64);
        item.sha1 = Some(hex::encode(Sha1::digest(&self.state.bytes)));
        item.sha256 = Some(hex::encode(Sha256::digest(&self.state.bytes)));
        item
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
struct Active(Arc<State>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

async fn serve(mut socket: TcpStream, state: Arc<State>) -> std::io::Result<()> {
    let mut request = Vec::new();
    let mut byte = [0; 1];
    while request.len() < 16384 && !request.ends_with(b"\r\n\r\n") {
        if socket.read(&mut byte).await? == 0 {
            return Ok(());
        }
        request.push(byte[0]);
    }
    let request = String::from_utf8_lossy(&request);
    let path = request
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_owned();
    let range = request.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("range")
            .then(|| value.trim().to_owned())
    });
    state.requests.lock().unwrap().push((path, range.clone()));
    let active = state.active.fetch_add(1, Ordering::SeqCst) + 1;
    state.peak.fetch_max(active, Ordering::SeqCst);
    let guard = Active(state.clone());
    let delay = state.delay_ms.load(Ordering::SeqCst);
    if delay > 0 {
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
    let mode = state.mode.load(Ordering::SeqCst);
    if mode == FAIL {
        drop(guard);
        socket
            .write_all(
                b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await?;
        return Ok(());
    }
    let total = state.bytes.len();
    let parsed = range
        .as_ref()
        .and_then(|r| r.strip_prefix("bytes="))
        .map(|r| {
            let (start, end) = r.split_once('-').unwrap();
            (
                start.parse::<usize>().unwrap(),
                if end.is_empty() {
                    total - 1
                } else {
                    end.parse().unwrap()
                },
            )
        });
    if let Some((start, _)) = parsed {
        if start >= total && mode != IGNORE_RANGE {
            let total = if mode == WRONG_TOTAL {
                total + 1
            } else {
                total
            };
            let etag = state.validator.load(Ordering::SeqCst);
            drop(guard);
            socket.write_all(format!("HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{total}\r\nETag: \"v{etag}\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await?;
            return Ok(());
        }
    }
    let (start, end, partial) = match parsed {
        Some((start, end)) if mode != IGNORE_RANGE => (start, end.min(total - 1), true),
        _ => (0, total - 1, false),
    };
    let length = end - start + 1;
    let declared = if mode == WRONG_LENGTH {
        length + 1
    } else {
        length
    };
    let content_range = if partial {
        let header_start = if mode == WRONG_RANGE {
            start + 1
        } else {
            start
        };
        let header_total = if mode == WRONG_TOTAL {
            total + 1
        } else {
            total
        };
        format!("Content-Range: bytes {header_start}-{end}/{header_total}\r\n")
    } else {
        String::new()
    };
    let encoding = if mode == ENCODED {
        "Content-Encoding: gzip\r\n"
    } else {
        ""
    };
    let length_header = if mode == OVERSIZE {
        String::new()
    } else {
        format!("Content-Length: {declared}\r\n")
    };
    let etag = state.validator.load(Ordering::SeqCst);
    let headers = format!(
        "HTTP/1.1 {}\r\n{content_range}{length_header}{encoding}ETag: \"v{etag}\"\r\nConnection: close\r\n\r\n",
        if partial {
            "206 Partial Content"
        } else {
            "200 OK"
        }
    );
    socket.write_all(headers.as_bytes()).await?;
    if mode == DISCONNECT || mode == STALL {
        let sent = length.min(1024);
        socket.write_all(&state.bytes[start..start + sent]).await?;
        if mode == STALL {
            // A readable EOF promptly observes the client's cancellation.
            let mut discard = [0; 1];
            let _ = socket.read(&mut discard).await;
        }
        drop(guard);
        return Ok(());
    }
    if length > 32768 && state.drip_ms.load(Ordering::SeqCst) == 0 {
        let split = length - 32768;
        socket.write_all(&state.bytes[start..start + split]).await?;
        // Final bytes finish the response; update activity before making them visible.
        drop(guard);
        socket.write_all(&state.bytes[start + split..=end]).await?;
    } else {
        drop(guard);
        let drip = state.drip_ms.load(Ordering::SeqCst);
        let mut cursor = start;
        while cursor <= end {
            let piece = (end - cursor + 1).min(32 * 1024);
            socket
                .write_all(&state.bytes[cursor..cursor + piece])
                .await?;
            cursor += piece;
            if drip > 0 && cursor <= end {
                tokio::time::sleep(Duration::from_millis(drip)).await;
            }
        }
    }
    if mode == OVERSIZE {
        socket.write_all(b"extra").await?;
    }
    Ok(())
}

struct TestDir(PathBuf);
impl TestDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "cube-download-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for TestDir {
    fn drop(&mut self) {
        assert_eq!(self.0.parent(), Some(std::env::temp_dir().as_path()));
        assert!(self
            .0
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(&format!("cube-download-test-{}-", std::process::id())));
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn progress() -> Progress {
    Progress::new(None, "test")
}
fn downloader(concurrency: usize) -> Downloader {
    Downloader::new(concurrency, false, None).unwrap()
}
async fn until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("condition timed out");
}

#[tokio::test]
async fn download_resume_survives_restart_and_cleans_only_legacy_pid() {
    let server = Server::new(8192).await;
    let dir = TestDir::new();
    let item = server.item(&dir, "foo.jar");
    let partial = temp_path(&item.dest);
    tokio::fs::write(&partial, &server.state.bytes[..1234])
        .await
        .unwrap();
    let legacy = dir.0.join("foo.jar.12345.part");
    let unrelated = dir.0.join("foo.jar.notes.part");
    tokio::fs::write(&legacy, b"old").await.unwrap();
    tokio::fs::write(&unrelated, b"keep").await.unwrap();
    downloader(2).download(&item, &progress()).await.unwrap();
    assert!(item.is_satisfied());
    assert_eq!(server.ranges(), vec![Some("bytes=1234-".into())]);
    assert!(!legacy.exists());
    assert!(unrelated.exists());
    assert!(!partial.exists());
}

#[tokio::test]
async fn download_network_failure_retains_bytes_and_restarts_from_them() {
    let server = Server::new(16384).await;
    let dir = TestDir::new();
    let item = server.item(&dir, "file");
    server.mode(DISCONNECT);
    assert!(downloader(2).download(&item, &progress()).await.is_err());
    assert!(!item.dest.exists());
    let kept = tokio::fs::read(temp_path(&item.dest)).await.unwrap();
    assert!(!kept.is_empty());
    assert_eq!(kept, server.state.bytes[..kept.len()]);
    server.mode(NORMAL);
    server.clear_requests();
    downloader(2).download(&item, &progress()).await.unwrap();
    assert_eq!(server.ranges()[0], Some(format!("bytes={}-", kept.len())));
    assert!(item.is_satisfied());
}

#[tokio::test]
async fn hashless_resume_binds_source_and_restarts_on_changed_etag() {
    let server = Server::new(16384).await;
    let dir = TestDir::new();
    let mut item = server.item(&dir, "hashless");
    item.sha1 = None;
    item.sha256 = None;
    server.mode(DISCONNECT);
    assert!(downloader(2).download(&item, &progress()).await.is_err());
    let partial = tokio::fs::read(temp_path(&item.dest)).await.unwrap();
    assert!(!partial.is_empty());
    server.validator(1);
    server.mode(NORMAL);
    server.clear_requests();
    downloader(2).download(&item, &progress()).await.unwrap();
    assert!(server
        .ranges()
        .iter()
        .any(|range| range == &Some(format!("bytes={}-", partial.len()))));
    assert!(server.ranges().iter().any(Option::is_none));
    assert!(item.is_satisfied());
}

#[tokio::test]
async fn hashless_partial_without_validator_is_restarted_from_zero() {
    let server = Server::new(8192).await;
    let dir = TestDir::new();
    let mut item = server.item(&dir, "hashless-no-validator");
    item.sha1 = None;
    item.sha256 = None;
    tokio::fs::write(temp_path(&item.dest), &server.state.bytes[..1234])
        .await
        .unwrap();
    downloader(2).download(&item, &progress()).await.unwrap();
    assert_eq!(server.ranges(), vec![None]);
    assert!(item.is_satisfied());
}

#[tokio::test]
async fn download_ignored_range_restarts_without_appending() {
    let server = Server::new(8192).await;
    server.mode(IGNORE_RANGE);
    let dir = TestDir::new();
    let item = server.item(&dir, "file");
    tokio::fs::write(temp_path(&item.dest), &server.state.bytes[..999])
        .await
        .unwrap();
    downloader(2).download(&item, &progress()).await.unwrap();
    assert_eq!(server.ranges(), vec![Some("bytes=999-".into())]);
    assert!(item.is_satisfied());
}

#[tokio::test]
async fn download_rejects_wrong_range_length_and_total_before_writing() {
    let server = Server::new(8192).await;
    let dir = TestDir::new();
    let item = server.item(&dir, "file");
    let partial = temp_path(&item.dest);
    let prefix = &server.state.bytes[..1000];
    for mode in [WRONG_RANGE, WRONG_LENGTH, WRONG_TOTAL, ENCODED] {
        tokio::fs::write(&partial, prefix).await.unwrap();
        server.mode(mode);
        assert!(downloader(1).download(&item, &progress()).await.is_err());
        assert_eq!(tokio::fs::read(&partial).await.unwrap(), prefix);
        assert!(!item.dest.exists());
    }
}

#[tokio::test]
async fn download_416_requires_complete_size_and_hash() {
    let server = Server::new(8192).await;
    let dir = TestDir::new();
    let good = server.item(&dir, "good");
    tokio::fs::write(temp_path(&good.dest), &server.state.bytes)
        .await
        .unwrap();
    downloader(2).download(&good, &progress()).await.unwrap();
    assert_eq!(server.ranges(), vec![Some("bytes=8192-".into())]);
    assert!(good.is_satisfied());
    server.clear_requests();
    let bad = server.item(&dir, "bad");
    tokio::fs::write(temp_path(&bad.dest), vec![0; 8192])
        .await
        .unwrap();
    downloader(2).download(&bad, &progress()).await.unwrap();
    assert_eq!(server.ranges(), vec![Some("bytes=8192-".into()), None]);
    assert!(bad.is_satisfied());
    server.mode(WRONG_TOTAL);
    let wrong = server.item(&dir, "wrong");
    tokio::fs::write(temp_path(&wrong.dest), &server.state.bytes)
        .await
        .unwrap();
    assert!(downloader(1).download(&wrong, &progress()).await.is_err());
    assert!(!wrong.dest.exists());
}

#[tokio::test]
async fn download_cancels_stalled_body_quickly_and_keeps_resume_data() {
    let server = Server::new(32768).await;
    server.mode(STALL);
    let dir = TestDir::new();
    let item = server.item(&dir, "file");
    let progress = progress();
    let task_item = item.clone();
    let task_progress = progress.clone();
    let task =
        tokio::spawn(async move { downloader(2).download(&task_item, &task_progress).await });
    until(|| std::fs::metadata(temp_path(&item.dest)).is_ok_and(|m| m.len() > 0)).await;
    progress.cancel.store(true, Ordering::SeqCst);
    let result = tokio::time::timeout(Duration::from_millis(500), task)
        .await
        .expect("cancel was slow")
        .unwrap();
    assert!(result.is_err());
    let kept = tokio::fs::read(temp_path(&item.dest)).await.unwrap();
    assert!(!kept.is_empty());
    server.mode(NORMAL);
    server.clear_requests();
    downloader(2)
        .download(&item, &super::tests::progress())
        .await
        .unwrap();
    assert_eq!(server.ranges()[0], Some(format!("bytes={}-", kept.len())));
    assert!(item.is_satisfied());
}

#[tokio::test]
async fn download_large_file_segments_parallel_and_share_global_budget() {
    let server = Server::new(8 * 1024 * 1024).await;
    server.state.delay_ms.store(20, Ordering::SeqCst);
    let dir = TestDir::new();
    let items: Vec<_> = (0..3)
        .map(|i| server.item(&dir, &format!("large-{i}")))
        .collect();
    let downloader = downloader(3);
    let metadata_url = server.url("/metadata");
    let download_progress = progress();
    let (files, metadata) = tokio::join!(
        downloader.download_all(items.clone(), &download_progress),
        downloader.fetch_bytes(&metadata_url)
    );
    files.unwrap();
    metadata.unwrap();
    assert!(items.iter().all(DownloadItem::is_satisfied));
    let peak = server.state.peak.load(Ordering::SeqCst);
    assert!((2..=3).contains(&peak), "peak={peak}");
    assert!(server
        .ranges()
        .contains(&Some("bytes=2097152-4194303".into())));
}

#[tokio::test]
async fn download_small_files_parallel_and_same_destination_serialized() {
    let server = Server::new(8192).await;
    server.state.delay_ms.store(30, Ordering::SeqCst);
    let dir = TestDir::new();
    let items: Vec<_> = (0..7)
        .map(|i| server.item(&dir, &format!("small-{i}")))
        .collect();
    downloader(2)
        .download_all(items.clone(), &progress())
        .await
        .unwrap();
    assert_eq!(server.state.peak.load(Ordering::SeqCst), 2);
    assert!(items.iter().all(DownloadItem::is_satisfied));
    let item = server.item(&dir, "shared");
    server.clear_requests();
    let first = downloader(2);
    let second = downloader(2);
    let a = progress();
    let b = progress();
    let (a, b) = tokio::join!(first.download(&item, &a), second.download(&item, &b));
    a.unwrap();
    b.unwrap();
    assert_eq!(server.ranges().len(), 1);
}

#[tokio::test]
async fn download_large_file_without_ranges_falls_back() {
    let server = Server::new(8 * 1024 * 1024).await;
    server.mode(IGNORE_RANGE);
    let dir = TestDir::new();
    let item = server.item(&dir, "large");
    downloader(4).download(&item, &progress()).await.unwrap();
    assert_eq!(server.ranges(), vec![Some("bytes=0-0".into()), None]);
    assert!(item.is_satisfied());
}

#[tokio::test]
async fn download_segments_resume_after_failure_with_changed_concurrency() {
    let server = Server::new(8 * 1024 * 1024).await;
    server.mode(DISCONNECT);
    let dir = TestDir::new();
    let item = server.item(&dir, "large");
    assert!(downloader(4).download(&item, &progress()).await.is_err());
    let lengths: Vec<_> = (0..4)
        .map(|i| std::fs::metadata(segment_path(&item, i)).unwrap().len())
        .collect();
    assert!(lengths.iter().all(|n| *n > 0));
    server.mode(NORMAL);
    server.clear_requests();
    downloader(1).download(&item, &progress()).await.unwrap();
    for (i, length) in lengths.iter().enumerate() {
        let start = i as u64 * 2097152 + length;
        let end = (i as u64 + 1) * 2097152 - 1;
        assert!(server
            .ranges()
            .contains(&Some(format!("bytes={start}-{end}"))));
        assert!(!segment_path(&item, i as u64).exists());
    }
    assert!(item.is_satisfied());
}

#[tokio::test]
async fn download_mirror_failures_fall_back_for_metadata_and_files() {
    let mirror = Server::new(8192).await;
    let origin = Server::new(8192).await;
    mirror.mode(FAIL);
    let dir = TestDir::new();
    let item = origin.item(&dir, "file");
    let sources = vec![mirror.url("/file"), item.url.clone()];
    let downloader = downloader(2);
    let (bytes, from_mirror) = downloader
        .fetch_bytes_candidates(&item.url, &sources)
        .await
        .unwrap();
    assert_eq!(bytes, origin.state.bytes);
    assert!(!from_mirror);
    downloader
        .download_candidates(&item, &progress(), &sources)
        .await
        .unwrap();
    assert!(item.is_satisfied());
    assert_eq!(mirror.ranges().len(), 2);
    mirror.mode(DISCONNECT);
    let interrupted = origin.item(&dir, "interrupted");
    let sources = vec![mirror.url("/interrupted"), interrupted.url.clone()];
    origin.clear_requests();
    downloader
        .download_candidates(&interrupted, &progress(), &sources)
        .await
        .unwrap();
    assert!(origin.ranges()[0]
        .as_ref()
        .is_some_and(|r| r.starts_with("bytes=1024-")));
    assert!(interrupted.is_satisfied());
}

#[tokio::test]
async fn download_never_publishes_oversize_or_bad_hash() {
    let server = Server::new(8192).await;
    let dir = TestDir::new();
    let mut item = server.item(&dir, "file");
    item.sha256 = Some("00".repeat(32));
    assert!(downloader(2).download(&item, &progress()).await.is_err());
    assert!(!item.dest.exists());
    server.mode(OVERSIZE);
    let mut oversized = server.item(&dir, "oversized");
    oversized.size = Some(1);
    assert!(downloader(2)
        .download(&oversized, &progress())
        .await
        .is_err());
    assert!(!oversized.dest.exists());
}

#[tokio::test]
async fn download_cancelled_cached_work_is_still_cancelled() {
    let server = Server::new(16).await;
    let dir = TestDir::new();
    let item = server.item(&dir, "cached");
    tokio::fs::write(&item.dest, &server.state.bytes)
        .await
        .unwrap();
    let progress = progress();
    progress.cancel.store(true, Ordering::SeqCst);
    let downloader = downloader(2);
    assert!(downloader.download(&item, &progress).await.is_err());
    assert!(downloader
        .download_all(vec![item.clone()], &progress)
        .await
        .is_err());
    assert!(downloader
        .download_one(item, &progress, "download")
        .await
        .is_err());
    assert!(server.ranges().is_empty());
}

#[test]
fn download_partial_name_and_resource_url_are_stable() {
    assert_eq!(
        temp_path(Path::new("foo.jar")),
        PathBuf::from("foo.jar.part")
    );
    assert_eq!(resource_url("abcdef"), format!("{RESOURCES_URL}/ab/abcdef"));
    assert!(!DownloadItem::new(
        "https://example.invalid/file",
        PathBuf::from("/tmp/nope-download-test")
    )
    .is_satisfied());
}

#[tokio::test]
async fn progress_reports_the_file_transferring_and_its_bytes() {
    let server = Server::new(512 * 1024).await;
    server.state.drip_ms.store(40, Ordering::SeqCst);
    let dir = TestDir::new();
    let mut item = server.item(&dir, "client.jar");
    item.label = "客户端 JAR".into();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<InstallTask>(64);
    let progress = Progress::new(Some(tx), "test");
    let task_progress = progress.clone();
    let download = tokio::spawn(async move {
        downloader(1).download(&item, &task_progress).await.unwrap();
    });
    download.await.unwrap();
    drop(progress);
    let mut events = Vec::new();
    while let Some(task) = rx.recv().await {
        events.push(task);
    }
    let labeled = events
        .iter()
        .find(|task| task.active.iter().any(|file| file.name == "客户端 JAR"));
    assert!(labeled.is_some(), "no event listed the labeled file");
    let midpoint = events.iter().find(|task| {
        task.active.iter().any(|file| {
            file.name == "客户端 JAR" && 0 < file.downloaded && file.downloaded < file.total
        })
    });
    assert!(
        midpoint.is_some(),
        "no event showed a partially downloaded file: {events:#?}"
    );
    // Byte totals only move forward while this file transfers.
    let mut last = 0;
    for task in &events {
        assert!(task.downloaded_bytes >= last, "bytes went backwards");
        last = task.downloaded_bytes;
    }
    let terminal = events.last().unwrap();
    assert_eq!(terminal.total_bytes, 512 * 1024);
    assert_eq!(terminal.downloaded_bytes, terminal.total_bytes);
    assert!(terminal.active.is_empty());
}

#[tokio::test]
async fn progress_final_snapshot_counts_completed_files_and_drops_active_ones() {
    let server = Server::new(32 * 1024).await;
    let dir = TestDir::new();
    // One file is already valid on disk and counts as downloaded from the start.
    let existing = server.item(&dir, "existing");
    tokio::fs::write(&existing.dest, &server.state.bytes)
        .await
        .unwrap();
    let fetched = server.item(&dir, "fetched");
    let items = vec![existing, fetched.clone()];
    let (tx, mut rx) = tokio::sync::mpsc::channel::<InstallTask>(64);
    let progress = Progress::new(Some(tx), "test");
    downloader(2).download_all(items, &progress).await.unwrap();
    drop(progress);
    let mut terminal = None;
    while let Some(task) = rx.recv().await {
        terminal = Some(task);
    }
    let terminal = terminal.unwrap();
    assert_eq!(terminal.current, 2);
    assert_eq!(terminal.downloaded_bytes, 64 * 1024);
    assert_eq!(terminal.total_bytes, 64 * 1024);
    assert!(terminal.active.is_empty());
    // A label-less item still names its file in the list.
    assert_eq!(display_name(&fetched), "fetched");
}

#[tokio::test]
async fn progress_failed_transfers_leave_no_active_files() {
    let server = Server::new(32 * 1024).await;
    server.mode(DISCONNECT);
    let dir = TestDir::new();
    let item = server.item(&dir, "killed");
    let (tx, mut rx) = tokio::sync::mpsc::channel::<InstallTask>(64);
    let progress = Progress::new(Some(tx), "test");
    assert!(downloader(2)
        .download_all(vec![item], &progress)
        .await
        .is_err());
    drop(progress);
    let mut terminal = None;
    while let Some(task) = rx.recv().await {
        terminal = Some(task);
    }
    let terminal = terminal.unwrap();
    assert!(
        terminal.active.is_empty(),
        "failed files must leave the list"
    );
    assert!(terminal.downloaded_bytes > 0, "kept partial bytes count");
}
