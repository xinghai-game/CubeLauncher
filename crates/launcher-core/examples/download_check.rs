//! Live BMCLAPI client-JAR download check (not part of the offline test suite).
//! cargo run -p launcher-core --example download_check -- /tmp/cube-download-check 1.20.1
use anyhow::{Context, Result};
use launcher_core::download::download_item_from;
use launcher_core::{Downloader, Manifest, Progress, VersionMeta, MANIFEST_URL};
use std::path::PathBuf;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let directory = PathBuf::from(args.next().context("请指定用于下载验证的目录")?);
    let version = args.next().unwrap_or_else(|| "1.20.1".into());
    let downloader = Downloader::new(4, false, Some("https://bmclapi2.bangbang93.com".into()))?;
    let manifest: Manifest = downloader.fetch_json(MANIFEST_URL).await?;
    let entry = manifest
        .versions
        .iter()
        .find(|entry| entry.id == version)
        .context("版本清单中未找到指定版本")?;
    let meta: VersionMeta = downloader.fetch_json(&entry.url).await?;
    let client = meta
        .downloads
        .as_ref()
        .and_then(|downloads| downloads.get("client"))
        .context("版本元数据缺少客户端 JAR")?;
    let mut item = download_item_from(client, directory.join(format!("{version}.jar")));
    // Pin this check to the mapped endpoint so an origin fallback cannot mask a
    // broken mirror. Normal installations keep the original URL for fallback.
    item.url = downloader.map_url(&item.url);
    println!("下载地址：{}", item.url);
    println!(
        "目标：{}；大小：{:?}；SHA-1：{:?}",
        item.dest.display(),
        item.size,
        item.sha1
    );
    let started = Instant::now();
    downloader
        .download(&item, &Progress::new(None, "download-check"))
        .await?;
    anyhow::ensure!(item.is_satisfied(), "下载结果校验失败");
    println!(
        "下载与校验完成，耗时 {:.2}s",
        started.elapsed().as_secs_f64()
    );
    let cached = Instant::now();
    downloader
        .download(&item, &Progress::new(None, "cache-check"))
        .await?;
    println!(
        "再次请求已复用完整缓存，耗时 {:.2}s",
        cached.elapsed().as_secs_f64()
    );
    Ok(())
}
