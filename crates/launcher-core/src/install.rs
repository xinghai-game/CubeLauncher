use crate::download::{resource_url, DownloadItem, Progress};
use crate::meta::ResolvedVersion;
use crate::rules::{classpath_entry, maven_path, native_classifier, require_java_major};
use crate::types::*;
use anyhow::{anyhow, bail, Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Everything the launcher has to place on disk for one version.
pub struct InstallPlan {
    pub resolved: ResolvedVersion,
    pub files: Vec<DownloadItem>,
    pub natives: Vec<DownloadItem>,
    pub asset_count: usize,
    pub java_major: u32,
}

impl crate::LauncherCore {
    /// Install or repair the files for an instance, dispatching on its loader.
    pub async fn install_instance(
        &self,
        instance: &Instance,
        progress: Progress,
    ) -> Result<InstallLock> {
        progress
            .stage("prepare", format!("准备安装 {}", instance.name))
            .await;
        let lock = match instance.loader {
            Loader::Vanilla => self.install_vanilla(instance, &progress).await?,
            Loader::Fabric => self.install_fabric(instance, &progress).await?,
            Loader::Forge => {
                self.install_forge_like(instance, &progress, Loader::Forge)
                    .await?
            }
            Loader::NeoForge => {
                self.install_forge_like(instance, &progress, Loader::NeoForge)
                    .await?
            }
        };
        let mut updated = instance.clone();
        updated.installed = true;
        updated.loader_version = lock.loader_version.clone();
        self.update_instance(&updated).await?;
        progress
            .finish(format!(
                "{} {} 安装完成",
                instance.loader.label(),
                lock.loader_version.as_deref().unwrap_or("")
            ))
            .await;
        Ok(lock)
    }

    /// The vanilla half of every installation: client jar, libraries, assets, natives.
    pub async fn install_vanilla(
        &self,
        instance: &Instance,
        progress: &Progress,
    ) -> Result<InstallLock> {
        let plan = self.plan_vanilla(&instance.game_version).await?;
        self.apply_plan(instance, &plan, progress).await?;
        let lock = InstallLock {
            schema_version: 1,
            game_version: instance.game_version.clone(),
            loader: Loader::Vanilla,
            loader_version: None,
            metadata_path: Some(
                self.paths
                    .versions_dir()
                    .join(format!("{}.json", instance.game_version)),
            ),
            installed_at: chrono::Utc::now(),
            java_major: Some(plan.java_major),
            mods_managed: false,
        };
        self.write_lock(&instance.id, &lock).await?;
        Ok(lock)
    }

    pub async fn plan_vanilla(&self, game_version: &str) -> Result<InstallPlan> {
        let resolved = self.resolve_version(game_version).await?;
        self.plan_for(&resolved).await
    }

    pub async fn plan_for(&self, resolved: &ResolvedVersion) -> Result<InstallPlan> {
        let platform = crate::rules::Platform::current();
        let env = crate::rules::RuleEnv::new(true, !platform.os_version.is_empty());
        let java_major = require_java_major(
            &resolved.jar_version,
            resolved
                .meta
                .java_version
                .as_ref()
                .and_then(|value| value.major_version),
        );
        let mut files: Vec<DownloadItem> = Vec::new();
        let mut seen: HashSet<PathBuf> = HashSet::new();
        let mut natives: Vec<DownloadItem> = Vec::new();

        // Client jar: keep it next to the version metadata so several instances share it.
        if let Some(client) = resolved
            .meta
            .downloads
            .as_ref()
            .and_then(|downloads| downloads.get("client"))
        {
            let dest = self.client_jar_path(&resolved.jar_version);
            files.push(
                crate::download::download_item_from(client, dest.clone()).with_label("客户端 JAR"),
            );
            seen.insert(dest);
        }

        for library in &resolved.meta.libraries {
            if !crate::rules::library_allowed(library, &platform, &env) {
                continue;
            }
            if let Some((path, url)) = classpath_entry(library, &platform) {
                let dest = self.paths.libraries.join(&path);
                if seen.insert(dest.clone()) {
                    files.push(
                        DownloadItem::new(url, dest)
                            .with_sha1(library_sha1(library))
                            .with_size(library_size(library))
                            .with_label(short_name(&library.name)),
                    );
                }
            }
            if let Some((path, info)) = native_classifier(library, &platform) {
                let dest = self.paths.libraries.join(&path);
                if seen.insert(dest.clone()) {
                    // Native classifiers carry their own hash: verify against that.
                    natives.push(
                        crate::download::download_item_from(&info, dest)
                            .with_label(format!("{} 原生库", short_name(&library.name))),
                    );
                }
            }
        }
        files.extend(natives.iter().cloned());

        // Asset index and every object it lists.
        let mut asset_count = 0;
        if let Some(index) = &resolved.meta.asset_index {
            let index_path = self
                .paths
                .assets
                .join("indexes")
                .join(format!("{}.json", index.id));
            files.push(
                DownloadItem::new(index.url.clone(), index_path.clone())
                    .with_sha1(Some(index.sha1.clone()))
                    .with_size(Some(index.size))
                    .with_label("资源索引"),
            );
            let assets = self.ensure_asset_index(&index_path, index).await?;
            for object in assets.objects.values() {
                asset_count += 1;
                let dest = self
                    .paths
                    .assets
                    .join("objects")
                    .join(&object.hash[..2])
                    .join(&object.hash);
                if seen.insert(dest.clone()) {
                    files.push(
                        DownloadItem::new(resource_url(&object.hash), dest)
                            .with_sha1(Some(object.hash.clone()))
                            .with_size(Some(object.size))
                            .with_label("游戏资源"),
                    );
                }
            }
        }

        // Log4j configuration referenced by the version's JVM arguments.
        if let Some(logging) = resolved
            .meta
            .logging
            .as_ref()
            .and_then(|logging| logging.client.as_ref())
        {
            let dest = self.log_config_path(&resolved.meta.id);
            files.push(
                crate::download::download_item_from(&logging.file, dest.clone())
                    .with_label("日志配置"),
            );
            seen.insert(dest);
        }

        Ok(InstallPlan {
            resolved: resolved.clone(),
            files,
            natives,
            asset_count,
            java_major,
        })
    }

    async fn read_assets_index(&self, path: &Path) -> Result<AssetsFile> {
        let bytes = tokio::fs::read(path)
            .await
            .with_context(|| format!("读取资源索引失败：{}", path.display()))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Planning needs the asset index on disk to know which objects to fetch, so
    /// fetch it here (verified) when it is missing or damaged.
    async fn ensure_asset_index(&self, path: &Path, index: &AssetIndex) -> Result<AssetsFile> {
        let valid = crate::download::verify_sync(path, Some(&index.sha1), None).unwrap_or(false);
        if !valid {
            let item = DownloadItem::new(index.url.clone(), path.to_path_buf())
                .with_sha1(Some(index.sha1.clone()))
                .with_size(Some(index.size))
                .with_label("资源索引");
            let progress = Progress::new(None, "");
            self.downloader.download(&item, &progress).await?;
        }
        self.read_assets_index(path).await
    }

    pub fn client_jar_path(&self, version: &str) -> PathBuf {
        self.paths
            .libraries
            .join("versions")
            .join(version)
            .join(format!("{version}.jar"))
    }

    pub fn log_config_path(&self, version: &str) -> PathBuf {
        self.paths
            .assets
            .join("log_configs")
            .join(format!("{version}.xml"))
    }

    /// Fetch a version jar from disk, preferring a loader version jar when present.
    pub fn version_jar_for(&self, version_id: &str, fallback_version: &str) -> Option<PathBuf> {
        let loader_jar = self
            .paths
            .versions_dir()
            .join(version_id)
            .join(format!("{version_id}.jar"));
        if loader_jar.is_file() {
            return Some(loader_jar);
        }
        let vanilla = self.client_jar_path(fallback_version);
        vanilla.is_file().then_some(vanilla)
    }

    /// Download a plan with reporting, then extract legacy natives.
    pub async fn apply_plan(
        &self,
        instance: &Instance,
        plan: &InstallPlan,
        progress: &Progress,
    ) -> Result<()> {
        progress.set_total(plan.files.len() as u64);
        progress
            .stage(
                "download",
                format!(
                    "下载 {} 个文件（含 {} 个资源）",
                    plan.files.len(),
                    plan.asset_count
                ),
            )
            .await;
        self.downloader
            .download_all(plan.files.clone(), progress)
            .await?;
        self.extract_natives(instance, &plan.resolved).await?;
        Ok(())
    }

    /// Extract old-style native classifier jars into the instance natives directory.
    pub async fn extract_natives(
        &self,
        instance: &Instance,
        resolved: &ResolvedVersion,
    ) -> Result<()> {
        let platform = crate::rules::Platform::current();
        let env = crate::rules::RuleEnv::new(true, !platform.os_version.is_empty());
        let natives_dir = self.paths.natives_dir(&instance.id);
        tokio::fs::create_dir_all(&natives_dir).await?;
        for library in &resolved.meta.libraries {
            if !crate::rules::library_allowed(library, &platform, &env) {
                continue;
            }
            let Some((path, _url)) = native_classifier(library, &platform) else {
                continue;
            };
            let archive = self.paths.libraries.join(&path);
            if !archive.is_file() {
                continue;
            }
            let exclude = library
                .extract
                .as_ref()
                .and_then(|rule| rule.exclude.clone())
                .unwrap_or_default();
            let natives_dir = natives_dir.clone();
            let label = library.name.clone();
            tokio::task::spawn_blocking(move || -> Result<()> {
                let file = std::fs::File::open(&archive)?;
                let mut zip = zip::ZipArchive::new(file)?;
                for index in 0..zip.len() {
                    let mut entry = zip.by_index(index)?;
                    let name = entry.name().to_string();
                    if entry.is_dir()
                        || name.starts_with("META-INF/")
                        || name.ends_with('/')
                        || exclude.iter().any(|rule| name.starts_with(rule))
                    {
                        continue;
                    }
                    let Some(relative) = entry.enclosed_name() else {
                        bail!("原生库包含越界路径：{name}");
                    };
                    let destination = natives_dir.join(relative);
                    if !destination.starts_with(&natives_dir) {
                        bail!("原生库试图写入数据目录之外：{name}");
                    }
                    if let Some(parent) = destination.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let mut out = std::fs::File::create(&destination)?;
                    std::io::copy(&mut entry, &mut out)?;
                }
                Ok(())
            })
            .await
            .with_context(|| format!("解压原生库失败：{label}"))??;
        }
        Ok(())
    }

    pub async fn write_lock(&self, instance_id: &str, lock: &InstallLock) -> Result<()> {
        crate::atomic_write(
            &self.paths.instance_dir(instance_id).join("install.lock"),
            &serde_json::to_vec_pretty(lock)?,
        )
        .await
    }

    /// Install Fabric Loader on top of vanilla.
    pub async fn install_fabric(
        &self,
        instance: &Instance,
        progress: &Progress,
    ) -> Result<InstallLock> {
        progress.stage("loader", "准备 Fabric Loader").await;
        let vanilla = self.install_vanilla(instance, progress).await?;
        let loader_version = match &instance.loader_version {
            Some(version) if !version.is_empty() && version != "latest" => version.clone(),
            _ => self.latest_fabric_loader(&instance.game_version).await?,
        };
        progress
            .stage("loader", format!("获取 Fabric {loader_version} 启动配置"))
            .await;
        let url = format!(
            "{FABRIC_META_URL}/versions/loader/{}/{}/profile/json",
            urlencoding::encode(&instance.game_version),
            urlencoding::encode(&loader_version)
        );
        let bytes = self.downloader.fetch_bytes(&url).await?;
        let profile: VersionMeta =
            serde_json::from_slice(&bytes).context("解析 Fabric 启动配置失败")?;
        let profile_path = self.paths.versions_dir().join(format!(
            "{}-fabric-{loader_version}.json",
            instance.game_version
        ));
        crate::atomic_write(&profile_path, &bytes).await?;

        let platform = crate::rules::Platform::current();
        let env = crate::rules::RuleEnv::new(true, !platform.os_version.is_empty());
        let mut files = Vec::new();
        let mut seen = HashSet::new();
        for library in &profile.libraries {
            if !crate::rules::library_allowed(library, &platform, &env) {
                continue;
            }
            if let Some((path, url)) = classpath_entry(library, &platform) {
                let dest = self.paths.libraries.join(&path);
                if seen.insert(dest.clone()) {
                    files.push(
                        DownloadItem::new(url, dest)
                            .with_sha1(library_sha1(library))
                            .with_size(library_size(library))
                            .with_label(short_name(&library.name)),
                    );
                }
            }
        }
        progress.set_total(files.len() as u64);
        progress
            .stage("loader", format!("下载 Fabric 依赖（{} 个）", files.len()))
            .await;
        self.downloader.download_all(files, progress).await?;

        let lock = InstallLock {
            schema_version: 1,
            game_version: instance.game_version.clone(),
            loader: Loader::Fabric,
            loader_version: Some(loader_version),
            metadata_path: Some(profile_path),
            installed_at: chrono::Utc::now(),
            java_major: vanilla.java_major,
            mods_managed: true,
        };
        self.write_lock(&instance.id, &lock).await?;
        Ok(lock)
    }

    /// Available Fabric Loader versions for a game version, newest first.
    pub async fn fabric_loader_versions(&self, game_version: &str) -> Result<Vec<LoaderVersion>> {
        let url = format!(
            "{FABRIC_META_URL}/versions/loader/{}",
            urlencoding::encode(game_version)
        );
        let raw: Vec<serde_json::Value> = self.downloader.fetch_json(&url).await?;
        let mut out = Vec::new();
        for entry in raw {
            let loader = entry.get("loader").cloned().unwrap_or_default();
            let version = loader
                .get("version")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            if version.is_empty() {
                continue;
            }
            out.push(LoaderVersion {
                version: version.to_string(),
                stable: loader
                    .get("stable")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false),
                game_version: game_version.to_string(),
            });
        }
        Ok(out)
    }

    async fn latest_fabric_loader(&self, game_version: &str) -> Result<String> {
        let versions = self.fabric_loader_versions(game_version).await?;
        versions
            .iter()
            .find(|version| version.stable)
            .or_else(|| versions.first())
            .map(|version| version.version.clone())
            .ok_or_else(|| anyhow!("Fabric 没有支持 Minecraft {game_version} 的 Loader"))
    }

    /// Game versions Fabric supports, used to keep the version list honest.
    pub async fn fabric_game_versions(&self) -> Result<Vec<String>> {
        let raw: Vec<serde_json::Value> = self
            .downloader
            .fetch_json(&format!("{FABRIC_META_URL}/versions/game"))
            .await?;
        Ok(raw
            .into_iter()
            .filter_map(|entry| {
                entry
                    .get("version")
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string())
            })
            .collect())
    }

    /// Verify that an installed instance still has every file it needs.
    pub async fn verify_instance(&self, instance: &Instance) -> Result<Vec<String>> {
        let Some(lock) = self.install_lock(&instance.id).await else {
            return Ok(vec!["实例尚未安装".to_string()]);
        };
        let resolved = self.resolved_for(&instance.id).await?;
        let platform = crate::rules::Platform::current();
        let env = crate::rules::RuleEnv::new(true, !platform.os_version.is_empty());
        let mut missing = Vec::new();
        if self
            .version_jar_for(&resolved.meta.id, &resolved.jar_version)
            .is_none()
        {
            missing.push(format!("缺少版本 JAR：{}", resolved.meta.id));
        }
        for library in &resolved.meta.libraries {
            if !crate::rules::library_allowed(library, &platform, &env) {
                continue;
            }
            if let Some((path, _url)) = classpath_entry(library, &platform) {
                let dest = self.paths.libraries.join(&path);
                if !dest.is_file() {
                    missing.push(short_name(&library.name));
                }
            }
        }
        if let Some(index) = &resolved.meta.asset_index {
            let index_path = self
                .paths
                .assets
                .join("indexes")
                .join(format!("{}.json", index.id));
            if !index_path.is_file() {
                missing.push(format!("缺少资源索引 {}", index.id));
            }
        }
        if missing.is_empty() {
            // A cheap consistency check: the lock must match the instance's loader.
            if lock.loader != instance.loader {
                missing.push("安装记录与实例加载器不一致，建议重新安装".to_string());
            }
        }
        Ok(missing)
    }

    /// Resolve the version chain that an installed instance should launch.
    pub async fn resolved_for(&self, instance_id: &str) -> Result<ResolvedVersion> {
        let lock = self
            .install_lock(instance_id)
            .await
            .ok_or_else(|| anyhow!("实例尚未安装，请先安装"))?;
        match (&lock.metadata_path, lock.loader) {
            (Some(path), Loader::Fabric)
            | (Some(path), Loader::Forge)
            | (Some(path), Loader::NeoForge)
                if path.is_file() =>
            {
                let meta = self.version_meta_from_path(path).await?;
                let id = meta.id.clone();
                self.resolve_from_meta(meta, id).await
            }
            _ => self.resolve_version(&lock.game_version).await,
        }
    }
}

