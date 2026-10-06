//! Command line smoke harness for the launcher core.
//!
//! It is the same code path the desktop UI drives, so it can prove an install
//! and a launch work without opening a window:
//!
//! ```text
//! cargo run -p launcher-core --example smoke -- plan 1.20.1
//! cargo run -p launcher-core --example smoke -- install /tmp/cube alice 1.20.1 fabric
//! cargo run -p launcher-core --example smoke -- verify /tmp/cube alice
//! cargo run -p launcher-core --example smoke -- preview /tmp/cube alice
//! cargo run -p launcher-core --example smoke -- launch /tmp/cube alice
//! ```

use anyhow::{bail, Result};
use launcher_core::rules::current_arch;
use launcher_core::{AppSettings, Instance, LauncherCore, Loader, Progress};
use std::path::PathBuf;
use tokio::sync::mpsc;

fn settings_for(data_dir: &str, offline: bool) -> AppSettings {
    // `--mirror <url>` exercises the optional mirror path, which is also how this
    // harness reaches loader Maven hosts whose TLS chain is broken locally.
    let args: Vec<String> = std::env::args().collect();
    let mirror = args
        .iter()
        .position(|value| value == "--mirror")
        .and_then(|index| args.get(index + 1).cloned());
    let java_mirror = args
        .iter()
        .position(|value| value == "--java-mirror")
        .and_then(|index| args.get(index + 1).cloned());
    AppSettings {
        data_dir: PathBuf::from(data_dir),
        download_concurrency: 8,
        offline_mode: offline,
        mirror_base_url: mirror,
        java_mirror_base_url: java_mirror,
        ..Default::default()
    }
}

async fn progress_channel(
    label: &'static str,
    instance_id: &str,
) -> (Progress, tokio::task::JoinHandle<()>) {
    let (sender, mut receiver) = mpsc::channel(256);
    let progress = Progress::new(Some(sender), instance_id.to_string());
    let handle = tokio::spawn(async move {
        let mut last = String::new();
        let mut count = 0u64;
        while let Some(task) = receiver.recv().await {
            count += 1;
            let line = format!(
                "[{label}] {:>10} {}/{} {}",
                task.phase, task.current, task.total, task.message
            );
            if line != last || task.done {
                println!("{line}");
                last = line;
            }
            if count.is_multiple_of(200) {
                println!("[{label}] … {count} 次进度更新");
            }
        }
    });
    (progress, handle)
}

async fn find_instance(core: &LauncherCore, name: &str) -> Result<Instance> {
    let instances = core.instances().await?;
    instances
        .iter()
        .find(|instance| instance.id == name || instance.name == name)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("找不到实例 {name}"))
}

