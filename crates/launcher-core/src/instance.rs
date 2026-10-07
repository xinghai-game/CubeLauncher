use crate::types::*;
use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use std::path::{Path, PathBuf};

/// Validate a Minecraft-compatible offline user name.
pub fn validate_username(name: &str) -> Result<()> {
    let length = name.chars().count();
    if !(3..=16).contains(&length) {
        bail!("离线角色名需要 3–16 个字符");
    }
    if !name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        bail!("离线角色名只能包含英文字母、数字和下划线");
    }
    Ok(())
}

/// Deterministic offline UUID, matching Minecraft's
/// `UUID.nameUUIDFromBytes(("OfflinePlayer:" + name).getBytes(UTF_8))`.
/// That algorithm is MD5-based with the version and variant bits rewritten.
pub fn offline_uuid(name: &str) -> String {
    let mut bytes = md5::compute(format!("OfflinePlayer:{name}").as_bytes()).0;
    bytes[6] = (bytes[6] & 0x0f) | 0x30; // version 3
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // IETF variant
    format!(
        "{}-{}-{}-{}-{}",
        hex::encode(&bytes[0..4]),
        hex::encode(&bytes[4..6]),
        hex::encode(&bytes[6..8]),
        hex::encode(&bytes[8..10]),
        hex::encode(&bytes[10..16])
    )
}

/// Undashed form, which is what the game expects on the command line.
pub fn offline_uuid_compact(name: &str) -> String {
    offline_uuid(name).replace('-', "")
}

pub(crate) fn slug(value: &str) -> String {
    let slug: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        "instance".to_string()
    } else {
        trimmed.chars().take(32).collect()
    }
}

impl crate::LauncherCore {
    /// Every stored identity, offline roles and Microsoft accounts alike.
    /// Entries written by older versions (offline only) parse unchanged.
    pub async fn accounts(&self) -> Result<Vec<Account>> {
        let path = self.paths.root.join("accounts.json");
        let Ok(bytes) = tokio::fs::read(&path).await else {
            return Ok(Vec::new());
        };
        Ok(serde_json::from_slice(&bytes).unwrap_or_default())
    }

    /// The account file holds refresh tokens, so it is written owner-only.
    async fn write_accounts(&self, accounts: &[Account]) -> Result<()> {
        crate::atomic_write_private(
            &self.paths.root.join("accounts.json"),
            &serde_json::to_vec_pretty(accounts)?,
        )
        .await
    }

    pub async fn account(&self, id: &str) -> Option<Account> {
        self.accounts()
            .await
            .ok()?
            .into_iter()
            .find(|account| account.id == id)
    }

    pub async fn add_offline_account(&self, name: &str) -> Result<Account> {
        let name = name.trim();
        validate_username(name)?;
        let mut accounts = self.accounts().await?;
        if accounts
            .iter()
            .any(|account| !account.is_microsoft() && account.name == name)
        {
            bail!("离线角色 “{name}” 已经存在");
        }
        let account = Account {
            id: offline_uuid(name),
            name: name.to_string(),
            uuid: offline_uuid(name),
            kind: "offline".to_string(),
            created_at: Utc::now(),
            microsoft: None,
        };
        accounts.push(account.clone());
        self.write_accounts(&accounts).await?;
        Ok(account)
    }

    /// Store a Microsoft account, replacing the entry for the same UUID. This is
    /// both "add account" and "sign in again", which is what makes a repeated
    /// login converge instead of piling up duplicates.
    pub async fn upsert_account(&self, account: Account) -> Result<Account> {
        let mut accounts = self.accounts().await?;
        match accounts.iter().position(|entry| entry.id == account.id) {
            Some(index) => accounts[index] = account.clone(),
            None => accounts.push(account.clone()),
        }
        self.write_accounts(&accounts).await?;
        Ok(account)
    }

