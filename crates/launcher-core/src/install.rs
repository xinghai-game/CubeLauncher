use crate::download::{resource_url, DownloadItem, Progress};
use crate::meta::ResolvedVersion;
use crate::rules::{classpath_entry, maven_path, native_classifier, require_java_major};
use crate::types::*;
use anyhow::{anyhow, bail, Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

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
        let lock = if instance.version_id.is_some() && instance.game_dir.is_some() {
            // An imported installation already has its version document on disk:
            // read it, place whatever is missing next to it and stop there.
            self.install_imported(instance, &progress).await?
        } else {
            match instance.loader {
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
            }
        };
        progress.check_cancelled()?;
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
        let plan = self.plan_instance(instance).await?;
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

    /// Repair an imported installation from its own version document instead of
    /// reinstalling a loader. Whatever the game directory already provides is kept,
    /// only genuinely missing files are fetched.
    async fn install_imported(
        &self,
        instance: &Instance,
        progress: &Progress,
    ) -> Result<InstallLock> {
        progress.stage("prepare", "读取导入目录的版本文件").await;
        let layout = self.layout(instance);
        let version_id = instance.version_id.clone().unwrap_or_default();
        let document = layout
            .version_doc(&version_id)
            .ok_or_else(|| anyhow!("找不到版本文件 {version_id}.json，请检查游戏目录"))?;
        let resolved = self.resolve_for_install(instance).await?;
        let mut plan = self.plan_for(&layout, &resolved).await?;
        let planned = plan.files.len();
        plan.files.retain(|item| !layout.satisfied(item));
        progress
            .stage(
                "download",
                format!(
                    "导入目录已提供 {} 个文件，需要补齐 {} 个",
                    planned - plan.files.len(),
                    plan.files.len()
                ),
            )
            .await;
        self.apply_plan(instance, &plan, progress).await?;
        let lock = InstallLock {
            schema_version: 1,
            game_version: instance.game_version.clone(),
            loader: instance.loader,
            loader_version: instance.loader_version.clone(),
            metadata_path: Some(document),
            installed_at: chrono::Utc::now(),
            java_major: Some(plan.java_major),
            mods_managed: layout.game_dir.join("mods").is_dir(),
        };
        self.write_lock(&instance.id, &lock).await?;
        Ok(lock)
    }

    /// Plan a version that is not attached to an instance yet, using the
    /// launcher's own directories (the version list and the smoke harness do this).
    pub async fn plan_vanilla(&self, game_version: &str) -> Result<InstallPlan> {
        let resolved = self.resolve_version(game_version).await?;
        self.plan_for(&self.data_layout(), &resolved).await
    }

    /// Plan the files one instance needs, preferring what its own game directory
    /// already holds: an imported `.minecraft` is reused, not downloaded again.
    pub async fn plan_instance(&self, instance: &Instance) -> Result<InstallPlan> {
        let resolved = self.resolve_for_install(instance).await?;
        let layout = self.layout(instance);
        let mut plan = self.plan_for(&layout, &resolved).await?;
        plan.files.retain(|item| !layout.satisfied(item));
        Ok(plan)
    }

    /// Build the download list for one resolved version. `layout` decides where the
    /// files are written, so an imported game directory keeps its own libraries.
    pub async fn plan_for(
        &self,
        layout: &crate::gamedir::InstanceLayout,
        resolved: &ResolvedVersion,
    ) -> Result<InstallPlan> {
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
                let dest = layout.libraries.join(&path);
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
                let dest = layout.libraries.join(&path);
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
            let index_path = layout
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
                let relative = asset_object_relative(&object.hash)?;
                let dest = layout.assets.join(relative);
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
            let dest = layout
                .assets
                .join("log_configs")
                .join(format!("{}.xml", resolved.meta.id));
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
    /// The instance's own game directory is searched before the launcher's cache,
    /// so an imported installation uses the jar it already has.
    pub fn version_jar_for(
        &self,
        instance: &Instance,
        version_id: &str,
        fallback_version: &str,
    ) -> Option<PathBuf> {
        let layout = self.layout(instance);
        layout
            .version_jar(version_id)
            .or_else(|| layout.version_jar(fallback_version))
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
        let missing = self.ensure_assets(instance, &plan.resolved, false).await?;
        if !missing.is_empty() {
            bail!(
                "资源下载完成后仍缺少 {} 个文件：{}",
                missing.len(),
                missing.iter().take(3).cloned().collect::<Vec<_>>().join("、")
            );
        }
        self.extract_natives(instance, &plan.resolved).await?;
        Ok(())
    }

    /// Ensure the asset index is usable by the game. Modern versions read the
    /// hashed object store directly; legacy indexes additionally need the
    /// logical `virtual/legacy` tree created by Mojang's launcher.
    pub async fn ensure_assets(
        &self,
        instance: &Instance,
        resolved: &ResolvedVersion,
        verify_hashes: bool,
    ) -> Result<Vec<String>> {
        let Some(index) = &resolved.meta.asset_index else {
            return Ok(Vec::new());
        };
        let layout = self.layout(instance);
        let index_relative = format!("indexes/{}.json", index.id);
        let Some(index_path) = layout.asset(&index_relative) else {
            return Ok(vec![format!("缺少资源索引 {}", index.id)]);
        };
        if !index.sha1.is_empty()
            && !asset_file_matches(&index_path, index.size, &index.sha1, true)
        {
            return Ok(vec![format!("资源索引 {} 已损坏", index.id)]);
        }
        let assets = self.read_assets_index(&index_path).await?;
        let legacy = assets.virtual_.unwrap_or(false)
            || assets.map_to_resources.unwrap_or(false)
            || resolved.meta.assets.as_deref() == Some("legacy");
        let virtual_root = layout.assets.join("virtual").join("legacy");
        let mut missing = Vec::new();

        for (name, object) in assets.objects {
            let relative = match asset_object_relative(&object.hash) {
                Ok(relative) => relative,
                Err(error) => {
                    missing.push(format!("资源 {name}（{error:#}）"));
                    continue;
                }
            };
            let object_path = layout.asset(&relative);
            let object_ok = object_path
                .as_deref()
                .map(|path| asset_file_matches(path, object.size, &object.hash, verify_hashes))
                .unwrap_or(false);
            let virtual_path = if legacy {
                let logical = safe_asset_relative(&name)?;
                Some(virtual_root.join(logical))
            } else {
                None
            };

            if !object_ok {
                if virtual_path
                    .as_deref()
                    .map(|path| asset_file_matches(path, object.size, &object.hash, verify_hashes))
                    .unwrap_or(false)
                {
                    continue;
                }
                missing.push(format!("资源 {name}"));
                continue;
            }

            if let Some(destination) = virtual_path {
                let destination_ok = asset_file_matches(
                    &destination,
                    object.size,
                    &object.hash,
                    verify_hashes,
                );
                if !destination_ok {
                    let source = object_path.as_ref().expect("object_ok implies a source");
                    materialize_legacy_asset(source, &destination, object.size).await?;
                    if !asset_file_matches(
                        &destination,
                        object.size,
                        &object.hash,
                        verify_hashes,
                    ) {
                        missing.push(format!("资源映射 {name}"));
                    }
                }
            }
        }
        Ok(missing)
    }

    /// Extract old-style native classifier jars into the instance natives directory.
    pub async fn extract_natives(
        &self,
        instance: &Instance,
        resolved: &ResolvedVersion,
    ) -> Result<()> {
        let platform = crate::rules::Platform::current();
        let env = crate::rules::RuleEnv::new(true, !platform.os_version.is_empty());
        let layout = self.layout(instance);
        let natives_dir = layout.natives_dir.clone();
        tokio::fs::create_dir_all(&natives_dir).await?;
        for library in &resolved.meta.libraries {
            if !crate::rules::library_allowed(library, &platform, &env) {
                continue;
            }
            let Some((path, _url)) = native_classifier(library, &platform) else {
                continue;
            };
            // Read the archive from wherever the instance keeps its libraries.
            let Some(archive) = layout.library(&path) else {
                continue;
            };
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

    /// Verify that an installed instance still has every file it needs. Files are
    /// looked up in the instance's own game directory first, so an imported
    /// installation verifies as complete without downloading anything.
    pub async fn verify_instance(&self, instance: &Instance) -> Result<Vec<String>> {
        let Some(lock) = self.install_lock(&instance.id).await else {
            return Ok(vec!["实例尚未安装".to_string()]);
        };
        let resolved = self.resolved_for(instance).await?;
        let layout = self.layout(instance);
        let platform = crate::rules::Platform::current();
        let env = crate::rules::RuleEnv::new(true, !platform.os_version.is_empty());
        let mut missing = Vec::new();
        if self
            .version_jar_for(instance, &resolved.meta.id, &resolved.jar_version)
            .is_none()
        {
            missing.push(format!("缺少版本 JAR：{}", resolved.meta.id));
        }
        for library in &resolved.meta.libraries {
            if !crate::rules::library_allowed(library, &platform, &env) {
                continue;
            }
            if let Some((path, _url)) = classpath_entry(library, &platform) {
                if layout.library(&path).is_none() {
                    missing.push(short_name(&library.name));
                }
            }
        }
        missing.extend(self.ensure_assets(instance, &resolved, true).await?);
        if missing.is_empty() {
            // A cheap consistency check: the lock must match the instance's loader.
            if lock.loader != instance.loader {
                missing.push("安装记录与实例加载器不一致，建议重新安装".to_string());
            }
        }
        Ok(missing)
    }

    /// Resolve metadata needed while installing. A new or failed managed instance
    /// has no install.lock yet, so launch-time resolution is intentionally not used.
    async fn resolve_for_install(&self, instance: &Instance) -> Result<ResolvedVersion> {
        let layout = self.layout(instance);
        if let Some(version_id) = instance.version_id.as_deref() {
            let path = layout
                .version_doc(version_id)
                .ok_or_else(|| anyhow!("找不到版本文件 {version_id}.json，请检查游戏目录"))?;
            let meta = self.version_meta_from_path(&path).await?;
            return self
                .resolve_from_meta_in(&layout.version_roots, meta, version_id.to_string())
                .await;
        }
        self.resolve_version_in(&layout.version_roots, &instance.game_version)
            .await
    }

    /// Resolve the version chain that an installed instance should launch.
    ///
    /// An imported installation carries its own version document, so that is read
    /// first (and its `inheritsFrom` parents from the same directory), which keeps
    /// launching independent of the launcher's metadata cache and of the network.
    pub async fn resolved_for(&self, instance: &Instance) -> Result<ResolvedVersion> {
        let lock = self
            .install_lock(&instance.id)
            .await
            .ok_or_else(|| anyhow!("实例尚未安装，请先安装"))?;
        let layout = self.layout(instance);
        if let Some(version_id) = instance.version_id.as_deref() {
            let path = layout
                .version_doc(version_id)
                .ok_or_else(|| anyhow!("找不到版本文件 {version_id}.json，请检查游戏目录"))?;
            let meta = self.version_meta_from_path(&path).await?;
            return self
                .resolve_from_meta_in(&layout.version_roots, meta, version_id.to_string())
                .await;
        }
        match (&lock.metadata_path, lock.loader) {
            (Some(path), Loader::Fabric)
            | (Some(path), Loader::Forge)
            | (Some(path), Loader::NeoForge)
                if path.is_file() =>
            {
                let meta = self.version_meta_from_path(path).await?;
                let id = meta.id.clone();
                self.resolve_from_meta_in(&layout.version_roots, meta, id)
                    .await
            }
            _ => {
                self.resolve_version_in(&layout.version_roots, &lock.game_version)
                    .await
            }
        }
    }
}

pub fn shortcut_target(instance: &Instance) -> PathBuf {
    PathBuf::from(&instance.id)
}

fn asset_object_relative(hash: &str) -> Result<String> {
    if hash.len() < 2 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("资源对象哈希无效：{hash}");
    }
    Ok(format!("objects/{}/{}", &hash[..2], hash))
}

fn safe_asset_relative(name: &str) -> Result<PathBuf> {
    let path = Path::new(name);
    if name.is_empty() || path.is_absolute() {
        bail!("资源路径无效：{name}");
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        bail!("资源路径越界：{name}");
    }
    Ok(path.to_path_buf())
}

fn asset_file_matches(path: &Path, size: u64, hash: &str, verify_hash: bool) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() || metadata.len() != size {
        return false;
    }
    if !verify_hash {
        return true;
    }
    DownloadItem::new("", path.to_path_buf())
        .with_sha1(Some(hash.to_string()))
        .with_size(Some(size))
        .is_satisfied()
}

async fn materialize_legacy_asset(source: &Path, destination: &Path, _size: u64) -> Result<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| anyhow!("资源映射没有父目录：{}", destination.display()))?;
    tokio::fs::create_dir_all(parent).await?;
    if destination.is_dir() {
        bail!("资源映射目标是目录：{}", destination.display());
    }
    if destination.exists() {
        tokio::fs::remove_file(destination).await?;
    }

    let file_name = destination
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "asset".to_string());
    let temporary = parent.join(format!(".{file_name}.cubelauncher-part-{}", std::process::id()));
    let _ = tokio::fs::remove_file(&temporary).await;
    if let Err(link_error) = tokio::fs::hard_link(source, &temporary).await {
        tokio::fs::copy(source, &temporary)
            .await
            .with_context(|| {
                format!(
                    "创建旧版资源映射失败：{} -> {}（硬链接失败：{link_error}）",
                    source.display(),
                    destination.display()
                )
            })?;
    }
    tokio::fs::rename(&temporary, destination).await.with_context(|| {
        format!(
            "写入旧版资源映射失败：{}",
            destination.display()
        )
    })?;
    Ok(())
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

    #[tokio::test]
    async fn failed_install_can_be_retried_without_an_install_lock() {
        let root =
            std::env::temp_dir().join(format!("cube-install-initial-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&root).await;
        let settings = AppSettings {
            data_dir: root.join("data"),
            offline_mode: true,
            ..Default::default()
        };
        let core = crate::LauncherCore::new(settings).await.expect("core");
        let version_path = core.paths.versions_dir().join("1.20.1.json");
        tokio::fs::create_dir_all(version_path.parent().expect("version parent"))
            .await
            .expect("version directory");
        tokio::fs::write(
            &version_path,
            br#"{
                "id": "1.20.1",
                "mainClass": "net.minecraft.client.main.Main",
                "downloads": {
                    "client": {
                        "url": "https://example.invalid/client.jar",
                        "size": 4
                    }
                },
                "libraries": []
            }"#,
        )
        .await
        .expect("version metadata");

        let instance = core
            .create_instance("Fresh", "1.20.1", Loader::Vanilla, None, None)
            .await
            .expect("instance");
        assert!(core.install_lock(&instance.id).await.is_none());

        let cancelled = Progress::new(None, &instance.id);
        cancelled
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(core.install_instance(&instance, cancelled).await.is_err());
        assert!(core.install_lock(&instance.id).await.is_none());

        let client_jar = core.client_jar_path("1.20.1");
        tokio::fs::create_dir_all(client_jar.parent().expect("client parent"))
            .await
            .expect("client directory");
        tokio::fs::write(&client_jar, b"jar!")
            .await
            .expect("reusable client");
        let lock = core
            .install_instance(&instance, Progress::new(None, &instance.id))
            .await
            .expect("retry install");
        assert_eq!(lock.loader, Loader::Vanilla);
        assert!(
            core.instance(&instance.id)
                .await
                .expect("saved instance")
                .installed
        );
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

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
    fn asset_index_virtual_flag_and_paths_are_safe() {
        let assets: AssetsFile = serde_json::from_str(
            r#"{
                "virtual": true,
                "objects": {
                    "minecraft/lang/en_us.lang": {"hash": "abcdef", "size": 3}
                }
            }"#,
        )
        .expect("asset index");
        assert_eq!(assets.virtual_, Some(true));
        assert_eq!(asset_object_relative("abcdef").expect("object path"), "objects/ab/abcdef");
        assert!(asset_object_relative("x").is_err());
        assert!(safe_asset_relative("minecraft/lang/en_us.lang").is_ok());
        assert!(safe_asset_relative("../outside").is_err());
    }

    #[tokio::test]
    async fn ensure_assets_materializes_legacy_files_and_reports_missing() {
        use sha1::{Digest, Sha1};

        let root = std::env::temp_dir().join(format!("cube-asset-check-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&root).await;
        let core = crate::LauncherCore::new(AppSettings {
            data_dir: root.join("data"),
            offline_mode: true,
            ..Default::default()
        })
        .await
        .expect("core");
        let instance = Instance {
            id: "legacy".into(),
            name: "Legacy".into(),
            game_version: "1.7.10".into(),
            loader: Loader::Vanilla,
            version_id: None,
            game_dir: None,
            loader_version: None,
            account_id: None,
            java_path: None,
            min_memory_mb: 1024,
            max_memory_mb: 4096,
            jvm_args: Vec::new(),
            game_args: Vec::new(),
            width: Some(1280),
            height: Some(720),
            fullscreen: false,
            installed: true,
            last_played: None,
            created_at: chrono::Utc::now(),
        };
        let object_bytes = b"abc";
        let object_hash = hex::encode(Sha1::digest(object_bytes));
        let index_bytes = serde_json::to_vec(&serde_json::json!({
            "virtual": true,
            "objects": {
                "minecraft/lang/en_us.lang": {
                    "hash": object_hash,
                    "size": object_bytes.len()
                }
            }
        }))
        .expect("index json");
        let index_hash = hex::encode(Sha1::digest(&index_bytes));
        let index = AssetIndex {
            id: "legacy".into(),
            sha1: index_hash,
            size: index_bytes.len() as u64,
            total_size: None,
            url: String::new(),
        };
        let resolved = ResolvedVersion {
            meta: VersionMeta {
                id: "legacy".into(),
                assets: Some("legacy".into()),
                asset_index: Some(index),
                ..Default::default()
            },
            jar_version: "legacy".into(),
            chain: vec!["legacy".into()],
        };
        let assets = core.paths.assets.clone();
        let object_path = assets
            .join("objects")
            .join(&object_hash[..2])
            .join(&object_hash);
        let index_path = assets.join("indexes/legacy.json");
        tokio::fs::create_dir_all(object_path.parent().expect("object parent"))
            .await
            .expect("object directory");
        tokio::fs::create_dir_all(index_path.parent().expect("index parent"))
            .await
            .expect("index directory");
        tokio::fs::write(&object_path, object_bytes)
            .await
            .expect("object");
        tokio::fs::write(&index_path, &index_bytes)
            .await
            .expect("index");

        assert!(core
            .ensure_assets(&instance, &resolved, true)
            .await
            .expect("ensure")
            .is_empty());
        assert_eq!(
            tokio::fs::read(assets.join("virtual/legacy/minecraft/lang/en_us.lang"))
                .await
                .expect("logical asset"),
            object_bytes
        );

        tokio::fs::remove_file(&object_path).await.expect("remove object");
        tokio::fs::remove_file(assets.join("virtual/legacy/minecraft/lang/en_us.lang"))
            .await
            .expect("remove logical asset");
        let missing = core
            .ensure_assets(&instance, &resolved, true)
            .await
            .expect("missing report");
        assert_eq!(missing.len(), 1);
        assert!(missing[0].contains("minecraft/lang/en_us.lang"));
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn legacy_asset_is_materialized_at_logical_path() {
        let root = std::env::temp_dir().join(format!("cube-legacy-assets-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&root).await;
        let source = root.join("objects/ab/abcdef");
        let destination = root.join("virtual/legacy/minecraft/lang/en_us.lang");
        tokio::fs::create_dir_all(source.parent().expect("object parent"))
            .await
            .expect("object directory");
        tokio::fs::write(&source, b"abc").await.expect("object");
        materialize_legacy_asset(&source, &destination, 3)
            .await
            .expect("materialize");
        assert_eq!(tokio::fs::read(&destination).await.expect("logical asset"), b"abc");
        let _ = tokio::fs::remove_dir_all(&root).await;
    }
}
