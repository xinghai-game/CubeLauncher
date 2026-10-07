//! Tauri adapter: a thin, typed boundary between the web view and `launcher-core`.
//! Long operations run on background tasks and report through events, so the UI
//! never blocks and never needs filesystem or process permissions of its own.

use launcher_core::auth::MicrosoftAuth;
use launcher_core::instance::ModEntry;
use launcher_core::{
    Account, AppSettings, DeviceCodePrompt, GameDirScan, InstallTask, Instance, JavaRequirement,
    JavaRuntime, LaunchPreview, LauncherCore, Loader, LogLine, LoginPoll, Progress, VersionCatalog,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{Emitter, Manager, State};
use tokio::sync::{mpsc, Mutex, RwLock};

pub struct AppState {
    core: Arc<RwLock<LauncherCore>>,
    installations: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    /// Device code sign-ins in flight, keyed by the handle the UI polls with.
    logins: Arc<Mutex<HashMap<String, PendingLogin>>>,
    /// Skin textures already downloaded, as data URLs, keyed by account id.
    skins: Arc<Mutex<HashMap<String, String>>>,
}

/// One sign-in waiting for the user to finish on Microsoft's page. It keeps the
/// client it started with, so changing the setting mid-flight cannot strand it.
#[derive(Clone)]
struct PendingLogin {
    auth: MicrosoftAuth,
    device_code: String,
    interval: u64,
    expires_at: chrono::DateTime<chrono::Utc>,
}

impl AppState {
    async fn read(&self) -> tokio::sync::RwLockReadGuard<'_, LauncherCore> {
        self.core.read().await
    }
}

/// Identity status of a 正版 account, as the UI is allowed to see it. Tokens and
/// refresh tokens stay inside the core: the dialog needs a name, a head and the
/// expiry, not the credentials.
#[derive(Debug, Serialize)]
pub struct MicrosoftView {
    pub xuid: String,
    pub owns_java: bool,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Still usable right now; a launch refreshes it otherwise.
    pub token_valid: bool,
    pub skin_url: Option<String>,
    pub last_login: Option<chrono::DateTime<chrono::Utc>>,
    /// A refresh token is stored, so signing in again needs no user action.
    pub refreshable: bool,
}

#[derive(Debug, Serialize)]
pub struct AccountView {
    pub id: String,
    pub name: String,
    pub uuid: String,
    pub kind: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub microsoft: Option<MicrosoftView>,
}

impl From<&Account> for AccountView {
    fn from(account: &Account) -> Self {
        Self {
            id: account.id.clone(),
            name: account.name.clone(),
            uuid: account.uuid.clone(),
            kind: account.kind.clone(),
            created_at: account.created_at,
            microsoft: account.microsoft.as_ref().map(|microsoft| MicrosoftView {
                xuid: microsoft.xuid.clone(),
                owns_java: microsoft.owns_java,
                expires_at: microsoft.access_expires_at,
                token_valid: MicrosoftAuth::token_valid(account),
                skin_url: microsoft.skin_url.clone(),
                last_login: microsoft.last_login,
                refreshable: !microsoft.refresh_token.is_empty(),
            }),
        }
    }
}

async fn accounts_view(core: &LauncherCore) -> Result<Vec<AccountView>, String> {
    Ok(core
        .accounts()
        .await
        .map_err(err)?
        .iter()
        .map(AccountView::from)
        .collect())
}

