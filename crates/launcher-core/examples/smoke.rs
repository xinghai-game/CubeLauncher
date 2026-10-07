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
//! cargo run -p launcher-core --example smoke -- catalog ~/.local/share/CubeLauncher
//! cargo run -p launcher-core --example smoke -- launch /tmp/cube alice
//! cargo run -p launcher-core --example smoke -- scan ~/.minecraft
//! cargo run -p launcher-core --example smoke -- import /tmp/cube imported ~/.minecraft 1.20.1-forge-47.4.26
//!
//! 正版登录 walks the real Microsoft chain and is therefore interactive:
//!
//! ```text
//! cargo run -p launcher-core --example smoke -- login   /tmp/cube
//! cargo run -p launcher-core --example smoke -- accounts /tmp/cube
//! cargo run -p launcher-core --example smoke -- refresh  /tmp/cube Steve
//! ```

use anyhow::{bail, Result};
use launcher_core::rules::current_arch;
use launcher_core::{AppSettings, Instance, LauncherCore, Loader, Progress, VersionKind};
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
        bail!("用法：smoke <plan|install|verify|preview|launch|java|versions|catalog|mods|scan|import> …");
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
        // Version list as the UI consumes it: categories, counts and provenance.
        "catalog" => {
            let data_dir = args
                .get(1)
                .cloned()
                .unwrap_or_else(|| "/tmp/cube-smoke".into());
            // Offline on purpose: this must work from the cache alone.
            let core = LauncherCore::new(settings_for(&data_dir, true)).await?;
            let catalog = core.catalog(false).await?;
            println!(
                "版本目录：{} 个版本，来源 {}（来自缓存：{}），缓存时间 {}",
                catalog.total,
                catalog.source.label(),
                if catalog.cached { "是" } else { "否" },
                catalog
                    .fetched_at
                    .map(|value| value.to_rfc3339())
                    .unwrap_or_else(|| "未知".into())
            );
            for kind in [
                VersionKind::Release,
                VersionKind::Snapshot,
                VersionKind::OldBeta,
                VersionKind::OldAlpha,
                VersionKind::AprilFools,
            ] {
                let count = catalog
                    .versions
                    .iter()
                    .filter(|version| version.kind == kind)
                    .count();
                println!("  {}：{count}", kind.label());
            }
            println!(
                "最新正式版 {}，最新快照 {}",
                catalog.latest.release, catalog.latest.snapshot
            );
            for id in ["1.20.1", "1.12.2", "b1.8.1"] {
                let requirement = core.required_java_major(id).await;
                println!(
                    "  {id} 需要 Java {}（来源：{}）",
                    requirement.major, requirement.source
                );
            }
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
        // Look inside an existing `.minecraft` the way the import page does.
        "scan" => {
            let path = args
                .get(1)
                .cloned()
                .unwrap_or_else(|| "/tmp/cube-smoke/.minecraft".into());
            let core = LauncherCore::new(settings_for("/tmp/cube-smoke", false)).await?;
            let scan = core.scan_game_dir(std::path::Path::new(&path)).await?;
            println!(
                "游戏目录 {}（{}，模组 {} 个，存档 {} 个，自带 libraries={} assets={}）",
                scan.game_dir.display(),
                if scan.nested {
                    "取自内层 .minecraft"
                } else {
                    "直接使用"
                },
                scan.mod_count,
                scan.save_count,
                scan.has_libraries,
                scan.has_assets
            );
            for version in &scan.versions {
                println!(
                    "  {} → {} / {}{}{}{}{}{}",
                    version.id,
                    version.game_version,
                    version.loader.label(),
                    version
                        .loader_version
                        .as_ref()
                        .map(|value| format!(" {value}"))
                        .unwrap_or_default(),
                    if version.jar { " · 有 JAR" } else { "" },
                    if version.launchable {
                        ""
                    } else {
                        " · 缺少 mainClass"
                    },
                    if version.imported {
                        " · 已导入"
                    } else {
                        ""
                    },
                    version
                        .note
                        .as_ref()
                        .map(|note| format!(" · {note}"))
                        .unwrap_or_default()
                );
            }
        }
        // Register an existing installation in place and immediately verify it.
        "import" => {
            let data_dir = args
                .get(1)
                .cloned()
                .unwrap_or_else(|| "/tmp/cube-smoke".into());
            let name = args.get(2).cloned().unwrap_or_else(|| "imported".into());
            let path = args
                .get(3)
                .cloned()
                .unwrap_or_else(|| "/tmp/cube-smoke/.minecraft".into());
            let version_id = args.get(4).cloned().unwrap_or_else(|| "1.20.1".into());
            let offline = args.iter().any(|value| value == "--offline");
            let core = LauncherCore::new(settings_for(&data_dir, offline)).await?;
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
                Some(existing) => {
                    core.bind_game_dir(
                        &existing.id,
                        Some(std::path::Path::new(&path)),
                        Some(&version_id),
                    )
                    .await?
                }
                None => {
                    core.import_instance(
                        &name,
                        std::path::Path::new(&path),
                        &version_id,
                        Some(account.id.clone()),
                    )
                    .await?
                }
            };
            println!(
                "已导入实例 {}（{} / {} / 版本 {}）",
                instance.name,
                instance.game_version,
                instance.loader.label(),
                instance.version_id.clone().unwrap_or_default()
            );
            println!("游戏目录：{}", core.game_dir(&instance).display());
            let missing = core.verify_instance(&instance).await?;
            println!("校验结果：{} 个缺失项 {missing:?}", missing.len());
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
        // 正版 sign-in, the same calls the account dialog makes: print the code,
        // then poll until the user finishes on Microsoft's page.
        "login" => {
            let data_dir = args
                .get(1)
                .cloned()
                .unwrap_or_else(|| "/tmp/cube-smoke".into());
            let core = LauncherCore::new(settings_for(&data_dir, false)).await?;
            let prompt = core.auth.request_device_code().await?;
            println!(
                "请在浏览器中打开 {} 并输入代码 {}",
                prompt.verification_uri, prompt.user_code
            );
            println!(
                "（{} 秒内有效，每 {} 秒查询一次，Ctrl+C 可以放弃）",
                prompt.expires_in, prompt.interval
            );
            let deadline =
                std::time::Instant::now() + std::time::Duration::from_secs(prompt.expires_in);
            let mut interval = prompt.interval.max(1);
            loop {
                if std::time::Instant::now() >= deadline {
                    bail!("设备代码已过期，请重新登录");
                }
                tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
                match core.auth.poll_device_code(&prompt.login_id).await? {
                    launcher_core::LoginPoll::Pending => {
                        println!("等待用户在 Microsoft 页面完成登录……");
                        continue;
                    }
                    launcher_core::LoginPoll::SlowDown => {
                        interval += 5;
                        println!("Microsoft 要求放慢查询，间隔调整为 {interval} 秒");
                    }
                    launcher_core::LoginPoll::Expired => bail!("设备代码已过期，请重新登录"),
                    launcher_core::LoginPoll::Declined => {
                        bail!("用户在 Microsoft 页面上取消了登录")
                    }
                    launcher_core::LoginPoll::Failed { message } => bail!("{message}"),
                    launcher_core::LoginPoll::Ready { account } => {
                        let account = core.upsert_account(*account).await?;
                        let session = launcher_core::auth::session_for(&account);
                        let microsoft = account.microsoft.as_ref().expect("microsoft block");
                        println!(
                            "登录成功：{}（{}）拥有 Java 版：{}",
                            account.name, account.uuid, microsoft.owns_java
                        );
                        println!(
                            "启动身份：user_type={} xuid={} uuid={} client_id={} 令牌长度={}",
                            session.user_type,
                            session.xuid,
                            session.uuid,
                            session.client_id,
                            session.access_token.len()
                        );
                        println!(
                            "皮肤纹理：{}",
                            microsoft.skin_url.clone().unwrap_or_else(|| "无".into())
                        );
                        println!("刷新令牌已写入 {}/accounts.json", data_dir);
                    }
                }
            }
        }
        // What is stored right now, without ever printing a token.
        "accounts" => {
            let data_dir = args
                .get(1)
                .cloned()
                .unwrap_or_else(|| "/tmp/cube-smoke".into());
            let core = LauncherCore::new(settings_for(&data_dir, false)).await?;
            let accounts = core.accounts().await?;
            println!("共 {} 个角色：", accounts.len());
            for account in &accounts {
                match account.microsoft.as_ref() {
                    Some(microsoft) => println!(
                        "  [正版] {} {} 拥有 Java 版：{} xuid={} 令牌到期：{} 刷新令牌：{} 字符",
                        account.name,
                        account.uuid,
                        microsoft.owns_java,
                        microsoft.xuid,
                        microsoft
                            .access_expires_at
                            .map(|value| value.to_rfc3339())
                            .unwrap_or_else(|| "未知".into()),
                        microsoft.refresh_token.len()
                    ),
                    None => println!("  [离线] {} {}", account.name, account.uuid),
                }
            }
        }
        // Force a refresh, which is what launching an expired account does.
        "refresh" => {
            let data_dir = args
                .get(1)
                .cloned()
                .unwrap_or_else(|| "/tmp/cube-smoke".into());
            let name = args.get(2).cloned().unwrap_or_else(|| "Steve".into());
            let core = LauncherCore::new(settings_for(&data_dir, false)).await?;
            let account = core
                .accounts()
                .await?
                .into_iter()
                .find(|account| account.name == name || account.id == name)
                .ok_or_else(|| anyhow::anyhow!("没有叫 {name} 的角色"))?;
            let refreshed = core.auth.refresh(&account).await?;
            core.update_account(&refreshed).await?;
            let microsoft = refreshed.microsoft.as_ref().expect("microsoft block");
            println!(
                "已刷新 {}：令牌到期 {}，长度 {} 字符",
                refreshed.name,
                microsoft
                    .access_expires_at
                    .map(|value| value.to_rfc3339())
                    .unwrap_or_else(|| "未知".into()),
                microsoft.access_token.len()
            );
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