    /// Keep the stored tokens in step with a refreshed account. A missing entry
    /// is not an error: the account may have been deleted while it was used.
    pub async fn update_account(&self, account: &Account) -> Result<()> {
        let mut accounts = self.accounts().await?;
        let Some(entry) = accounts.iter_mut().find(|entry| entry.id == account.id) else {
            return Ok(());
        };
        *entry = account.clone();
        self.write_accounts(&accounts).await
    }

    pub async fn delete_account(&self, id: &str) -> Result<()> {
        let mut accounts = self.accounts().await?;
        let before = accounts.len();
        accounts.retain(|account| account.id != id);
        if accounts.len() == before {
            bail!("找不到该角色");
        }
        self.write_accounts(&accounts).await
    }

    pub async fn instances(&self) -> Result<Vec<Instance>> {
        let mut out = Vec::new();
        let mut entries = match tokio::fs::read_dir(&self.paths.instances).await {
            Ok(entries) => entries,
            Err(_) => return Ok(out),
        };
        while let Some(entry) = entries.next_entry().await? {
            if !entry.file_type().await?.is_dir() {
                continue;
            }
            if let Ok(bytes) = tokio::fs::read(entry.path().join("instance.json")).await {
                if let Ok(instance) = serde_json::from_slice::<Instance>(&bytes) {
                    out.push(instance);
                }
            }
        }
        out.sort_by(|left, right| {
            right
                .last_played
                .cmp(&left.last_played)
                .then_with(|| left.name.cmp(&right.name))
        });
        Ok(out)
    }

    pub async fn create_instance(
        &self,
        name: &str,
        game_version: &str,
        loader: Loader,
        loader_version: Option<String>,
        account_id: Option<String>,
    ) -> Result<Instance> {
        let name = name.trim();
        if name.is_empty() {
            bail!("实例名称不能为空");
        }
        if game_version.trim().is_empty() {
            bail!("请选择游戏版本");
        }
        let existing = self.instances().await?;
        if existing.iter().any(|instance| instance.name == name) {
            bail!("已经有一个叫 “{name}” 的实例");
        }
        let mut id = slug(name);
        if existing.iter().any(|instance| instance.id == id) {
            id = format!("{}-{}", id, &offline_uuid(name)[..8]);
        }
        let instance = Instance {
            id: id.clone(),
            name: name.to_string(),
            game_version: game_version.to_string(),
            loader,
            version_id: None,
            game_dir: None,
            loader_version,
            account_id,
            java_path: self.settings.default_java.clone(),
            min_memory_mb: 1024,
            max_memory_mb: self.settings.default_memory_mb,
            jvm_args: Vec::new(),
            game_args: Vec::new(),
            width: Some(1280),
            height: Some(720),
            fullscreen: false,
            installed: false,
            last_played: None,
            created_at: Utc::now(),
        };
        let dir = self.paths.instance_dir(&id);
        for path in [
            dir.clone(),
            dir.join(".minecraft"),
            dir.join(".minecraft").join("mods"),
            dir.join(".minecraft").join("shaderpacks"),
            dir.join(".minecraft").join("schematics"),
            dir.join(".minecraft").join("config"),
            dir.join("natives"),
            dir.join("logs"),
        ] {
            tokio::fs::create_dir_all(path).await?;
        }
        self.update_instance(&instance).await?;
        Ok(instance)
    }

    pub async fn update_instance(&self, instance: &Instance) -> Result<()> {
        if instance.min_memory_mb > instance.max_memory_mb {
            bail!("最小内存不能大于最大内存");
        }
        if instance.max_memory_mb < 512 {
            bail!("最大内存至少需要 512 MB");
        }
        crate::atomic_write(
            &self.paths.instance_dir(&instance.id).join("instance.json"),
            &serde_json::to_vec_pretty(instance)?,
        )
        .await
    }