pub fn shortcut_target(instance: &Instance) -> PathBuf {
    PathBuf::from(&instance.id)
}

fn library_sha1(library: &Library) -> Option<String> {
    library
        .downloads
        .as_ref()
        .and_then(|downloads| downloads.artifact.as_ref())
        .and_then(|artifact| artifact.sha1.clone())
}

fn library_size(library: &Library) -> Option<u64> {
    library
        .downloads
        .as_ref()
        .and_then(|downloads| downloads.artifact.as_ref())
        .and_then(|artifact| artifact.size)
}

fn short_name(name: &str) -> String {
    let parts: Vec<&str> = name.split(':').collect();
    match (parts.get(1), parts.get(2)) {
        (Some(artifact), Some(version)) => format!("{artifact}-{version}"),
        _ => name.to_string(),
    }
}

/// Coordinates of a Maven artifact inside the libraries directory.
pub fn library_destination(libraries: &Path, coordinate: &str) -> PathBuf {
    libraries.join(maven_path(coordinate))
}

/// Map an installer `data` value to a concrete value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataValue {
    Maven(String),
    Literal(String),
    Embedded(String),
    Variable(String),
}

pub fn parse_data_value(raw: &str) -> DataValue {
    let trimmed = raw.trim();
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        DataValue::Maven(trimmed[1..trimmed.len() - 1].to_string())
    } else if (trimmed.starts_with('\'') && trimmed.ends_with('\''))
        || (trimmed.starts_with('"') && trimmed.ends_with('"'))
    {
        DataValue::Literal(trimmed[1..trimmed.len() - 1].to_string())
    } else if trimmed.starts_with('/') {
        DataValue::Embedded(trimmed.trim_start_matches('/').to_string())
    } else {
        DataValue::Variable(trimmed.to_string())
    }
}

