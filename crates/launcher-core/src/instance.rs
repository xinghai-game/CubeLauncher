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

fn slug(value: &str) -> String {
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
    pub async fn accounts(&self) -> Result<Vec<OfflineAccount>> {
        let path = self.paths.root.join("accounts.json");
        let Ok(bytes) = tokio::fs::read(&path).await else {
            return Ok(Vec::new());
        };
        Ok(serde_json::from_slice(&bytes).unwrap_or_default())
    }

    async fn write_accounts(&self, accounts: &[OfflineAccount]) -> Result<()> {
        crate::atomic_write(
            &self.paths.root.join("accounts.json"),
            &serde_json::to_vec_pretty(accounts)?,
        )
        .await
    }

    pub async fn add_offline_account(&self, name: &str) -> Result<OfflineAccount> {
        let name = name.trim();
        validate_username(name)?;
        let mut accounts = self.accounts().await?;
        if accounts.iter().any(|account| account.name == name) {
            bail!("离线角色 “{name}” 已经存在");
        }
        let account = OfflineAccount {
            id: offline_uuid(name),
            name: name.to_string(),
            uuid: offline_uuid(name),
            kind: "offline".to_string(),
            created_at: Utc::now(),
        };
        accounts.push(account.clone());
        self.write_accounts(&accounts).await?;
        Ok(account)
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
    /// removes saves and mods that live inside the instance.
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

    pub fn mods_dir(&self, id: &str) -> PathBuf {
        self.paths.game_dir(id).join("mods")
    }

    pub async fn ensure_mods_dir(&self, id: &str) -> Result<PathBuf> {
        let dir = self.mods_dir(id);
        tokio::fs::create_dir_all(&dir).await?;
        Ok(dir)
    }

    /// List local mods with their enabled state, sorted by name.
    pub async fn list_mods(&self, id: &str) -> Result<Vec<ModEntry>> {
        let dir = self.ensure_mods_dir(id).await?;
        let mut out = Vec::new();
        let mut entries = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if !entry.file_type().await?.is_file() {
                continue;
            }
            let file_name = entry.file_name().to_string_lossy().to_string();
            let enabled = file_name.ends_with(".jar");
            let is_mod = enabled || file_name.ends_with(".jar.disabled");
            if !is_mod {
                continue;
            }
            let metadata = entry.metadata().await?;
            out.push(ModEntry {
                file_name: file_name.clone(),
                enabled,
                size: metadata.len(),
                modified: metadata.modified().ok().map(chrono::DateTime::<Utc>::from),
                display_name: file_name
                    .trim_end_matches(".disabled")
                    .trim_end_matches(".jar")
                    .to_string(),
            });
        }
        out.sort_by(|left, right| left.display_name.cmp(&right.display_name));
        Ok(out)
    }

    /// Copy a mod file into the instance's mods directory.
    pub async fn add_mod(&self, id: &str, source: &Path) -> Result<String> {
        if !source.is_file() {
            bail!("找不到模组文件：{}", source.display());
        }
        let file_name = source
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .ok_or_else(|| anyhow!("模组文件名无效"))?;
        if !file_name.to_lowercase().ends_with(".jar") {
            bail!("只支持添加 .jar 模组文件");
        }
        let dir = self.ensure_mods_dir(id).await?;
        let destination = dir.join(&file_name);
        if destination.exists() {
            bail!("模组目录中已经有 {file_name}");
        }
        tokio::fs::copy(source, &destination).await?;
        Ok(file_name)
    }

    /// Toggle a mod by renaming between `.jar` and `.jar.disabled`.
    ///
    /// The caller may pass either the plain file name or the stored name that
    /// still carries the `.disabled` suffix.
    pub async fn toggle_mod(&self, id: &str, file_name: &str, enabled: bool) -> Result<String> {
        let dir = self.ensure_mods_dir(id).await?;
        validate_file_name(file_name)?;
        let base = file_name.trim_end_matches(".disabled");
        if base.is_empty() || !base.ends_with(".jar") {
            bail!("只能启用或停用 .jar 模组文件");
        }
        let (from, to) = if enabled {
            (dir.join(format!("{base}.disabled")), dir.join(base))
        } else {
            (dir.join(base), dir.join(format!("{base}.disabled")))
        };
        if !from.is_file() {
            if to.is_file() {
                // Already in the requested state: nothing to do.
                return Ok(to
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_default());
            }
            bail!("找不到模组文件：{}", from.display());
        }
        tokio::fs::rename(&from, &to).await?;
        Ok(to
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default())
    }

    pub async fn delete_mod(&self, id: &str, file_name: &str) -> Result<()> {
        let dir = self.ensure_mods_dir(id).await?;
        validate_file_name(file_name)?;
        let path = dir.join(file_name);
        if !path.is_file() {
            bail!("找不到模组文件：{}", path.display());
        }
        tokio::fs::remove_file(path).await?;
        Ok(())
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModEntry {
    pub file_name: String,
    pub display_name: String,
    pub enabled: bool,
    pub size: u64,
    pub modified: Option<chrono::DateTime<Utc>>,
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

    #[test]
    fn file_names_cannot_escape() {
        assert!(validate_file_name("ok.jar").is_ok());
        assert!(validate_file_name("../escape.jar").is_err());
        assert!(validate_file_name("sub/dir.jar").is_err());
    }
}
