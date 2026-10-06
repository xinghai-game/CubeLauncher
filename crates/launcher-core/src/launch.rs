use crate::download::Progress;
use crate::meta::{expand_arguments, substitute, LaunchVariables};
use crate::rules::{classpath_entry, require_java_major, Platform, RuleEnv};
use crate::types::*;
use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{mpsc, oneshot, Mutex};

/// Registry of running game processes so the UI can stop them.
#[derive(Default)]
pub struct ProcessRegistry {
    running: Mutex<HashMap<String, oneshot::Sender<()>>>,
}

impl ProcessRegistry {
    pub async fn is_running(&self, instance_id: &str) -> bool {
        self.running.lock().await.contains_key(instance_id)
    }
    pub async fn running_ids(&self) -> Vec<String> {
        self.running.lock().await.keys().cloned().collect()
    }
    async fn register(&self, instance_id: &str, cancel: oneshot::Sender<()>) -> Result<()> {
        let mut running = self.running.lock().await;
        if running.contains_key(instance_id) {
            bail!("该实例已在运行中");
        }
        running.insert(instance_id.to_string(), cancel);
        Ok(())
    }
    async fn unregister(&self, instance_id: &str) {
        self.running.lock().await.remove(instance_id);
    }
    pub async fn stop(&self, instance_id: &str) -> Result<()> {
        let sender = self.running.lock().await.remove(instance_id);
        match sender {
            Some(sender) => {
                let _ = sender.send(());
                Ok(())
            }
            None => bail!("实例没有在运行"),
        }
    }
}

/// A fully resolved launch command, shown to the user before starting.
pub struct BuiltLaunch {
    pub java: PathBuf,
    pub args: Vec<String>,
    pub working_dir: PathBuf,
    pub natives_dir: PathBuf,
    pub classpath_entries: usize,
    pub missing: Vec<String>,
    pub log_config: Option<PathBuf>,
}

impl crate::LauncherCore {
    pub fn processes(&self) -> &Arc<ProcessRegistry> {
        &self.processes
    }