#[derive(Debug, Serialize)]
pub struct Bootstrap {
    pub settings: AppSettings,
    pub accounts: Vec<AccountView>,
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

/// Import request: a version that already exists inside someone's `.minecraft`.
#[derive(Debug, Deserialize)]
pub struct ImportInstance {
    pub name: String,
    pub path: String,
    pub version_id: String,
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
        accounts: accounts_view(&core).await?,
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
) -> Result<AccountView, String> {
    let account = state
        .read()
        .await
        .add_offline_account(&name)
        .await
        .map_err(err)?;
    Ok(AccountView::from(&account))
}

#[tauri::command]
async fn delete_account(state: State<'_, AppState>, id: String) -> Result<CommandResult, String> {
    state.read().await.delete_account(&id).await.map_err(err)?;
    Ok(CommandResult {
        ok: true,
        message: "角色已删除".into(),
    })
}

/* ------------------------------------------------------------------ 正版登录 */

/// One poll of a pending sign-in. The state machine lives in the core; this is
/// just the wire shape the dialog needs: a state, an optional account, an
/// optional message, and how long to wait before asking again.
#[derive(Debug, Serialize)]
pub struct LoginStatus {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<AccountView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval: Option<u64>,
}

fn login_status(poll: LoginPoll, interval: Option<u64>) -> LoginStatus {
    let (state, account, message) = match poll {
        LoginPoll::Pending => ("pending", None, None),
        LoginPoll::SlowDown => ("slow_down", None, None),
        LoginPoll::Ready { account } => ("ready", Some(AccountView::from(account.as_ref())), None),
        LoginPoll::Expired => ("expired", None, None),
        LoginPoll::Declined => ("declined", None, None),
        LoginPoll::Failed { message } => ("failed", None, Some(message)),
    };
    LoginStatus {
        state: state.into(),
        account,
        message,
        interval,
    }
}

/// Ask Microsoft for a device code. The outcome is polled, not pushed: the
/// dialog owns the loop so a closed window cannot leave a task behind.
#[tauri::command]
async fn start_microsoft_login(state: State<'_, AppState>) -> Result<DeviceCodePrompt, String> {
    let auth = state.read().await.auth.clone();
    let prompt = auth.request_device_code().await.map_err(err)?;
    let mut logins = state.logins.lock().await;
    // Abandoned sign-ins never come back, so drop them instead of growing.
    let now = chrono::Utc::now();
    logins.retain(|_, pending| pending.expires_at > now);
    logins.insert(
        prompt.login_id.clone(),
        PendingLogin {
            auth,
            device_code: prompt.login_id.clone(),
            interval: prompt.interval,
            expires_at: now + chrono::Duration::seconds(prompt.expires_in as i64),
        },
    );
    Ok(prompt)
}

#[tauri::command]
async fn poll_microsoft_login(
    state: State<'_, AppState>,
    login_id: String,
) -> Result<LoginStatus, String> {
    let pending = state.logins.lock().await.get(&login_id).cloned();
    let Some(pending) = pending else {
        // Unknown handle: the dialog was reopened after a restart.
        return Ok(login_status(LoginPoll::Expired, None));
    };
    if chrono::Utc::now() >= pending.expires_at {
        state.logins.lock().await.remove(&login_id);
        return Ok(login_status(LoginPoll::Expired, None));
    }
    let poll = pending
        .auth
        .poll_device_code(&pending.device_code)
        .await
        .map_err(err)?;
    match &poll {
        LoginPoll::Pending => Ok(login_status(poll, Some(pending.interval))),
        LoginPoll::SlowDown => {
            // Microsoft asked for room; add five seconds and remember it.
            let interval = pending.interval + 5;
            if let Some(entry) = state.logins.lock().await.get_mut(&login_id) {
                entry.interval = interval;
            }
            Ok(login_status(poll, Some(interval)))
        }
        LoginPoll::Ready { account } => {
            state
                .read()
                .await
                .upsert_account((**account).clone())
                .await
                .map_err(err)?;
            state.logins.lock().await.remove(&login_id);
            state.skins.lock().await.clear();
            Ok(login_status(poll, None))
        }
        _ => {
            state.logins.lock().await.remove(&login_id);
            Ok(login_status(poll, None))
        }
    }
}

#[tauri::command]
async fn cancel_microsoft_login(
    state: State<'_, AppState>,
    login_id: String,
) -> Result<(), String> {
    state.logins.lock().await.remove(&login_id);
    Ok(())
}

/// Sign in again with the stored refresh token, e.g. after the Minecraft token
/// expired while the launcher was closed.
#[tauri::command]
async fn refresh_account(state: State<'_, AppState>, id: String) -> Result<AccountView, String> {
    let core = state.read().await;
    let account = core
        .account(&id)
        .await
        .ok_or_else(|| "找不到该角色".to_string())?;
    if !account.is_microsoft() {
        return Err("离线角色没有需要刷新的登录".into());
    }
    let refreshed = core.auth.refresh(&account).await.map_err(err)?;
    core.upsert_account(refreshed.clone()).await.map_err(err)?;
    Ok(AccountView::from(&refreshed))
}

/// Official skin texture as a data URL, or `None` when there is nothing to
/// show. Decoration never fails a command: a broken skin just falls back to the
/// letter avatar in the UI.
#[tauri::command]
async fn account_skin(state: State<'_, AppState>, id: String) -> Result<Option<String>, String> {
    let (auth, url) = {
        let core = state.read().await;
        let account = core
            .account(&id)
            .await
            .ok_or_else(|| "找不到该角色".to_string())?;
        let url = account
            .microsoft
            .as_ref()
            .and_then(|microsoft| microsoft.skin_url.clone());
        (core.auth.clone(), url)
    };
    let Some(url) = url else { return Ok(None) };
    if let Some(cached) = state.skins.lock().await.get(&id) {
        return Ok(Some(cached.clone()));
    }
    match auth.skin_texture(&url).await {
        Ok(data) => {
            state.skins.lock().await.insert(id, data.clone());
            Ok(Some(data))
        }
        Err(error) => {
            tracing::warn!("下载皮肤失败，账户头像将回退为文字：{error:#}");
            Ok(None)
        }
    }
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
    let mut progress = Progress::new(Some(sender.clone()), instance_id.clone());
    progress.task_id = task_id.clone();
    let installations = state.installations.clone();
    {
        let mut active = installations.lock().await;
        if active.contains_key(&instance_id) {
            return Err("该实例正在安装，请等待完成或暂停后再继续".into());
        }
        active.insert(instance_id.clone(), progress.cancel.clone());
    }

    let event_app = app.clone();
    let event_task = task_id.clone();
    tauri::async_runtime::spawn(async move {
        // Hold the terminal event until all writers have stopped and the active
        // registration is removed, so clicking Continue cannot race old writes.
        let mut terminal: Option<InstallTask> = None;
        while let Some(mut task) = receiver.recv().await {
            task.id = event_task.clone();
            if task.done {
                terminal = Some(task);
            } else {
                let _ = event_app.emit("task_updated", task);
            }
        }
        if let Some(task) = terminal {
            let state = match task.phase.as_str() {
                "done" => "installed",
                "cancelled" => "cancelled",
                _ => "error",
            };
            let _ = event_app.emit("task_updated", &task);
            let _ = event_app.emit(
                "instance_state_changed",
                InstanceState {
                    instance_id: task.instance_id,
                    state: state.into(),
                    exit_code: None,
                },
            );
        }
    });

    tauri::async_runtime::spawn(async move {
        {
            let core = core_handle.read().await;
            let result = core.install_instance(&instance, progress.clone()).await;
            if let Err(error) = result {
                if progress.cancelled() {
                    let _ = sender
                        .send(InstallTask {
                            id: progress.task_id.clone(),
                            instance_id: instance.id.clone(),
                            phase: "cancelled".into(),
                            current: 0,
                            total: 0,
                            message: "安装已暂停，已下载的数据会在继续安装时复用".into(),
                            done: true,
                            error: None,
                            downloaded_bytes: 0,
                            total_bytes: 0,
                            active: Vec::new(),
                        })
                        .await;
                } else {
                    progress.fail(format!("{error:#}")).await;
                }
            }
        }
        installations.lock().await.remove(&instance.id);
    });
    Ok(task_id)
}

#[tauri::command]
async fn cancel_install(state: State<'_, AppState>, instance_id: String) -> Result<(), String> {
    let active = state.installations.lock().await;
    if let Some(cancel) = active.get(&instance_id) {
        cancel.store(true, Ordering::Relaxed);
    }
    Ok(())
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

/// The `.minecraft` an instance plays in, imported or launcher-managed.
#[tauri::command]
async fn game_dir(state: State<'_, AppState>, instance_id: String) -> Result<String, String> {
    let core = state.read().await;
    let instance = core.instance(&instance_id).await.map_err(err)?;
    Ok(core.game_dir(&instance).display().to_string())
}

/// Look inside a directory the user picked, to show what can be imported.
#[tauri::command]
async fn scan_game_dir(state: State<'_, AppState>, path: String) -> Result<GameDirScan, String> {
    state
        .read()
        .await
        .scan_game_dir(Path::new(&path))
        .await
        .map_err(err)
}

/// Register an existing installation as an instance, without copying anything.
#[tauri::command]
async fn import_instance(
    state: State<'_, AppState>,
    input: ImportInstance,
) -> Result<Instance, String> {
    state
        .read()
        .await
        .import_instance(
            &input.name,
            Path::new(&input.path),
            &input.version_id,
            input.account_id,
        )
        .await
        .map_err(err)
}

/// Point an instance at another game directory, or back to the managed one when
/// `path` is `null`.
#[tauri::command]
async fn bind_instance_game_dir(
    state: State<'_, AppState>,
    instance_id: String,
    path: Option<String>,
    version_id: Option<String>,
) -> Result<Instance, String> {
    let core = state.read().await;
    core.bind_game_dir(
        &instance_id,
        path.as_deref().map(Path::new),
        version_id.as_deref(),
    )
    .await
    .map_err(err)
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
    let result = core.install_java(major, &arch, &progress).await;
    drop(core);
    // A download that only ever reports progress leaves the UI holding a task
    // that never finishes, which silently disables the install buttons until the
    // launcher is restarted. Always publish a terminal state.
    match result {
        Ok(runtime) => {
            progress
                .finish(format!("Java {} 运行时已就绪", runtime.major))
                .await;
            Ok(runtime)
        }
        Err(error) => {
            progress.fail(format!("{error:#}")).await;
            Err(err(error))
        }
    }
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
        accounts: accounts_view(&core).await?,
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
                    installations: Arc::new(Mutex::new(HashMap::new())),
                    logins: Arc::new(Mutex::new(HashMap::new())),
                    skins: Arc::new(Mutex::new(HashMap::new())),
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
            start_microsoft_login,
            poll_microsoft_login,
            cancel_microsoft_login,
            refresh_account,
            account_skin,
            create_instance,
            update_instance,
            delete_instance,
            install_instance,
            cancel_install,
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
            game_dir,
            scan_game_dir,
            import_instance,
            bind_instance_game_dir,
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

#[cfg(test)]
mod tests {
    use super::*;
    use launcher_core::MicrosoftAccount;

    fn microsoft_account() -> Account {
        Account {
            id: "069a79f444e94726a5befca90e38aaf5".into(),
            name: "Steve".into(),
            uuid: "069a79f4-44e9-4726-a5be-fca90e38aaf5".into(),
            kind: "microsoft".into(),
            created_at: chrono::Utc::now(),
            microsoft: Some(MicrosoftAccount {
                xuid: "2535412345678901".into(),
                refresh_token: "secret-refresh-token".into(),
                access_token: "secret-access-token".into(),
                access_expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(20)),
                skin_url: Some("http://textures.minecraft.net/texture/abc".into()),
                owns_java: true,
                last_login: Some(chrono::Utc::now()),
            }),
        }
    }

    /// The web view renders accounts; it must never be handed a credential.
    #[test]
    fn account_view_redacts_tokens() {
        let json = serde_json::to_string(&AccountView::from(&microsoft_account())).unwrap();
        assert!(!json.contains("secret"), "{json}");
        assert!(!json.contains("access_token"), "{json}");
        assert!(!json.contains("refresh_token"), "{json}");
        assert!(json.contains("\"refreshable\":true"), "{json}");
        assert!(json.contains("\"token_valid\":true"), "{json}");
        assert!(json.contains("\"owns_java\":true"), "{json}");
        assert!(json.contains("textures.minecraft.net"), "{json}");
    }

    /// An account whose token has expired still reports itself as renewable.
    #[test]
    fn expired_tokens_are_flagged_as_needing_a_refresh() {
        let mut account = microsoft_account();
        let microsoft = account.microsoft.as_mut().expect("microsoft block");
        microsoft.access_expires_at = Some(chrono::Utc::now() - chrono::Duration::hours(2));
        let view = AccountView::from(&account);
        let microsoft = view.microsoft.expect("view");
        assert!(!microsoft.token_valid);
        assert!(microsoft.refreshable);
    }

    /// An offline role has no 正版 block at all, so the UI cannot mislabel it.
    #[test]
    fn offline_roles_have_no_microsoft_block() {
        let account = Account {
            id: "abc".into(),
            name: "Alice".into(),
            uuid: "abc".into(),
            kind: "offline".into(),
            created_at: chrono::Utc::now(),
            microsoft: None,
        };
        assert!(AccountView::from(&account).microsoft.is_none());
    }
}