#[tokio::main]
async fn main() -> Result<()> {
    // Flags (`--mirror <url>`, `--offline`) are removed so positional arguments
    // keep their documented meaning.
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut args: Vec<String> = Vec::new();
    let mut index = 0;
    while index < raw.len() {
        match raw[index].as_str() {
            "--mirror" | "--java-mirror" => index += 2,
            "--offline" => index += 1,
            value => {
                args.push(value.to_string());
                index += 1;
            }
        }
    }
    let Some(command) = args.first().map(|value| value.as_str()) else {
        bail!("用法：smoke <plan|install|verify|preview|launch|java|versions> …");
    };
    match command {
        // Resolve metadata and print the install size without downloading anything.
        "plan" => {
            let version = args.get(1).cloned().unwrap_or_else(|| "1.20.1".into());
            let core = LauncherCore::new(settings_for("/tmp/cube-smoke", false)).await?;
            let plan = core.plan_vanilla(&version).await?;
            let total: u64 = plan.files.iter().filter_map(|file| file.size).sum();
            println!(
                "版本 {}（客户端 JAR 来自 {}）：{} 个文件，{} 个资源对象，约 {:.1} MiB，需要 Java {}",
                plan.resolved.meta.id,
                plan.resolved.jar_version,
                plan.files.len(),
                plan.asset_count,
                total as f64 / 1024.0 / 1024.0,
                plan.java_major
            );
            println!("继承链：{}", plan.resolved.chain.join(" -> "));
        }
        "versions" => {
            let loader = match args.get(1).map(|value| value.as_str()) {
                Some("forge") => Loader::Forge,
                Some("neoforge") => Loader::NeoForge,
                _ => Loader::Fabric,
            };
            let game_version = args.get(2).cloned().unwrap_or_else(|| "1.20.1".into());
            let core = LauncherCore::new(settings_for("/tmp/cube-smoke", false)).await?;
            let versions = match loader {
                Loader::Fabric => core
                    .fabric_loader_versions(&game_version)
                    .await?
                    .into_iter()
                    .map(|entry| format!("{} stable={}", entry.version, entry.stable))
                    .collect::<Vec<_>>(),
                other => core.loader_versions(&game_version, other).await?,
            };
            println!(
                "{} {} 前 5 个版本：{:#?}",
                loader.label(),
                game_version,
                &versions[..versions.len().min(5)]
            );
        }
        "java" => {
            let major: u32 = args
                .get(1)
                .and_then(|value| value.parse().ok())
                .unwrap_or(21);
            let data_dir = args
                .get(2)
                .cloned()
                .unwrap_or_else(|| "/tmp/cube-smoke".into());
            let core = LauncherCore::new(settings_for(&data_dir, false)).await?;
            println!("已发现运行时：{:#?}", core.discover_java().await);
            let (progress, handle) = progress_channel("java", "java").await;
            let runtime = core.install_java(major, current_arch(), &progress).await?;
            drop(progress);
            let _ = handle.await;
            println!(
                "已安装 Java：{}（{}，{}）",
                runtime.path.display(),
                runtime.major,
                runtime.architecture
            );
        }
        "install" => {
            let data_dir = args
                .get(1)
                .cloned()
                .unwrap_or_else(|| "/tmp/cube-smoke".into());
            let name = args.get(2).cloned().unwrap_or_else(|| "smoke".into());
            let game_version = args.get(3).cloned().unwrap_or_else(|| "1.20.1".into());
            let loader = match args.get(4).map(|value| value.as_str()) {
                Some("fabric") => Loader::Fabric,
                Some("forge") => Loader::Forge,
                Some("neoforge") => Loader::NeoForge,
                _ => Loader::Vanilla,
            };
            let loader_version = args.get(5).cloned();
            let core = LauncherCore::new(settings_for(&data_dir, false)).await?;
            let account = match core.accounts().await?.first() {
                Some(account) => account.clone(),
                None => core.add_offline_account("Alice").await?,
            };
            let instance = match core
                .instances()
                .await?
                .into_iter()
                .find(|item| item.name == name)
            {
                Some(existing) => existing,
                None => {
                    core.create_instance(
                        &name,
                        &game_version,
                        loader,
                        loader_version,
                        Some(account.id.clone()),
                    )
                    .await?
                }
            };
            println!(
                "实例 {}（{} / {}）数据目录 {}",
                instance.name,
                instance.game_version,
                instance.loader.label(),
                core.paths.root.display()
            );
            let (progress, handle) = progress_channel("install", &instance.id).await;
            let lock = core.install_instance(&instance, progress).await?;
            let _ = handle.await;
            println!(
                "安装完成：{:?} {} / Java {:?}",
                lock.loader,
                lock.loader_version.clone().unwrap_or_default(),
                lock.java_major
            );
            let instance = core.instance(&instance.id).await?;
            let missing = core.verify_instance(&instance).await?;
            println!("校验结果：{} 个缺失项 {missing:?}", missing.len());
        }
        "verify" => {
            let data_dir = args
                .get(1)
                .cloned()
                .unwrap_or_else(|| "/tmp/cube-smoke".into());
            let name = args.get(2).cloned().unwrap_or_else(|| "smoke".into());
            let offline = args.iter().any(|value| value == "--offline");
            let core = LauncherCore::new(settings_for(&data_dir, offline)).await?;
            let instance = find_instance(&core, &name).await?;
            let missing = core.verify_instance(&instance).await?;
            println!("缺失 {} 项：{missing:#?}", missing.len());
        }
        // Exercise local mod management: list, add, disable, enable, delete.
        "mods" => {
            let data_dir = args
                .get(1)
                .cloned()
                .unwrap_or_else(|| "/tmp/cube-smoke".into());
            let name = args.get(2).cloned().unwrap_or_else(|| "smoke".into());
            let core = LauncherCore::new(settings_for(&data_dir, false)).await?;
            let instance = find_instance(&core, &name).await?;
            let directory = core.ensure_mods_dir(&instance.id).await?;
            println!("模组目录：{}", directory.display());

            // A tiny valid jar keeps the test self-contained.
            let sample = directory.join("cubelauncher-sample.jar");
            write_sample_jar(&sample)?;
            let added = core.list_mods(&instance.id).await?;
            println!("添加后：{added:#?}");

            core.toggle_mod(&instance.id, "cubelauncher-sample.jar", false)
                .await?;
            println!("停用后：{:#?}", core.list_mods(&instance.id).await?);

            core.toggle_mod(&instance.id, "cubelauncher-sample.jar.disabled", true)
                .await?;
            println!("重新启用后：{:#?}", core.list_mods(&instance.id).await?);

            core.delete_mod(&instance.id, "cubelauncher-sample.jar")
                .await?;
            println!(
                "删除后剩余 {} 个文件",
                core.list_mods(&instance.id).await?.len()
            );

            // Path traversal must be rejected.
            match core.delete_mod(&instance.id, "../escape.jar").await {
                Ok(_) => bail!("应当拒绝越界文件名"),
                Err(error) => println!("越界文件名被拒绝：{error}"),
            }
        }
        "preview" | "launch" => {
            let data_dir = args
                .get(1)
                .cloned()
                .unwrap_or_else(|| "/tmp/cube-smoke".into());
            let name = args.get(2).cloned().unwrap_or_else(|| "smoke".into());
            let offline = args.iter().any(|value| value == "--offline");
            let core = LauncherCore::new(settings_for(&data_dir, offline)).await?;
            let instance = find_instance(&core, &name).await?;
            let preview = core.launch_preview(&instance).await?;
            println!("Java：{}", preview.java.display());
            println!("工作目录：{}", preview.working_dir.display());
            println!("原生库：{}", preview.natives_dir.display());
            println!("classpath 条目：{}", preview.classpath_entries);
            println!("缺失文件：{:?}", preview.missing_files);
            println!("参数：{}", preview.args.join(" "));
            if command == "launch" {
                if !preview.missing_files.is_empty() {
                    bail!("缺少文件，无法启动");
                }
                let (sender, mut receiver) = mpsc::channel::<launcher_core::LogLine>(512);
                let logger = tokio::spawn(async move {
                    while let Some(line) = receiver.recv().await {
                        println!("[{}] {}", line.stream, line.line);
                    }
                });
                let code = core.launch_game(&instance, sender).await?;
                let _ = logger.await;
                println!("退出码：{code}");
            }
        }
        other => bail!("未知命令 {other}"),
    }
    Ok(())
}

/// Write the smallest possible valid jar so mod handling can be tested offline.
fn write_sample_jar(path: &std::path::Path) -> Result<()> {
    use std::io::Write;
    let file = std::fs::File::create(path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("pack.mcmeta", options)?;
    zip.write_all(br#"{"pack":{"pack_format":15,"description":"CubeLauncher test"}}"#)?;
    zip.finish()?;
    Ok(())
}