    /// Build the exact command line for an instance without starting anything.
    pub async fn build_launch(&self, instance: &Instance) -> Result<BuiltLaunch> {
        let lock = self
            .install_lock(&instance.id)
            .await
            .ok_or_else(|| anyhow!("实例尚未安装，请先安装"))?;
        let resolved = self.resolved_for(&instance.id).await?;
        let meta = &resolved.meta;
        let account = self.account_for(instance).await;
        let game_dir = self.paths.game_dir(&instance.id);
        tokio::fs::create_dir_all(&game_dir).await?;

        // Pick Java first: library rules follow the runtime that will start the game.
        let required_major = require_java_major(
            &resolved.jar_version,
            meta.java_version
                .as_ref()
                .and_then(|value| value.major_version),
        )
        .max(lock.java_major.unwrap_or(0));
        let java = match &instance.java_path {
            Some(path) if !path.as_os_str().is_empty() => crate::java::inspect_java(path)
                .with_context(|| format!("实例指定的 Java 不可用：{}", path.display()))?,
            _ => self
                .select_java(required_major, crate::rules::current_arch())
                .await
                .ok_or_else(|| {
                    anyhow!("没有找到 Java {required_major}，请在设置中选择或让启动器自动下载")
                })?,
        };
        let platform = Platform::for_java(Some(&java.architecture));
        let env = RuleEnv::new(
            instance.width.is_some() && instance.height.is_some(),
            !platform.os_version.is_empty(),
        );

        // Native extraction must happen before we point the JVM at the directory.
        self.extract_natives(instance, &resolved).await?;
        let natives_dir = self.paths.natives_dir(&instance.id);
        tokio::fs::create_dir_all(&natives_dir).await?;

        // Classpath: version jar first, then every allowed library artifact.
        let mut classpath: Vec<PathBuf> = Vec::new();
        let mut missing: Vec<String> = Vec::new();
        match self.version_jar_for(&meta.id, &resolved.jar_version) {
            Some(jar) => classpath.push(jar),
            None => missing.push(format!("版本 JAR {}", resolved.jar_version)),
        }
        for library in &meta.libraries {
            if !crate::rules::library_allowed(library, &platform, &env) {
                continue;
            }
            if let Some((path, _url)) = classpath_entry(library, &platform) {
                let dest = self.paths.libraries.join(&path);
                if dest.is_file() {
                    classpath.push(dest);
                } else {
                    missing.push(path);
                }
            }
        }
        let log_config = meta
            .logging
            .as_ref()
            .and_then(|logging| logging.client.as_ref())
            .map(|_| self.log_config_path(&meta.id))
            .filter(|path| path.is_file());

        let separator = if cfg!(windows) { ";" } else { ":" };
        let mut variables = LaunchVariables::default();
        variables.insert("auth_player_name", account.name.clone());
        variables.insert("auth_uuid", account.uuid.replace('-', ""));
        variables.insert("auth_access_token", "0");
        variables.insert("auth_session", format!("token:0:{}", account.uuid));
        variables.insert("auth_xuid", "");
        variables.insert("clientid", "");
        variables.insert("user_type", "legacy");
        variables.insert("user_properties", "{}");
        variables.insert("version_name", meta.id.clone());
        variables.insert("version_type", "release");
        variables.insert("game_directory", game_dir.display().to_string());
        variables.insert("assets_root", self.paths.assets.display().to_string());
        variables.insert("assets_index_name", meta.assets.clone().unwrap_or_default());
        variables.insert(
            "game_assets",
            self.paths
                .assets
                .join("virtual")
                .join("legacy")
                .display()
                .to_string(),
        );
        variables.insert("natives_directory", natives_dir.display().to_string());
        variables.insert("launcher_name", LAUNCHER_NAME);
        variables.insert("launcher_version", LAUNCHER_VERSION);
        variables.insert(
            "library_directory",
            self.paths.libraries.display().to_string(),
        );
        variables.insert(
            "classpath",
            classpath
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(separator),
        );
        variables.insert("classpath_separator", separator);
        if let Some(width) = instance.width {
            variables.insert("resolution_width", width.to_string());
        }
        if let Some(height) = instance.height {
            variables.insert("resolution_height", height.to_string());
        }

        // JVM arguments: our memory settings, then the version's own arguments.
        let mut args: Vec<String> = Vec::new();
        args.push(format!("-Xms{}M", instance.min_memory_mb));
        args.push(format!("-Xmx{}M", instance.max_memory_mb));
        if let Some(values) = meta.arguments.as_ref().and_then(|value| value.jvm.as_ref()) {
            args.extend(expand_arguments(values, &platform, &env, &variables)?);
        }
        let joined = args.join(" ");
        if !joined.contains("java.library.path") {
            args.push(format!("-Djava.library.path={}", natives_dir.display()));
        }
        if cfg!(target_os = "macos") && !joined.contains("XstartOnFirstThread") {
            args.push("-XstartOnFirstThread".to_string());
        }
        if !joined.contains("minecraft.launcher.brand") {
            args.push(format!("-Dminecraft.launcher.brand={LAUNCHER_NAME}"));
            args.push(format!("-Dminecraft.launcher.version={LAUNCHER_VERSION}"));
        }
        // Versions before 1.13 have no `arguments.jvm` section, so the classpath
        // has to be supplied here; without it the main class cannot be found.
        if !joined.contains("-cp ") && !joined.contains("-classpath ") {
            args.push("-cp".to_string());
            args.push(
                classpath
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(separator),
            );
        }
        if let Some(log_config) = &log_config {
            if let Some(client) = meta.logging.as_ref().and_then(|l| l.client.as_ref()) {
                let mut log_variables = variables.clone();
                log_variables.insert("path", log_config.display().to_string());
                args.push(substitute(&client.argument, &log_variables)?);
            }
        }
        args.extend(instance.jvm_args.iter().cloned());

        let main_class = meta
            .main_class
            .clone()
            .ok_or_else(|| anyhow!("版本元数据缺少 mainClass"))?;
        args.push(main_class);

        // Game arguments, legacy string form or modern argument list.
        if let Some(values) = meta
            .arguments
            .as_ref()
            .and_then(|value| value.game.as_ref())
        {
            args.extend(expand_arguments(values, &platform, &env, &variables)?);
        } else if let Some(legacy) = &meta.minecraft_arguments {
            for token in crate::meta::split_legacy_arguments(legacy) {
                args.push(substitute(&token, &variables)?);
            }
        }
        if instance.fullscreen {
            args.push("--fullscreen".to_string());
        }
        args.extend(instance.game_args.iter().cloned());

        Ok(BuiltLaunch {
            java: java.path,
            args,
            working_dir: game_dir,
            natives_dir,
            classpath_entries: classpath.len(),
            missing,
            log_config,
        })
    }

    pub async fn launch_preview(&self, instance: &Instance) -> Result<LaunchPreview> {
        let built = self.build_launch(instance).await?;
        Ok(LaunchPreview {
            java: built.java,
            args: built.args,
            working_dir: built.working_dir,
            classpath_entries: built.classpath_entries,
            natives_dir: built.natives_dir,
            missing_files: built.missing,
        })
    }

