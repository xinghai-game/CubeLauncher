//! Tauri adapter: a thin, typed boundary between the web view and `launcher-core`.
//! Long operations run on background tasks and report through events, so the UI
//! never blocks and never needs filesystem or process permissions of its own.

use launcher_core::instance::ModEntry;
use launcher_core::{
    AppSettings, InstallTask, Instance, JavaRequirement, JavaRuntime, LaunchPreview, LauncherCore,
    Loader, LogLine, OfflineAccount, Progress, VersionCatalog,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{Emitter, Manager, State};
use tokio::sync::{mpsc, RwLock};

pub struct AppState {
    core: Arc<RwLock<LauncherCore>>,
}

impl AppState {
    async fn read(&self) -> tokio::sync::RwLockReadGuard<'_, LauncherCore> {
        self.core.read().await
    }
}

#[derive(Debug, Serialize)]
pub struct Bootstrap {
    pub settings: AppSettings,
    pub accounts: Vec<OfflineAccount>,
    pub instances: Vec<Instance>,
    pub java: Vec<JavaRuntime>,
    pub running: Vec<String>,
    pub data_dir: String,
}

#[derive(Debug, Deserialize)]
pub struct NewInstance {
    pub name: String,
    pub game_version: String,
    pub loader: Loader,
    #[serde(default)]
    pub loader_version: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CommandResult {
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstanceState {
    pub instance_id: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

fn err(error: anyhow::Error) -> String {
    format!("{error:#}")
}

#[tauri::command]
async fn bootstrap(state: State<'_, AppState>) -> Result<Bootstrap, String> {
    let core = state.read().await;
    Ok(Bootstrap {
        settings: core.settings.clone(),
        accounts: core.accounts().await.map_err(err)?,
        instances: core.instances().await.map_err(err)?,
        java: core.discover_java().await,
        running: core.processes().running_ids().await,
        data_dir: core.paths.root.display().to_string(),
    })
}

/// Version list with categories and provenance: fresh cache, live fetch, or a
/// stale cache kept when the refresh failed.
#[tauri::command]
async fn version_catalog(
    state: State<'_, AppState>,
    force: bool,
) -> Result<VersionCatalog, String> {
    state.read().await.catalog(force).await.map_err(err)
}

/// Java major a version needs, from cached metadata when available.
#[tauri::command]
async fn version_java(
    state: State<'_, AppState>,
    version_id: String,
) -> Result<JavaRequirement, String> {
    Ok(state.read().await.required_java_major(&version_id).await)
}

#[tauri::command]
async fn loader_versions(
    state: State<'_, AppState>,
    game_version: String,
    loader: Loader,
) -> Result<Vec<String>, String> {
    let core = state.read().await;
    match loader {
        Loader::Vanilla => Ok(Vec::new()),
        Loader::Fabric => Ok(core
            .fabric_loader_versions(&game_version)
            .await
            .map_err(err)?
            .into_iter()
            .filter(|entry| entry.stable)
            .map(|entry| entry.version)
            .collect()),
        other => core
            .loader_versions(&game_version, other)
            .await
            .map_err(err),
    }
}

#[tauri::command]
async fn add_offline_account(
    state: State<'_, AppState>,
    name: String,
) -> Result<OfflineAccount, String> {
    state
        .read()
        .await
        .add_offline_account(&name)
        .await
        .map_err(err)
}

#[tauri::command]
async fn delete_account(state: State<'_, AppState>, id: String) -> Result<CommandResult, String> {
    state.read().await.delete_account(&id).await.map_err(err)?;
    Ok(CommandResult {
        ok: true,
        message: "角色已删除".into(),
    })
}

#[tauri::command]
async fn create_instance(
    state: State<'_, AppState>,
    input: NewInstance,
) -> Result<Instance, String> {
    state
        .read()
        .await
        .create_instance(
            &input.name,
            &input.game_version,
            input.loader,
            input.loader_version,
            input.account_id,
        )
        .await
        .map_err(err)
}

#[tauri::command]
async fn update_instance(
    state: State<'_, AppState>,
    instance: Instance,
) -> Result<Instance, String> {
    state
        .read()
        .await
        .update_instance(&instance)
        .await
        .map_err(err)?;
    Ok(instance)
}

#[tauri::command]
async fn delete_instance(state: State<'_, AppState>, id: String) -> Result<CommandResult, String> {
    state.read().await.delete_instance(&id).await.map_err(err)?;
    Ok(CommandResult {
        ok: true,
        message: "实例已删除".into(),
    })
}

/// Start an installation, reporting progress through `task_updated` events.
#[tauri::command]
async fn install_instance(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    instance_id: String,
) -> Result<String, String> {
    let core_handle = state.core.clone();
    let instance = core_handle
        .read()
        .await
        .instance(&instance_id)
        .await
        .map_err(err)?;
    let task_id = format!("install-{instance_id}");
    let (sender, mut receiver) = mpsc::channel::<InstallTask>(64);
    let mut progress = Progress::new(Some(sender), instance_id.clone());
    progress.task_id = task_id.clone();

    let event_app = app.clone();
    let event_task = task_id.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(mut task) = receiver.recv().await {
            task.id = event_task.clone();
            let _ = event_app.emit("task_updated", task);
        }
    });

    let worker_app = app.clone();
    let worker_task = task_id.clone();
    tauri::async_runtime::spawn(async move {
        {
            let core = core_handle.read().await;
            let result = core.install_instance(&instance, progress.clone()).await;
            if let Err(error) = result {
                let message = format!("{error:#}");
                let _ = progress.fail(message.clone()).await;
                let _ = worker_app.emit(
                    "task_updated",
                    InstallTask {
                        id: worker_task,
                        instance_id: instance.id.clone(),
                        phase: "error".into(),
                        current: 0,
                        total: 0,
                        message: message.clone(),
                        done: true,
                        error: Some(message),
                    },
                );
            }
        }
        let _ = worker_app.emit(
            "instance_state_changed",
            InstanceState {
                instance_id: instance.id.clone(),
                state: "installed".into(),
                exit_code: None,
            },
        );
    });
    Ok(task_id)
}

#[tauri::command]
async fn verify_instance(
    state: State<'_, AppState>,
    instance_id: String,
) -> Result<Vec<String>, String> {
    let core = state.read().await;
    let instance = core.instance(&instance_id).await.map_err(err)?;
    core.verify_instance(&instance).await.map_err(err)
}

#[tauri::command]
async fn repair_instance(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    instance_id: String,
) -> Result<String, String> {
    install_instance(app, state, instance_id).await
}

#[tauri::command]
async fn launch_preview(
    state: State<'_, AppState>,
    instance_id: String,
) -> Result<LaunchPreview, String> {
    let core = state.read().await;
    let instance = core.instance(&instance_id).await.map_err(err)?;
    core.launch_preview(&instance).await.map_err(err)
}

/// Launch a game process. Resolves when the process exits.
#[tauri::command]
async fn launch_instance(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    instance_id: String,
) -> Result<CommandResult, String> {
    let core_handle = state.core.clone();
    let instance = core_handle
        .read()
        .await
        .instance(&instance_id)
        .await
        .map_err(err)?;
    let (sender, mut receiver) = mpsc::channel::<LogLine>(256);
    let log_app = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(line) = receiver.recv().await {
            let _ = log_app.emit("log_batch", line);
        }
    });
    let _ = app.emit(
        "instance_state_changed",
        InstanceState {
            instance_id: instance_id.clone(),
            state: "starting".into(),
            exit_code: None,
        },
    );
    let core = core_handle.read().await;
    let result = core.launch_game(&instance, sender).await.map_err(err);
    drop(core);
    let (state_label, exit_code) = match &result {
        Ok(code) => ("stopped", Some(*code)),
        Err(_) => ("failed", None),
    };
    let _ = app.emit(
        "instance_state_changed",
        InstanceState {
            instance_id: instance_id.clone(),
            state: state_label.into(),
            exit_code,
        },
    );
    let code = result?;
    Ok(CommandResult {
        ok: code == 0,
        message: format!("游戏进程已退出（退出码 {code}）"),
    })
}