/// Maven coordinate inside a data value, keeping any `@extension` suffix.
pub fn maven_with_extension(coordinate: &str) -> (String, Option<String>) {
    match coordinate.split_once('@') {
        Some((base, extension)) => (base.to_string(), Some(extension.to_string())),
        None => (coordinate.to_string(), None),
    }
}

/// Path for a Maven coordinate, honouring an `@extension` suffix.
pub fn maven_path_with_extension(coordinate: &str) -> String {
    let (base, extension) = maven_with_extension(coordinate);
    let path = maven_path(&base);
    match extension {
        Some(extension) => {
            let stem = path.strip_suffix(".jar").unwrap_or(&path).to_string();
            format!("{stem}.{extension}")
        }
        None => path,
    }
}

/// Expand `{VARIABLE}` references inside a processor argument.
pub fn substitute_data(
    text: &str,
    data: &HashMap<String, String>,
    extra: &HashMap<String, String>,
) -> Result<String> {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        result.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else {
            result.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let key = &after[..end];
        if let Some(value) = extra.get(key).or_else(|| data.get(key)) {
            result.push_str(value);
        } else {
            bail!("安装清单引用了未知变量 {{{key}}}");
        }
        rest = &after[end + 1..];
    }
    result.push_str(rest);
    Ok(result)
}