    /// Start the game, streaming stdout/stderr to the caller and to a log file.
    pub async fn launch_game(
        &self,
        instance: &Instance,
        logs: mpsc::Sender<LogLine>,
    ) -> Result<i32> {
        if self.processes().is_running(&instance.id).await {
            bail!("该实例已在运行中");
        }
        let built = self.build_launch(instance).await?;
        if !built.missing.is_empty() {
            bail!(
                "实例缺少 {} 个文件，请先安装或修复：{}",
                built.missing.len(),
                built
                    .missing
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("、")
            );
        }
        let mut command = tokio::process::Command::new(&built.java);
        command
            .args(&built.args)
            .current_dir(&built.working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());
        let mut child = command
            .spawn()
            .with_context(|| format!("无法启动 Java：{}", built.java.display()))?;
        let pid = child.id();
        let (cancel_tx, cancel_rx) = oneshot::channel();
        self.processes().register(&instance.id, cancel_tx).await?;
        let log_path = self.paths.logs.join(format!("{}.log", instance.id));
        rotate_log(&log_path).await;
        if let Some(pipe) = child.stdout.take() {
            spawn_reader(pipe, "stdout", logs.clone(), log_path.clone());
        }
        if let Some(pipe) = child.stderr.take() {
            spawn_reader(pipe, "stderr", logs.clone(), log_path.clone());
        }
        let _ = logs
            .send(LogLine {
                stream: "launcher".into(),
                line: format!(
                    "启动 {} · Java {} · {} 个 classpath 条目",
                    instance.name,
                    built.java.display(),
                    built.classpath_entries
                ),
                timestamp: chrono::Utc::now(),
            })
            .await;
        if let Some(pid) = pid {
            tracing::info!("实例 {} 已启动，pid={pid}", instance.id);
        }
        let mut cancelled = false;
        let exit_code = tokio::select! {
            status = child.wait() => {
                match status {
                    Ok(status) => status.code().unwrap_or(-1),
                    Err(error) => {
                        self.processes().unregister(&instance.id).await;
                        return Err(error.into());
                    }
                }
            }
            _ = cancel_rx => {
                cancelled = true;
                let _ = child.start_kill();
                match child.wait().await {
                    Ok(status) => status.code().unwrap_or(-1),
                    Err(_) => -1,
                }
            }
        };
        self.processes().unregister(&instance.id).await;
        let mut updated = instance.clone();
        updated.last_played = Some(chrono::Utc::now());
        if let Err(error) = self.update_instance(&updated).await {
            tracing::warn!("记录最近启动时间失败：{error}");
        }
        let summary = if cancelled {
            format!("游戏已停止（退出码 {exit_code}）")
        } else {
            format!("游戏进程已退出（退出码 {exit_code}）")
        };
        let _ = logs
            .send(LogLine {
                stream: "launcher".into(),
                line: summary.clone(),
                timestamp: chrono::Utc::now(),
            })
            .await;
        append_log_line(&log_path, &summary).await;
        Ok(exit_code)
    }

    /// Ensure an instance can start offline, optionally repairing missing files.
    pub async fn prepare_launch(
        &self,
        instance: &Instance,
        repair: bool,
        progress: Progress,
    ) -> Result<Vec<String>> {
        let missing = self.verify_instance(instance).await?;
        if missing.is_empty() {
            return Ok(missing);
        }
        if !repair {
            return Ok(missing);
        }
        if self.settings.offline_mode {
            bail!(
                "严格离线模式缺少 {} 个文件：{}",
                missing.len(),
                missing
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("、")
            );
        }
        self.install_instance(instance, progress).await?;
        self.verify_instance(instance).await
    }

    async fn account_for(&self, instance: &Instance) -> OfflineAccount {
        let accounts = self.accounts().await.unwrap_or_default();
        accounts
            .iter()
            .find(|account| Some(&account.id) == instance.account_id.as_ref())
            .cloned()
            .or_else(|| accounts.first().cloned())
            .unwrap_or_else(|| OfflineAccount {
                id: crate::instance::offline_uuid("Player"),
                name: "Player".into(),
                uuid: crate::instance::offline_uuid("Player"),
                kind: "offline".into(),
                created_at: chrono::Utc::now(),
            })
    }
}

fn spawn_reader<R>(reader: R, stream: &'static str, logs: mpsc::Sender<LogLine>, log_path: PathBuf)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let entry = LogLine {
                stream: stream.to_string(),
                line: line.clone(),
                timestamp: chrono::Utc::now(),
            };
            append_log_line(&log_path, &format!("[{stream}] {line}")).await;
            if logs.send(entry).await.is_err() {
                break;
            }
        }
    });
}

/// Keep one rotated log per instance so long sessions cannot fill the disk.
async fn rotate_log(path: &PathBuf) {
    const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;
    let Ok(metadata) = tokio::fs::metadata(path).await else {
        return;
    };
    if metadata.len() < MAX_LOG_BYTES {
        return;
    }
    let rotated = path.with_extension("log.1");
    let _ = tokio::fs::remove_file(&rotated).await;
    let _ = tokio::fs::rename(path, rotated).await;
}

async fn append_log_line(path: &PathBuf, line: &str) {
    use tokio::io::AsyncWriteExt;
    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    if let Ok(mut file) = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
    {
        let _ = file.write_all(line.as_bytes()).await;
        let _ = file.write_all(b"\n").await;
    }
}