#[tauri::command]
async fn stop_instance(
    state: State<'_, AppState>,
    instance_id: String,
) -> Result<CommandResult, String> {
    state
        .read()
        .await
        .processes()
        .stop(&instance_id)
        .await
        .map_err(err)?;
    Ok(CommandResult {
        ok: true,
        message: "已请求停止游戏".into(),
    })
}

#[tauri::command]
async fn list_mods(
    state: State<'_, AppState>,
    instance_id: String,
) -> Result<Vec<ModEntry>, String> {
    state
        .read()
        .await
        .list_mods(&instance_id)
        .await
        .map_err(err)
}

#[tauri::command]
async fn add_mod(
    state: State<'_, AppState>,
    instance_id: String,
    path: String,
) -> Result<CommandResult, String> {
    let name = state
        .read()
        .await
        .add_mod(&instance_id, std::path::Path::new(&path))
        .await
        .map_err(err)?;
    Ok(CommandResult {
        ok: true,
        message: format!("已添加 {name}"),
    })
}

#[tauri::command]
async fn toggle_mod(
    state: State<'_, AppState>,
    instance_id: String,
    file_name: String,
    enabled: bool,
) -> Result<CommandResult, String> {
    state
        .read()
        .await
        .toggle_mod(&instance_id, &file_name, enabled)
        .await
        .map_err(err)?;
    Ok(CommandResult {
        ok: true,
        message: if enabled {
            "模组已启用".into()
        } else {
            "模组已停用".into()
        },
    })
}

