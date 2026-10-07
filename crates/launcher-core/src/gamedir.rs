//! External `.minecraft` directories.
//!
//! Every instance owns a game directory. By default that is the launcher-managed
//! `.minecraft` inside the instance folder, but an instance may instead point at an
//! existing installation the user already had (the official launcher, HMCL, PCL…).
//! Importing never copies or rewrites anything inside that directory: the launcher
//! remembers where it is and reads versions, libraries and assets from there, and
//! only writes the files that are genuinely missing.

use crate::download::DownloadItem;
use crate::types::*;
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// Folder names that make a directory look like a Minecraft game directory.
const GAME_DIR_MARKERS: &[&str] = &[
    "versions",
    "libraries",
    "assets",
    "mods",
    "saves",
    "config",
    "resourcepacks",
    "shaderpacks",
    "options.txt",
];

/// Nested folder names a launcher-style game folder may use.
const NESTED_NAMES: &[&str] = &[".minecraft", "minecraft"];

/// Maven coordinates that identify the loaders this launcher can install itself.
/// NeoForge is listed first: it reuses artifacts named after Forge.
const LOADER_LIBRARIES: &[(&str, &str, Loader)] = &[
    ("net.neoforged", "neoforge", Loader::NeoForge),
    ("net.neoforged.fancymodloader", "loader", Loader::NeoForge),
    // NeoForge 1.20.1 was published under Forge's artifact name.
    ("net.neoforged", "forge", Loader::NeoForge),
    // Modern Forge lists FancyModLoader rather than the `forge` artifact itself.
    ("net.minecraftforge", "fmlloader", Loader::Forge),
    ("net.minecraftforge", "forge", Loader::Forge),
    ("net.fabricmc", "fabric-loader", Loader::Fabric),
];

/// Loaders that can be started from their own version document but have no
/// installer here. Importing them works; the label stays honest about the name.
const OTHER_LOADERS: &[(&str, &str)] = &[
    ("quilt", "Quilt"),
    ("optifine", "OptiFine"),
    ("liteloader", "LiteLoader"),
];

/// True when a directory looks like a Minecraft game directory.
pub fn is_game_dir(path: &Path) -> bool {
    path.is_dir() && GAME_DIR_MARKERS.iter().any(|name| path.join(name).exists())
}

/// Accept either a `.minecraft` directory or a folder that contains one, which is
/// how users usually pick another launcher's game folder. Returns the directory
/// that actually holds the game files and whether a nested one was used.
pub fn resolve_game_dir(picked: &Path) -> Result<(PathBuf, bool)> {
    if !picked.exists() {
        bail!("目录不存在：{}", picked.display());
    }
    if !picked.is_dir() {
        bail!("请选择目录，而不是文件：{}", picked.display());
    }
    if is_game_dir(picked) {
        return Ok((normalize(picked), false));
    }
    for name in NESTED_NAMES {
        let nested = picked.join(name);
        if is_game_dir(&nested) {
            return Ok((normalize(&nested), true));
        }
    }
    bail!(
        "{} 看起来不是 .minecraft 目录：没有找到 versions/、mods/ 之类的游戏文件夹",
        picked.display()
    )
}

/// Compare two directories the way the file system does, tolerating symlinks and
/// trailing separators.
pub fn same_dir(left: &Path, right: &Path) -> bool {
    normalize(left) == normalize(right)
}