    /// Delete an instance directory. The caller must confirm in the UI, because this
    /// removes saves and mods that live inside the instance. An imported `.minecraft`
    /// lives outside that directory and is therefore never touched.
    pub async fn delete_instance(&self, id: &str) -> Result<()> {
        if self.processes().is_running(id).await {
            bail!("实例正在运行，请先退出游戏");
        }
        let dir = self.paths.instance_dir(id);
        if dir.is_dir() {
            tokio::fs::remove_dir_all(&dir)
                .await
                .with_context(|| format!("删除实例目录失败：{}", dir.display()))?;
        }
        Ok(())
    }

    /// Resolve one managed resource folder inside the instance game directory.
    pub fn resource_dir(&self, instance: &Instance, kind: ResourceKind) -> PathBuf {
        self.game_dir(instance).join(kind.directory())
    }

    pub async fn ensure_resource_dir(&self, id: &str, kind: ResourceKind) -> Result<PathBuf> {
        let dir = self.resource_dir(&self.instance(id).await?, kind);
        tokio::fs::create_dir_all(&dir).await?;
        Ok(dir)
    }

    /// List files for one resource type. Disabled files keep the same extension
    /// followed by `.disabled`, so the game will ignore them while the UI can
    /// restore them with one click.
    pub async fn list_resources(&self, id: &str, kind: ResourceKind) -> Result<Vec<ResourceEntry>> {
        let dir = self.ensure_resource_dir(id, kind).await?;
        let mut out = Vec::new();
        let mut entries = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if !entry.file_type().await?.is_file() {
                continue;
            }
            let file_name = entry.file_name().to_string_lossy().to_string();
            let (base_name, enabled) = resource_base_name(&file_name);
            if !kind.accepts(base_name) {
                continue;
            }
            let metadata = entry.metadata().await?;
            out.push(ResourceEntry {
                file_name: file_name.clone(),
                enabled,
                size: metadata.len(),
                modified: metadata.modified().ok().map(chrono::DateTime::<Utc>::from),
                display_name: resource_display_name(base_name),
            });
        }
        out.sort_by(|left, right| left.display_name.cmp(&right.display_name));
        Ok(out)
    }

    pub async fn add_resource(
        &self,
        id: &str,
        kind: ResourceKind,
        source: &Path,
    ) -> Result<String> {
        if !source.is_file() {
            bail!("找不到{}文件：{}", kind.label(), source.display());
        }
        let file_name = source
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .ok_or_else(|| anyhow!("{}文件名无效", kind.label()))?;
        if !kind.accepts(&file_name) {
            bail!("{}文件格式不受支持", kind.label());
        }
        let dir = self.ensure_resource_dir(id, kind).await?;
        let destination = dir.join(&file_name);
        if destination.exists() {
            bail!("{}目录中已经有 {file_name}", kind.label());
        }
        tokio::fs::copy(source, &destination).await?;
        Ok(file_name)
    }

    pub async fn toggle_resource(
        &self,
        id: &str,
        kind: ResourceKind,
        file_name: &str,
        enabled: bool,
    ) -> Result<String> {
        let dir = self.ensure_resource_dir(id, kind).await?;
        validate_file_name(file_name)?;
        let base = resource_base_name(file_name).0;
        if base.is_empty() || !kind.accepts(base) {
            bail!("只能启用或停用有效的{}文件", kind.label());
        }
        let (from, to) = if enabled {
            (dir.join(format!("{base}.disabled")), dir.join(base))
        } else {
            (dir.join(base), dir.join(format!("{base}.disabled")))
        };
        if !from.is_file() {
            if to.is_file() {
                return Ok(to
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_default());
            }
            bail!("找不到{}文件：{}", kind.label(), from.display());
        }
        tokio::fs::rename(&from, &to).await?;
        Ok(to
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default())
    }

    pub async fn delete_resource(
        &self,
        id: &str,
        kind: ResourceKind,
        file_name: &str,
    ) -> Result<()> {
        let dir = self.ensure_resource_dir(id, kind).await?;
        validate_file_name(file_name)?;
        let path = dir.join(file_name);
        if !path.is_file() {
            bail!("找不到{}文件：{}", kind.label(), path.display());
        }
        tokio::fs::remove_file(path).await?;
        Ok(())
    }

    // Compatibility helpers used by the existing CLI smoke example and callers.
    pub fn mods_dir(&self, instance: &Instance) -> PathBuf {
        self.resource_dir(instance, ResourceKind::Mods)
    }

    pub async fn ensure_mods_dir(&self, id: &str) -> Result<PathBuf> {
        self.ensure_resource_dir(id, ResourceKind::Mods).await
    }

    pub async fn list_mods(&self, id: &str) -> Result<Vec<ModEntry>> {
        self.list_resources(id, ResourceKind::Mods).await
    }

    pub async fn add_mod(&self, id: &str, source: &Path) -> Result<String> {
        self.add_resource(id, ResourceKind::Mods, source).await
    }

    pub async fn toggle_mod(&self, id: &str, file_name: &str, enabled: bool) -> Result<String> {
        self.toggle_resource(id, ResourceKind::Mods, file_name, enabled)
            .await
    }

    pub async fn delete_mod(&self, id: &str, file_name: &str) -> Result<()> {
        self.delete_resource(id, ResourceKind::Mods, file_name)
            .await
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    Mods,
    Shaders,
    Projections,
}