/// Expand a processor argument or data value.
///
/// Installer manifests use two notations: `{VARIABLE}` for the resolved data
/// table, and `[group:artifact:version:classifier@ext]` for an inline Maven
/// coordinate that must be located inside the libraries directory. Surrounding
/// single quotes mark a literal and are removed.
pub fn expand_installer_value(
    text: &str,
    data: &HashMap<String, String>,
    extra: &HashMap<String, String>,
    libraries: &Path,
) -> Result<String> {
    let trimmed = text.trim();
    let unquoted = if trimmed.len() >= 2 && trimmed.starts_with('\'') && trimmed.ends_with('\'') {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };
    // Inline Maven coordinates first, so their paths are not rescanned for `{}`.
    let mut with_paths = String::with_capacity(unquoted.len());
    let mut rest = unquoted;
    while let Some(start) = rest.find('[') {
        with_paths.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find(']') else {
            bail!("安装清单中的 Maven 坐标缺少右方括号：{unquoted}");
        };
        let coordinate = &after[..end];
        let path = libraries.join(maven_path_with_extension(coordinate));
        with_paths.push_str(&path.display().to_string());
        rest = &after[end + 1..];
    }
    with_paths.push_str(rest);
    substitute_data(&with_paths, data, extra)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_values_are_classified() {
        assert_eq!(
            parse_data_value("[net.minecraftforge:forge:1.20.1-47.4.26:client]"),
            DataValue::Maven("net.minecraftforge:forge:1.20.1-47.4.26:client".into())
        );
        assert_eq!(
            parse_data_value("'/data/client.lzma'"),
            DataValue::Literal("/data/client.lzma".into())
        );
        assert_eq!(
            parse_data_value("/data/client.lzma"),
            DataValue::Embedded("data/client.lzma".into())
        );
        assert_eq!(
            parse_data_value("{MC_SLIM}"),
            DataValue::Variable("{MC_SLIM}".into())
        );
    }

    #[test]
    fn extension_suffix_is_preserved() {
        assert_eq!(
            maven_path_with_extension("de.oceanlabs.mcp:mcp_config:1.20.1-20230612.114412:mappings@txt"),
            "de/oceanlabs/mcp/mcp_config/1.20.1-20230612.114412/mcp_config-1.20.1-20230612.114412-mappings.txt"
        );
        assert_eq!(
            maven_path_with_extension("net.minecraftforge:forge:1.20.1-47.4.26:client"),
            "net/minecraftforge/forge/1.20.1-47.4.26/forge-1.20.1-47.4.26-client.jar"
        );
    }

    #[test]
    fn unknown_variable_is_reported() {
        let mut data = HashMap::new();
        data.insert("MC_SLIM".to_string(), "/tmp/slim.jar".to_string());
        let extra = HashMap::new();
        assert_eq!(
            substitute_data("--input {MC_SLIM}", &data, &extra).expect("known"),
            "--input /tmp/slim.jar"
        );
        assert!(substitute_data("--x {NOPE}", &data, &extra).is_err());
    }
}