#[tauri::command]
async fn delete_mod(
    state: State<'_, AppState>,
    instance_id: String,
    file_name: String,
) -> Result<CommandResult, String> {
    state
        .read()
        .await
        .delete_mod(&instance_id, &file_name)
        .await
        .map_err(err)?;
    Ok(CommandResult {
        ok: true,
        message: "模组已删除".into(),
    })
}

#[tauri::command]
async fn mods_dir(state: State<'_, AppState>, instance_id: String) -> Result<String, String> {
    Ok(state
        .read()
        .await
        .ensure_mods_dir(&instance_id)
        .await
        .map_err(err)?
        .display()
        .to_string())
}

#[tauri::command]
async fn instance_dir(state: State<'_, AppState>, instance_id: String) -> Result<String, String> {
    Ok(state
        .read()
        .await
        .paths
        .instance_dir(&instance_id)
        .display()
        .to_string())
}

#[tauri::command]
async fn read_log(
    state: State<'_, AppState>,
    instance_id: String,
    max_lines: Option<usize>,
) -> Result<Vec<String>, String> {
    state
        .read()
        .await
        .read_log(&instance_id, max_lines.unwrap_or(400))
        .await
        .map_err(err)
}

#[tauri::command]
async fn clear_log(
    state: State<'_, AppState>,
    instance_id: String,
) -> Result<CommandResult, String> {
    let path = state
        .read()
        .await
        .paths
        .logs
        .join(format!("{instance_id}.log"));
    let _ = tokio::fs::remove_file(path).await;
    Ok(CommandResult {
        ok: true,
        message: "日志已清空".into(),
    })
}

#[tauri::command]
async fn list_java(state: State<'_, AppState>) -> Result<Vec<JavaRuntime>, String> {
    Ok(state.read().await.discover_java().await)
}

#[tauri::command]
async fn install_java(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    major: u32,
) -> Result<JavaRuntime, String> {
    let core_handle = state.core.clone();
    let arch = launcher_core::rules::current_arch().to_string();
    let (sender, mut receiver) = mpsc::channel::<InstallTask>(32);
    let progress = Progress::new(Some(sender), "java");
    let event_app = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(task) = receiver.recv().await {
            let _ = event_app.emit("task_updated", task);
        }
    });
    let core = core_handle.read().await;
    core.install_java(major, &arch, &progress)
        .await
        .map_err(err)
}

#[tauri::command]
async fn save_settings(
    state: State<'_, AppState>,
    settings: AppSettings,
) -> Result<Bootstrap, String> {
    let mut core = state.core.write().await;
    core.apply_settings(settings).await.map_err(err)?;
    Ok(Bootstrap {
        settings: core.settings.clone(),
        accounts: core.accounts().await.map_err(err)?,
        instances: core.instances().await.map_err(err)?,
        java: core.discover_java().await,
        running: core.processes().running_ids().await,
        data_dir: core.paths.root.display().to_string(),
    })
}

#[tauri::command]
async fn quit(app: tauri::AppHandle) {
    app.exit(0);
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("CUBELAUNCHER_LOG").unwrap_or_else(|_| "cube_launcher=info".into()),
        )
        .init();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::block_on(async move {
                let settings = LauncherCore::load_settings(AppSettings::default()).await;
                let core = LauncherCore::new(settings)
                    .await
                    .expect("初始化启动器核心失败");
                handle.manage(AppState {
                    core: Arc::new(RwLock::new(core)),
                });
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            version_catalog,
            version_java,
            loader_versions,
            add_offline_account,
            delete_account,
            create_instance,
            update_instance,
            delete_instance,
            install_instance,
            repair_instance,
            verify_instance,
            launch_preview,
            launch_instance,
            stop_instance,
            list_mods,
            add_mod,
            toggle_mod,
            delete_mod,
            mods_dir,
            instance_dir,
            read_log,
            clear_log,
            list_java,
            install_java,
            save_settings,
            quit
        ])
        .run(tauri::generate_context!())
        .expect("启动 CubeLauncher 失败");
}