impl ResourceKind {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "mods" => Ok(Self::Mods),
            "shaders" => Ok(Self::Shaders),
            "projections" => Ok(Self::Projections),
            _ => bail!("未知资源类型：{value}"),
        }
    }

    fn directory(self) -> &'static str {
        match self {
            Self::Mods => "mods",
            Self::Shaders => "shaderpacks",
            Self::Projections => "schematics",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Mods => "模组",
            Self::Shaders => "光影",
            Self::Projections => "投影",
        }
    }

    fn accepts(self, file_name: &str) -> bool {
        let lower = file_name.to_ascii_lowercase();
        match self {
            Self::Mods => lower.ends_with(".jar"),
            Self::Shaders => lower.ends_with(".zip") || lower.ends_with(".jar"),
            Self::Projections => {
                lower.ends_with(".litematic")
                    || lower.ends_with(".schematic")
                    || lower.ends_with(".schem")
            }
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ResourceEntry {
    pub file_name: String,
    pub display_name: String,
    pub enabled: bool,
    pub size: u64,
    pub modified: Option<chrono::DateTime<Utc>>,
}

pub type ModEntry = ResourceEntry;

fn resource_base_name(file_name: &str) -> (&str, bool) {
    if file_name.to_ascii_lowercase().ends_with(".disabled") {
        (&file_name[..file_name.len() - ".disabled".len()], false)
    } else {
        (file_name, true)
    }
}

fn resource_display_name(file_name: &str) -> String {
    file_name
        .rsplit_once('.')
        .map(|(name, _)| name)
        .unwrap_or(file_name)
        .to_string()
}
fn validate_file_name(file_name: &str) -> Result<()> {
    if file_name.is_empty()
        || file_name.contains('/')
        || file_name.contains('\\')
        || file_name.contains("..")
    {
        bail!("文件名不合法");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_kinds_use_expected_directories_and_extensions() {
        assert_eq!(ResourceKind::parse("mods").unwrap().directory(), "mods");
        assert_eq!(
            ResourceKind::parse("shaders").unwrap().directory(),
            "shaderpacks"
        );
        assert_eq!(
            ResourceKind::parse("projections").unwrap().directory(),
            "schematics"
        );
        assert!(ResourceKind::Mods.accepts("create.jar"));
        assert!(ResourceKind::Shaders.accepts("complementary.ZIP"));
        assert!(ResourceKind::Projections.accepts("base.litematic"));
        assert!(!ResourceKind::Projections.accepts("base.jar"));
        assert!(ResourceKind::parse("unknown").is_err());
    }

    #[test]
    fn resource_names_keep_disabled_state_and_strip_only_extension() {
        assert_eq!(resource_base_name("create.jar"), ("create.jar", true));
        assert_eq!(
            resource_base_name("create.jar.disabled"),
            ("create.jar", false)
        );
        assert_eq!(resource_display_name("my-shader.zip"), "my-shader");
        assert_eq!(resource_display_name("city.schematic"), "city");
    }

    #[test]
    fn offline_uuid_is_stable_and_versioned() {
        assert_eq!(offline_uuid("Steve"), offline_uuid("Steve"));
        assert_ne!(offline_uuid("Steve"), offline_uuid("steve"));
        let uuid = offline_uuid("Notch");
        assert_eq!(uuid.len(), 36);
        assert_eq!(&uuid[14..15], "3"); // MD5-based version 3
        assert!(
            ["8", "9", "a", "b"].contains(&&uuid[19..20]),
            "variant nibble must follow RFC 4122: {uuid}"
        );
    }

    #[test]
    fn offline_uuid_matches_java_reference_values() {
        // Generated with OpenJDK: UUID.nameUUIDFromBytes(("OfflinePlayer:"+name).getBytes(UTF_8))
        for (name, expected) in [
            ("Notch", "b50ad385-829d-3141-a216-7e7d7539ba7f"),
            ("Steve", "5627dd98-e6be-3c21-b8a8-e92344183641"),
            ("Alex", "36532b5e-c442-3dbb-a24c-c7e55d0f979a"),
            ("jeb_", "a762f560-4fce-3236-812a-b80efff0b62b"),
            ("Player", "a01e3843-e521-3998-958a-f459800e4d11"),
            ("abc", "3d5cec06-bd15-31fa-982f-5dac8c06f1c7"),
        ] {
            assert_eq!(offline_uuid(name), expected, "mismatch for {name}");
        }
    }

    #[test]
    fn username_rules_are_enforced() {
        assert!(validate_username("Steve").is_ok());
        assert!(validate_username("Steve_123").is_ok());
        assert!(validate_username("ab").is_err());
        assert!(validate_username("has space").is_err());
        assert!(validate_username("中文名").is_err());
        assert!(validate_username("aaaaaaaaaaaaaaaaa").is_err());
    }

    #[test]
    fn slug_handles_unicode_and_spaces() {
        assert_eq!(slug("My World"), "my-world");
        assert_eq!(slug("暮色草原"), "instance");
        assert_eq!(slug("  trim  "), "trim");
    }

    /* ------------------------------------------------------------ accounts */

    /// A core with its own data directory, plus a cleanup on drop.
    struct TestDir(std::path::PathBuf);
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    async fn core_with_data_dir(label: &str) -> (crate::LauncherCore, TestDir) {
        let dir = std::env::temp_dir().join(format!(
            "cube-account-{label}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let settings = AppSettings {
            data_dir: dir.clone(),
            ..Default::default()
        };
        let core = crate::LauncherCore::new(settings).await.expect("core");
        (core, TestDir(dir))
    }

    fn microsoft_account(id: &str, name: &str) -> Account {
        Account {
            id: id.to_string(),
            name: name.to_string(),
            uuid: crate::auth::dashed_uuid(id),
            kind: "microsoft".into(),
            created_at: Utc::now(),
            microsoft: Some(MicrosoftAccount {
                xuid: "2535412345678901".into(),
                refresh_token: format!("refresh-{name}"),
                access_token: format!("access-{name}"),
                access_expires_at: Some(Utc::now() + chrono::Duration::hours(12)),
                skin_url: None,
                owns_java: true,
                last_login: Some(Utc::now()),
            }),
        }
    }

    #[tokio::test]
    async fn accounts_survive_a_round_trip_and_are_deleted_by_id() {
        let (core, _dir) = core_with_data_dir("round-trip").await;
        let offline = core.add_offline_account("Alice").await.unwrap();
        assert_eq!(offline.kind, "offline");
        assert!(offline.microsoft.is_none());
        assert!(
            core.add_offline_account("Alice").await.is_err(),
            "duplicate name"
        );
        assert!(core.add_offline_account("a").await.is_err(), "too short");

        let id = "069a79f444e94726a5befca90e38aaf5";
        core.upsert_account(microsoft_account(id, "Steve"))
            .await
            .unwrap();
        let accounts = core.accounts().await.unwrap();
        assert_eq!(accounts.len(), 2);
        assert!(accounts.iter().any(|account| account.is_microsoft()));

        core.delete_account(id).await.unwrap();
        assert_eq!(core.accounts().await.unwrap().len(), 1);
        assert!(core.delete_account(id).await.is_err(), "already gone");
    }

    #[tokio::test]
    async fn signing_in_again_updates_instead_of_duplicating() {
        let (core, _dir) = core_with_data_dir("upsert").await;
        let id = "069a79f444e94726a5befca90e38aaf5";
        core.upsert_account(microsoft_account(id, "Steve"))
            .await
            .unwrap();
        let mut renamed = microsoft_account(id, "Alex");
        renamed.microsoft.as_mut().unwrap().access_token = "access-alex".into();
        core.upsert_account(renamed).await.unwrap();

        let accounts = core.accounts().await.unwrap();
        assert_eq!(accounts.len(), 1, "same UUID means same account");
        assert_eq!(accounts[0].name, "Alex");
        assert_eq!(
            accounts[0].microsoft.as_ref().unwrap().access_token,
            "access-alex"
        );
    }

    #[tokio::test]
    async fn refreshed_tokens_are_written_back() {
        let (core, _dir) = core_with_data_dir("refresh").await;
        let id = "069a79f444e94726a5befca90e38aaf5";
        let mut account = microsoft_account(id, "Steve");
        core.upsert_account(account.clone()).await.unwrap();
        account.microsoft.as_mut().unwrap().access_token = "access-new".into();
        core.update_account(&account).await.unwrap();
        assert_eq!(
            core.account(id)
                .await
                .unwrap()
                .microsoft
                .unwrap()
                .access_token,
            "access-new"
        );
        // Updating an account that is gone is a no-op, not an error: it may have
        // been deleted while it was being refreshed.
        core.delete_account(id).await.unwrap();
        core.update_account(&account).await.unwrap();
    }

    /// Files written by the offline-only versions of the launcher must keep
    /// loading, and the file must not become world-readable once it holds a
    /// refresh token.
    #[tokio::test]
    async fn account_file_migrates_and_stays_private() {
        let (core, dir) = core_with_data_dir("private").await;
        let legacy = format!(
            r#"[{{"id":"{0}","name":"Alice","uuid":"{0}","kind":"offline","created_at":"2026-01-01T00:00:00Z"}}]"#,
            offline_uuid("Alice")
        );
        tokio::fs::write(dir.0.join("accounts.json"), legacy)
            .await
            .unwrap();
        let accounts = core.accounts().await.unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].name, "Alice");
        assert!(accounts[0].microsoft.is_none());

        core.upsert_account(microsoft_account(
            "069a79f444e94726a5befca90e38aaf5",
            "Steve",
        ))
        .await
        .unwrap();
        let stored = tokio::fs::read_to_string(dir.0.join("accounts.json"))
            .await
            .unwrap();
        assert!(stored.contains("refresh-Steve"), "{stored}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.0.join("accounts.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "refresh tokens must stay owner-only");
        }
    }

    #[test]
    fn file_names_cannot_escape() {
        assert!(validate_file_name("ok.jar").is_ok());
        assert!(validate_file_name("../escape.jar").is_err());
        assert!(validate_file_name("sub/dir.jar").is_err());
    }
}