fn normalize(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Every location one instance reads or writes, resolved in one place so imported
/// directories and launcher-managed ones behave identically everywhere else.
#[derive(Debug, Clone)]
pub struct InstanceLayout {
    /// The `.minecraft` the game runs in.
    pub game_dir: PathBuf,
    /// The imported `.minecraft`, when the instance is bound to one.
    pub imported: Option<PathBuf>,
    /// Launcher-private directory for extracted natives and logs.
    pub natives_dir: PathBuf,
    /// Write targets for libraries and assets: the imported directory when it owns
    /// one, so a repaired installation stays in a single coherent place.
    pub libraries: PathBuf,
    pub assets: PathBuf,
    /// Write target for version jars, which always stays in the launcher's own
    /// cache: an imported `versions/` folder is someone else's to keep tidy.
    pub jars: PathBuf,
    /// Read order for game files, write target first, launcher cache last.
    pub library_roots: Vec<PathBuf>,
    pub asset_roots: Vec<PathBuf>,
    /// Read order for version documents and version jars.
    pub version_roots: Vec<PathBuf>,
    pub jar_roots: Vec<PathBuf>,
}

impl InstanceLayout {
    fn find(roots: &[PathBuf], relative: &Path) -> Option<PathBuf> {
        roots
            .iter()
            .map(|root| root.join(relative))
            .find(|candidate| candidate.is_file())
    }

    /// A library file, by its path inside a `libraries` directory.
    pub fn library(&self, relative: &str) -> Option<PathBuf> {
        Self::find(&self.library_roots, Path::new(relative))
    }

    /// An asset file, by its path inside an `assets` directory.
    pub fn asset(&self, relative: &str) -> Option<PathBuf> {
        Self::find(&self.asset_roots, Path::new(relative))
    }

    /// The version document `<id>/<id>.json`.
    pub fn version_doc(&self, id: &str) -> Option<PathBuf> {
        Self::find(
            &self.version_roots,
            &Path::new(id).join(format!("{id}.json")),
        )
    }

    /// The version jar `<id>/<id>.jar`.
    pub fn version_jar(&self, id: &str) -> Option<PathBuf> {
        Self::find(&self.jar_roots, &Path::new(id).join(format!("{id}.jar")))
    }

    /// True when a planned download can already be read: at its destination, or in
    /// the imported directory the instance plays in.
    pub fn satisfied(&self, item: &DownloadItem) -> bool {
        if satisfies(item, &item.dest) {
            return true;
        }
        // A version jar written to `libraries/versions` corresponds to the `versions`
        // folders an imported `.minecraft` already has, so both write targets are
        // mapped onto their own read order.
        for (write_root, roots) in [
            (&self.jars, &self.jar_roots),
            (&self.libraries, &self.library_roots),
            (&self.assets, &self.asset_roots),
        ] {
            let Ok(relative) = item.dest.strip_prefix(write_root) else {
                continue;
            };
            if roots
                .iter()
                .map(|root| root.join(relative))
                .any(|candidate| satisfies(item, &candidate))
            {
                return true;
            }
        }
        false
    }
}

/// Check a candidate path against the integrity data the plan item carries.
fn satisfies(item: &DownloadItem, path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let mut candidate = item.clone();
    candidate.dest = path.to_path_buf();
    candidate.is_satisfied()
}

/// Recognise the loader a version document belongs to, together with its version
/// and an optional note for the user.
///
/// The library list is authoritative: every loader adds a Maven coordinate no
/// vanilla version has. The version id is only a fallback for documents whose
/// libraries were stripped or merged away, and it is the better source for the
/// loader version when it carries one.
pub fn detect_loader(id: &str, meta: &VersionMeta) -> (Loader, Option<String>, Option<String>) {
    for library in &meta.libraries {
        let (group, artifact, version, _classifier) =
            crate::rules::split_maven_coordinate(&library.name);
        for (known_group, known_artifact, loader) in LOADER_LIBRARIES {
            if group == *known_group && artifact == *known_artifact {
                // `neoforge-21.1.255`: the FancyModLoader coordinate only knows its
                // own version, while the folder name has the one users recognise.
                let loader_version = loader_version_from_id(id, *loader)
                    .or_else(|| maven_loader_version(*loader, &version));
                return (*loader, loader_version, None);
            }
        }
    }
    let lower = id.to_lowercase();
    for (needle, label) in OTHER_LOADERS {
        if lower.contains(needle) {
            return (
                Loader::Vanilla,
                None,
                Some(format!(
                    "检测到 {label}：启动器没有它的安装器，将按该目录自带的版本文件启动"
                )),
            );
        }
    }
    // `neoforge` must be checked before `forge`, which it contains.
    for (needle, loader) in [
        ("neoforge", Loader::NeoForge),
        ("forge", Loader::Forge),
        ("fabric", Loader::Fabric),
    ] {
        if lower.contains(needle) {
            return (loader, loader_version_from_id(id, loader), None);
        }
    }
    (Loader::Vanilla, None, None)
}

/// Loader version from a Maven version string (`1.20.1-47.4.26` → `47.4.26`).
fn maven_loader_version(loader: Loader, maven_version: &str) -> Option<String> {
    let value = match loader {
        // Forge and its FancyModLoader prefix the coordinate with the game version.
        Loader::Forge => maven_version
            .split_once('-')
            .map(|(_game, rest)| rest)
            .unwrap_or(maven_version),
        _ => maven_version,
    };
    (!value.is_empty()).then(|| value.to_string())
}

/// Loader version parsed out of the version folder name. Some installations only
/// carry it there (`neoforge-21.1.255`), and it beats FancyModLoader's own number.
fn loader_version_from_id(id: &str, loader: Loader) -> Option<String> {
    let lower = id.to_lowercase();
    let after = |needle: &str| -> Option<&str> {
        let start = lower.find(needle)? + needle.len();
        let rest = &id[start..];
        (!rest.is_empty()).then_some(rest)
    };
    let value = match loader {
        // `1.20.1-forge-47.4.26`
        Loader::Forge => after("-forge-"),
        // `1.21.1-neoforge-21.1.255`, `neoforge-21.1.255`
        Loader::NeoForge => after("-neoforge-").or_else(|| after("neoforge-")),
        // `fabric-loader-0.19.5-1.20.1`: the game version is the last segment.
        Loader::Fabric => after("fabric-loader-")?
            .rsplit_once('-')
            .map(|(head, _)| head),
        Loader::Vanilla => None,
    }?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// Vanilla version an installation builds on: the inheritance parent when the
/// document declares one, otherwise the version id with the loader part removed.
pub fn game_version_of(id: &str, meta: &VersionMeta, loader: Loader) -> String {
    if let Some(parent) = meta
        .inherits_from
        .as_ref()
        .filter(|value| !value.is_empty())
    {
        return parent.clone();
    }
    let stripped = match loader {
        Loader::Forge => id.split_once("-forge-").map(|(base, _)| base.to_string()),
        Loader::NeoForge => id
            .split_once("-neoforge-")
            .map(|(base, _)| base.to_string()),
        // Fabric profiles are named `fabric-loader-<loader>-<game version>`.
        Loader::Fabric => id.rsplit_once('-').map(|(_head, game)| game.to_string()),
        Loader::Vanilla => None,
    };
    stripped
        .filter(|value| {
            !value.is_empty() && value.chars().next().is_some_and(|c| c.is_ascii_digit())
        })
        .unwrap_or_else(|| id.to_string())
}

/// Everything the launcher needs to know about one version inside a game directory.
struct ImportTarget {
    game_dir: PathBuf,
    version_id: String,
    document: PathBuf,
    game_version: String,
    loader: Loader,
    loader_version: Option<String>,
    java_major: Option<u32>,
}

impl crate::LauncherCore {
    /// The `.minecraft` an instance plays in.
    pub fn game_dir(&self, instance: &Instance) -> PathBuf {
        match instance.game_dir.as_ref() {
            Some(path) if !path.as_os_str().is_empty() => path.clone(),
            _ => self.paths.game_dir(&instance.id),
        }
    }

    /// Layout without an instance: planning for a version that is not installed yet.
    pub fn data_layout(&self) -> InstanceLayout {
        InstanceLayout {
            game_dir: self.paths.root.clone(),
            imported: None,
            natives_dir: self.paths.root.join("natives"),
            libraries: self.paths.libraries.clone(),
            assets: self.paths.assets.clone(),
            jars: self.paths.libraries.join("versions"),
            library_roots: vec![self.paths.libraries.clone()],
            asset_roots: vec![self.paths.assets.clone()],
            version_roots: vec![self.paths.versions_dir()],
            jar_roots: vec![
                self.paths.versions_dir(),
                self.paths.libraries.join("versions"),
            ],
        }
    }

    /// Resolve every path one instance uses. An imported `.minecraft` keeps its own
    /// libraries and assets, so an existing installation is never downloaded twice.
    pub fn layout(&self, instance: &Instance) -> InstanceLayout {
        let imported = instance
            .game_dir
            .as_ref()
            .filter(|path| !path.as_os_str().is_empty())
            .cloned();
        let (libraries, assets) = match &imported {
            Some(dir) => (
                if dir.join("libraries").is_dir() {
                    dir.join("libraries")
                } else {
                    self.paths.libraries.clone()
                },
                if dir.join("assets").is_dir() {
                    dir.join("assets")
                } else {
                    self.paths.assets.clone()
                },
            ),
            None => (self.paths.libraries.clone(), self.paths.assets.clone()),
        };
        let mut library_roots = vec![libraries.clone()];
        if libraries != self.paths.libraries {
            library_roots.push(self.paths.libraries.clone());
        }
        let mut asset_roots = vec![assets.clone()];
        if assets != self.paths.assets {
            asset_roots.push(self.paths.assets.clone());
        }
        let mut version_roots = Vec::new();
        let mut jar_roots = Vec::new();
        if let Some(dir) = &imported {
            version_roots.push(dir.join("versions"));
            jar_roots.push(dir.join("versions"));
        }
        version_roots.push(self.paths.versions_dir());
        jar_roots.push(self.paths.versions_dir());
        jar_roots.push(self.paths.libraries.join("versions"));
        InstanceLayout {
            game_dir: self.game_dir(instance),
            imported,
            natives_dir: self.paths.natives_dir(&instance.id),
            libraries,
            assets,
            jars: self.paths.libraries.join("versions"),
            library_roots,
            asset_roots,
            version_roots,
            jar_roots,
        }
    }

    /// Read a directory the user picked for import: which versions it holds, which
    /// of them are registered already, and how much content sits inside.
    pub async fn scan_game_dir(&self, picked: &Path) -> Result<GameDirScan> {
        let (game_dir, nested) = resolve_game_dir(picked)?;
        let mut versions: Vec<GameDirVersion> = Vec::new();
        if let Ok(mut entries) = tokio::fs::read_dir(game_dir.join("versions")).await {
            while let Some(entry) = entries.next_entry().await? {
                if !entry
                    .file_type()
                    .await
                    .map(|kind| kind.is_dir())
                    .unwrap_or(false)
                {
                    continue;
                }
                let folder = entry.file_name().to_string_lossy().to_string();
                let folder_path = entry.path();
                let Ok(bytes) = tokio::fs::read(folder_path.join(format!("{folder}.json"))).await
                else {
                    continue;
                };
                let Ok(meta) = serde_json::from_slice::<VersionMeta>(&bytes) else {
                    continue;
                };
                let id = if meta.id.trim().is_empty() {
                    folder.clone()
                } else {
                    meta.id.clone()
                };
                let (loader, loader_version, note) = detect_loader(&id, &meta);
                let game_version = game_version_of(&id, &meta, loader);
                versions.push(GameDirVersion {
                    game_version,
                    loader,
                    loader_version,
                    inherits_from: meta.inherits_from.clone(),
                    jar: folder_path.join(format!("{id}.jar")).is_file()
                        || folder_path.join(format!("{folder}.jar")).is_file(),
                    // A child document inherits the main class from its parent.
                    launchable: meta.main_class.is_some() || meta.inherits_from.is_some(),
                    note,
                    imported: false,
                    id,
                });
            }
        }
        let registered: Vec<(PathBuf, String)> = self
            .instances()
            .await?
            .into_iter()
            .filter_map(|instance| Some((instance.game_dir?, instance.version_id?)))
            .collect();
        for version in &mut versions {
            version.imported = registered
                .iter()
                .any(|(dir, id)| *id == version.id && same_dir(dir, &game_dir));
        }
        versions.sort_by(|left, right| {
            crate::rules::compare_versions(&right.game_version, &left.game_version)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(GameDirScan {
            mod_count: count_entries(&game_dir.join("mods"), true).await,
            save_count: count_entries(&game_dir.join("saves"), false).await,
            has_libraries: game_dir.join("libraries").is_dir(),
            has_assets: game_dir.join("assets").is_dir(),
            game_dir,
            nested,
            versions,
        })
    }

    /// Register an existing installation in place: nothing inside the picked
    /// directory is copied, the launcher only remembers where it is.
    pub async fn import_instance(
        &self,
        name: &str,
        picked: &Path,
        version_id: &str,
        account_id: Option<String>,
    ) -> Result<Instance> {
        let name = name.trim();
        if name.is_empty() {
            bail!("实例名称不能为空");
        }
        let target = self.inspect_import(picked, version_id).await?;
        let existing = self.instances().await?;
        if existing.iter().any(|instance| instance.name == name) {
            bail!("已经有一个叫 “{name}” 的实例");
        }
        self.ensure_not_bound(&existing, None, &target)?;
        let mut id = crate::instance::slug(name);
        if existing.iter().any(|instance| instance.id == id) {
            id = format!("{}-{}", id, &crate::instance::offline_uuid(name)[..8]);
        }
        let instance = Instance {
            id: id.clone(),
            name: name.to_string(),
            game_version: target.game_version.clone(),
            loader: target.loader,
            version_id: Some(target.version_id.clone()),
            game_dir: Some(target.game_dir.clone()),
            loader_version: target.loader_version.clone(),
            account_id,
            java_path: self.settings.default_java.clone(),
            min_memory_mb: 1024,
            max_memory_mb: self.settings.default_memory_mb,
            jvm_args: Vec::new(),
            game_args: Vec::new(),
            width: Some(1280),
            height: Some(720),
            fullscreen: false,
            // The files are already on disk, so there is nothing to install.
            installed: true,
            last_played: None,
            created_at: chrono::Utc::now(),
        };
        let dir = self.paths.instance_dir(&id);
        // Only launcher-private folders: the game directory stays untouched.
        for path in [dir.clone(), dir.join("natives"), dir.join("logs")] {
            tokio::fs::create_dir_all(path).await?;
        }
        self.update_instance(&instance).await?;
        self.write_import_lock(&instance, &target).await?;
        Ok(instance)
    }

    /// Point an existing instance at another game directory, or bring it back to
    /// the launcher-managed one with `picked = None`.
    pub async fn bind_game_dir(
        &self,
        instance_id: &str,
        picked: Option<&Path>,
        version_id: Option<&str>,
    ) -> Result<Instance> {
        let mut instance = self.instance(instance_id).await?;
        let Some(picked) = picked else {
            // Back to the launcher-managed directory. Whatever is inside it is not
            // the version this instance was playing, so it counts as uninstalled.
            instance.game_dir = None;
            instance.version_id = None;
            instance.installed = false;
            self.update_instance(&instance).await?;
            let _ =
                tokio::fs::remove_file(self.paths.instance_dir(&instance.id).join("install.lock"))
                    .await;
            return Ok(instance);
        };
        let target = self
            .inspect_import(picked, version_id.unwrap_or_default())
            .await?;
        let existing = self.instances().await?;
        self.ensure_not_bound(&existing, Some(&instance.id), &target)?;
        instance.game_version = target.game_version.clone();
        instance.loader = target.loader;
        instance.version_id = Some(target.version_id.clone());
        instance.game_dir = Some(target.game_dir.clone());
        instance.loader_version = target.loader_version.clone();
        instance.installed = true;
        self.update_instance(&instance).await?;
        self.write_import_lock(&instance, &target).await?;
        Ok(instance)
    }

    /// Read one version document out of a game directory and describe it.
    async fn inspect_import(&self, picked: &Path, version_id: &str) -> Result<ImportTarget> {
        let (game_dir, _nested) = resolve_game_dir(picked)?;
        let version_id = version_id.trim();
        if version_id.is_empty() {
            bail!("请选择要导入的游戏版本");
        }
        let document = game_dir
            .join("versions")
            .join(version_id)
            .join(format!("{version_id}.json"));
        let meta = self
            .version_meta_from_path(&document)
            .await
            .with_context(|| {
                format!(
                    "{} 里没有版本 {version_id}（找不到 {}）",
                    game_dir.display(),
                    document.display()
                )
            })?;
        if meta.main_class.is_none() && meta.inherits_from.is_none() {
            bail!("版本文件里没有 mainClass，无法启动：{}", document.display());
        }
        let id = if meta.id.trim().is_empty() {
            version_id.to_string()
        } else {
            meta.id.clone()
        };
        let (loader, loader_version, _note) = detect_loader(&id, &meta);
        let game_version = game_version_of(&id, &meta, loader);
        let java_major = meta
            .java_version
            .as_ref()
            .and_then(|value| value.major_version);
        Ok(ImportTarget {
            game_dir,
            version_id: id,
            document,
            game_version,
            loader,
            loader_version,
            java_major,
        })
    }

    /// Two instances sharing one version of one directory would fight over saves,
    /// mods and the log file, so that is refused instead of silently allowed.
    fn ensure_not_bound(
        &self,
        instances: &[Instance],
        skip: Option<&str>,
        target: &ImportTarget,
    ) -> Result<()> {
        for instance in instances {
            if skip == Some(instance.id.as_str()) {
                continue;
            }
            let (Some(dir), Some(version)) =
                (instance.game_dir.as_ref(), instance.version_id.as_ref())
            else {
                continue;
            };
            if version == &target.version_id && same_dir(dir, &target.game_dir) {
                bail!(
                    "这个目录的 {} 已经由实例 “{}” 使用，请选择别的版本或先删除该实例",
                    target.version_id,
                    instance.name
                );
            }
        }
        Ok(())
    }

    /// Record an imported installation in `install.lock`, pointing at the version
    /// document inside the game directory so nothing has to be fetched to launch.
    async fn write_import_lock(&self, instance: &Instance, target: &ImportTarget) -> Result<()> {
        let lock = InstallLock {
            schema_version: 1,
            game_version: target.game_version.clone(),
            loader: target.loader,
            loader_version: target.loader_version.clone(),
            metadata_path: Some(target.document.clone()),
            installed_at: chrono::Utc::now(),
            java_major: target.java_major,
            mods_managed: target.game_dir.join("mods").is_dir(),
        };
        self.write_lock(&instance.id, &lock).await
    }
}

/// Count files (`files_only`) or directories inside a folder, ignoring absence.
async fn count_entries(dir: &Path, files_only: bool) -> usize {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return 0;
    };
    let mut count = 0;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let is_file = entry
            .file_type()
            .await
            .map(|kind| kind.is_file())
            .unwrap_or(false);
        if is_file == files_only {
            count += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::DownloadItem;

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("cube-{label}-{}", std::process::id()))
    }

    async fn write(path: &Path, content: &str) {
        tokio::fs::create_dir_all(path.parent().expect("parent"))
            .await
            .expect("dir");
        tokio::fs::write(path, content).await.expect("write");
    }

    /// A minimal but complete `.minecraft` with one vanilla and one Forge version.
    async fn sample_game_dir(root: &Path) -> PathBuf {
        let game_dir = root.join("imported/.minecraft");
        write(
            &game_dir.join("versions/1.20.1/1.20.1.json"),
            r#"{
                "id": "1.20.1",
                "mainClass": "net.minecraft.client.main.Main",
                "javaVersion": { "majorVersion": 17 },
                "libraries": [],
                "assets": "5",
                "downloads": {
                    "client": { "url": "https://example.invalid/client.jar", "sha1": "aa", "size": 3 }
                }
            }"#,
        )
        .await;
        write(
            &game_dir.join("versions/1.20.1-forge-47.4.26/1.20.1-forge-47.4.26.json"),
            r#"{
                "id": "1.20.1-forge-47.4.26",
                "inheritsFrom": "1.20.1",
                "mainClass": "cpw.mods.bootstraplauncher.BootstrapLauncher",
                "libraries": [
                    { "name": "net.minecraftforge:forge:1.20.1-47.4.26", "downloads": {} }
                ]
            }"#,
        )
        .await;
        tokio::fs::create_dir_all(game_dir.join("libraries/net/minecraftforge/forge"))
            .await
            .expect("libraries");
        tokio::fs::create_dir_all(game_dir.join("assets/indexes"))
            .await
            .expect("assets");
        write(&game_dir.join("mods/example.jar"), "jar").await;
        write(&game_dir.join("saves/world/level.dat"), "level").await;
        game_dir
    }

    async fn core_for(root: &Path) -> crate::LauncherCore {
        let settings = AppSettings {
            data_dir: root.join("data"),
            offline_mode: true,
            ..Default::default()
        };
        crate::LauncherCore::new(settings).await.expect("core")
    }

    #[test]
    fn game_dir_detection_accepts_a_nested_minecraft() {
        let root = temp_root("gamedir-nested");
        let game_dir = root.join("official/.minecraft");
        std::fs::create_dir_all(game_dir.join("versions")).expect("dirs");
        // The picked folder itself is not a game directory, its child is.
        let (resolved, nested) = resolve_game_dir(&root.join("official")).expect("resolve");
        assert!(nested);
        assert!(same_dir(&resolved, &game_dir));
        // A `.minecraft` directory is taken as-is.
        let (direct, nested) = resolve_game_dir(&game_dir).expect("resolve direct");
        assert!(!nested);
        assert!(same_dir(&direct, &game_dir));
        // Anything else is rejected with a readable message.
        let empty = root.join("empty");
        std::fs::create_dir_all(&empty).expect("dirs");
        assert!(resolve_game_dir(&empty).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn loader_detection_covers_the_supported_loaders() {
        let meta = |libraries: &[&str]| VersionMeta {
            libraries: libraries
                .iter()
                .map(|name| Library {
                    name: (*name).to_string(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        // Modern Forge: the document lists FancyModLoader, not the `forge` artifact.
        let (loader, version, note) = detect_loader(
            "1.20.1-forge-47.4.26",
            &meta(&[
                "net.minecraftforge:fmlloader:1.20.1-47.4.26",
                "net.minecraftforge:forgespi:7.0.1",
            ]),
        );
        assert_eq!(loader, Loader::Forge);
        assert_eq!(version.as_deref(), Some("47.4.26"));
        assert!(note.is_none());

        // 1.12.2 Forge keeps the game version inside the Maven version, and the
        // official launcher names the folder without a separating dash.
        let (loader, version, _) = detect_loader(
            "1.12.2-forge1.12.2-14.23.5.2864",
            &meta(&["net.minecraftforge:forge:1.12.2-14.23.5.2864"]),
        );
        assert_eq!(loader, Loader::Forge);
        assert_eq!(version.as_deref(), Some("14.23.5.2864"));

        // NeoForge: FancyModLoader's own version is not the one users know.
        let (loader, version, _) = detect_loader(
            "neoforge-21.1.255",
            &meta(&[
                "net.neoforged.fancymodloader:loader:4.0.45",
                "net.neoforged:bus:8.0.5",
            ]),
        );
        assert_eq!(loader, Loader::NeoForge);
        assert_eq!(version.as_deref(), Some("21.1.255"));

        let (loader, version, _) = detect_loader(
            "1.21.1-neoforge-21.1.255",
            &meta(&["net.neoforged:neoforge:21.1.255"]),
        );
        assert_eq!(loader, Loader::NeoForge);
        assert_eq!(version.as_deref(), Some("21.1.255"));

        let (loader, version, _) = detect_loader(
            "fabric-loader-0.19.5-1.20.1",
            &meta(&[
                "net.fabricmc:fabric-loader:0.19.5",
                "net.fabricmc:intermediary:1.20.1",
            ]),
        );
        assert_eq!(loader, Loader::Fabric);
        assert_eq!(version.as_deref(), Some("0.19.5"));

        // A loader id without libraries still names itself, version included.
        let (loader, version, _) = detect_loader("1.20.1-forge-47.4.26", &meta(&[]));
        assert_eq!(loader, Loader::Forge);
        assert_eq!(version.as_deref(), Some("47.4.26"));

        // Plain vanilla, and a loader this launcher cannot install but can start.
        let (loader, version, note) =
            detect_loader("1.20.1", &meta(&["com.mojang:brigadier:1.0.18"]));
        assert_eq!(loader, Loader::Vanilla);
        assert!(version.is_none() && note.is_none());
        let (loader, _, note) = detect_loader("quilt-loader-0.20.0-1.20.1", &meta(&[]));
        assert_eq!(loader, Loader::Vanilla);
        assert!(note.expect("note").contains("Quilt"));
    }

    #[test]
    fn game_version_comes_from_inheritance_or_the_id() {
        let parent = VersionMeta {
            inherits_from: Some("1.20.1".into()),
            ..Default::default()
        };
        assert_eq!(
            game_version_of("1.20.1-forge-47.4.26", &parent, Loader::Forge),
            "1.20.1"
        );
        let empty = VersionMeta::default();
        assert_eq!(
            game_version_of("1.20.1-forge-47.4.26", &empty, Loader::Forge),
            "1.20.1"
        );
        assert_eq!(
            game_version_of("fabric-loader-0.19.5-1.20.1", &empty, Loader::Fabric),
            "1.20.1"
        );
        assert_eq!(game_version_of("1.21.1", &empty, Loader::Vanilla), "1.21.1");
    }

    #[tokio::test]
    async fn scan_reports_versions_and_content() {
        let root = temp_root("gamedir-scan");
        let game_dir = sample_game_dir(&root).await;
        let core = core_for(&root).await;
        let scan = core
            .scan_game_dir(&root.join("imported"))
            .await
            .expect("scan");
        assert!(scan.nested, "the picked folder contains .minecraft");
        assert!(same_dir(&scan.game_dir, &game_dir));
        assert_eq!(scan.versions.len(), 2);
        assert_eq!(scan.mod_count, 1);
        assert_eq!(scan.save_count, 1);
        assert!(scan.has_libraries && scan.has_assets);
        let forge = scan
            .versions
            .iter()
            .find(|version| version.id == "1.20.1-forge-47.4.26")
            .expect("forge version");
        assert_eq!(forge.loader, Loader::Forge);
        assert_eq!(forge.loader_version.as_deref(), Some("47.4.26"));
        assert_eq!(forge.game_version, "1.20.1");
        assert!(forge.launchable && !forge.imported);
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn import_registers_files_in_place() {
        let root = temp_root("gamedir-import");
        let game_dir = sample_game_dir(&root).await;
        let core = core_for(&root).await;
        let instance = core
            .import_instance("Imported", &game_dir, "1.20.1-forge-47.4.26", None)
            .await
            .expect("import");
        assert_eq!(instance.game_version, "1.20.1");
        assert_eq!(instance.loader, Loader::Forge);
        assert_eq!(instance.loader_version.as_deref(), Some("47.4.26"));
        assert_eq!(instance.version_id.as_deref(), Some("1.20.1-forge-47.4.26"));
        assert!(instance.installed);
        // Nothing is copied into the instance folder, the game directory is reused.
        let layout = core.layout(&instance);
        assert!(!core
            .paths
            .instance_dir(&instance.id)
            .join(".minecraft")
            .exists());
        assert!(same_dir(
            &layout.version_doc("1.20.1").expect("document"),
            &game_dir.join("versions/1.20.1/1.20.1.json")
        ));
        let lock = core.install_lock(&instance.id).await.expect("lock");
        assert_eq!(lock.loader, Loader::Forge);
        assert!(lock.mods_managed);
        // The same version of the same directory cannot be imported twice.
        let again = core
            .import_instance("Second", &game_dir, "1.20.1-forge-47.4.26", None)
            .await;
        assert!(again.is_err(), "duplicate import must be refused");
        // Scanning marks it as imported.
        let scan = core.scan_game_dir(&game_dir).await.expect("scan");
        assert!(
            scan.versions
                .iter()
                .find(|version| version.id == "1.20.1-forge-47.4.26")
                .expect("forge")
                .imported
        );
        // Deleting the instance keeps the imported directory untouched.
        core.delete_instance(&instance.id).await.expect("delete");
        assert!(game_dir.join("mods/example.jar").is_file());
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn imported_versions_resolve_offline_from_their_own_documents() {
        let root = temp_root("gamedir-resolve");
        let game_dir = sample_game_dir(&root).await;
        let core = core_for(&root).await;
        let instance = core
            .import_instance("Offline", &game_dir, "1.20.1-forge-47.4.26", None)
            .await
            .expect("import");
        // The parent document lives in the same imported directory: no network is
        // available here, so this only passes when it is read from there.
        let resolved = core.resolved_for(&instance).await.expect("resolve");
        assert_eq!(resolved.meta.id, "1.20.1-forge-47.4.26");
        assert_eq!(resolved.chain, vec!["1.20.1", "1.20.1-forge-47.4.26"]);
        assert_eq!(resolved.jar_version, "1.20.1");
        assert_eq!(
            resolved.meta.main_class.as_deref(),
            Some("cpw.mods.bootstraplauncher.BootstrapLauncher")
        );
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn imported_libraries_satisfy_planned_downloads() {
        let root = temp_root("gamedir-satisfied");
        let game_dir = sample_game_dir(&root).await;
        let core = core_for(&root).await;
        let instance = core
            .import_instance("Files", &game_dir, "1.20.1", None)
            .await
            .expect("import");
        let layout = core.layout(&instance);
        // An imported `.minecraft` owns its libraries, so that is the write target.
        assert!(same_dir(&layout.libraries, &game_dir.join("libraries")));
        let relative = "net/example/lib/1.0/lib-1.0.jar";
        let content = b"library";
        let path = layout.libraries.join(relative);
        tokio::fs::create_dir_all(path.parent().expect("parent"))
            .await
            .expect("dir");
        tokio::fs::write(&path, content).await.expect("write");
        let digest = {
            use sha1::{Digest, Sha1};
            hex::encode(Sha1::digest(content))
        };
        let item = DownloadItem::new("https://example.invalid/lib.jar", path.clone())
            .with_sha1(Some(digest));
        assert!(
            layout.satisfied(&item),
            "an identical file needs no download"
        );
        let missing = DownloadItem::new(
            "https://example.invalid/other.jar",
            layout.libraries.join("net/example/other/1.0/other-1.0.jar"),
        )
        .with_sha1(Some("00".repeat(20)));
        assert!(!layout.satisfied(&missing));
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn version_jars_in_the_imported_folder_need_no_download() {
        let root = temp_root("gamedir-client-jar");
        let game_dir = sample_game_dir(&root).await;
        let core = core_for(&root).await;
        let instance = core
            .import_instance("Jar", &game_dir, "1.20.1", None)
            .await
            .expect("import");
        let layout = core.layout(&instance);
        // Client jars are written to the launcher cache even for imported instances,
        // but an identical jar inside the imported folder must still be reused.
        let content = b"client jar";
        let imported_jar = game_dir.join("versions/1.20.1/1.20.1.jar");
        tokio::fs::write(&imported_jar, content)
            .await
            .expect("write");
        let digest = {
            use sha1::{Digest, Sha1};
            hex::encode(Sha1::digest(content))
        };
        let item = DownloadItem::new(
            "https://example.invalid/client.jar",
            layout.jars.join("1.20.1/1.20.1.jar"),
        )
        .with_sha1(Some(digest))
        .with_size(Some(content.len() as u64));
        assert!(
            layout.satisfied(&item),
            "the imported version jar must satisfy the plan"
        );
        // A different jar (the loader's patched one, say) is not reused by accident.
        let other = DownloadItem::new(
            "https://example.invalid/other.jar",
            layout.jars.join("1.20.1/1.20.1.jar"),
        )
        .with_sha1(Some("11".repeat(20)))
        .with_size(Some(content.len() as u64));
        assert!(!layout.satisfied(&other));
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn binding_and_unbinding_a_game_directory() {
        let root = temp_root("gamedir-bind");
        let game_dir = sample_game_dir(&root).await;
        let core = core_for(&root).await;
        let instance = core
            .create_instance("Managed", "1.20.1", Loader::Vanilla, None, None)
            .await
            .expect("create");
        assert!(instance.game_dir.is_none());
        assert!(core.paths.game_dir(&instance.id).is_dir());
        let bound = core
            .bind_game_dir(&instance.id, Some(&game_dir), Some("1.20.1"))
            .await
            .expect("bind");
        assert!(bound.installed);
        assert_eq!(bound.version_id.as_deref(), Some("1.20.1"));
        assert!(same_dir(bound.game_dir.as_deref().expect("dir"), &game_dir));
        // The instance's own `.minecraft` is left alone, not deleted or reused.
        let back = core
            .bind_game_dir(&instance.id, None, None)
            .await
            .expect("unbind");
        assert!(back.game_dir.is_none() && back.version_id.is_none());
        assert!(!back.installed);
        assert!(core.install_lock(&instance.id).await.is_none());
        let _ = tokio::fs::remove_dir_all(&root).await;
    }
}
