use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

pub const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
pub const FABRIC_META_URL: &str = "https://meta.fabricmc.net/v2";
pub const FORGE_MAVEN: &str = "https://maven.minecraftforge.net";
pub const NEOFORGE_MAVEN: &str = "https://maven.neoforged.net/releases";
pub const ADOPTIUM_API: &str = "https://api.adoptium.net/v3";
pub const RESOURCES_URL: &str = "https://resources.download.minecraft.net";
pub const LAUNCHER_NAME: &str = "CubeLauncher";
pub const LAUNCHER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppSettings {
    pub schema_version: u32,
    pub data_dir: PathBuf,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_locale")]
    pub locale: String,
    #[serde(default = "default_concurrency")]
    pub download_concurrency: usize,
    #[serde(default = "default_memory")]
    pub default_memory_mb: u32,
    #[serde(default)]
    pub default_java: Option<PathBuf>,
    #[serde(default)]
    pub offline_mode: bool,
    #[serde(default)]
    pub mirror_base_url: Option<String>,
    /// Optional redirect for Adoptium runtime downloads, e.g. a university mirror
    /// that serves the same files as `{base}/{major}/{image}/{arch}/{os}/{file}`.
    #[serde(default)]
    pub java_mirror_base_url: Option<String>,
    #[serde(default)]
    pub close_launcher_after_launch: bool,
}
fn default_theme() -> String {
    "dark".into()
}
fn default_locale() -> String {
    "zh-CN".into()
}
fn default_concurrency() -> usize {
    4
}
fn default_memory() -> u32 {
    4096
}
impl Default for AppSettings {
    fn default() -> Self {
        let data_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("CubeLauncher");
        Self {
            schema_version: 1,
            data_dir,
            theme: default_theme(),
            locale: default_locale(),
            download_concurrency: default_concurrency(),
            default_memory_mb: default_memory(),
            default_java: None,
            offline_mode: false,
            mirror_base_url: None,
            java_mirror_base_url: None,
            close_launcher_after_launch: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfflineAccount {
    pub id: String,
    pub name: String,
    pub uuid: String,
    pub kind: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Loader {
    Vanilla,
    Fabric,
    Forge,
    NeoForge,
}
impl Loader {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Vanilla => "原版",
            Self::Fabric => "Fabric",
            Self::Forge => "Forge",
            Self::NeoForge => "NeoForge",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instance {
    pub id: String,
    pub name: String,
    pub game_version: String,
    pub loader: Loader,
    /// Exact version folder inside the game directory. Imported installations keep
    /// the name the other launcher used (`1.20.1-forge-47.4.26`), because that is
    /// what `versions/` and the version document are called on disk.
    #[serde(default)]
    pub version_id: Option<String>,
    /// Game directory. `Some` means an imported `.minecraft` the user already had;
    /// `None` means the launcher-managed directory inside the instance folder.
    #[serde(default)]
    pub game_dir: Option<PathBuf>,
    #[serde(default)]
    pub loader_version: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub java_path: Option<PathBuf>,
    pub min_memory_mb: u32,
    pub max_memory_mb: u32,
    #[serde(default)]
    pub jvm_args: Vec<String>,
    #[serde(default)]
    pub game_args: Vec<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub fullscreen: bool,
    #[serde(default)]
    pub installed: bool,
    #[serde(default)]
    pub last_played: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JavaRuntime {
    pub major: u32,
    pub architecture: String,
    pub vendor: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionSummary {
    pub id: String,
    #[serde(rename = "type")]
    pub version_type: String,
    #[serde(rename = "releaseTime")]
    pub release_time: String,
    pub url: String,
    pub sha1: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub latest: LatestVersions,
    pub versions: Vec<VersionSummary>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatestVersions {
    pub release: String,
    pub snapshot: String,
}

/// Grouping used by the version list, following HMCL's categories. April Fools
/// releases are ordinary snapshots upstream but are unusable for normal play, so
/// they get their own group instead of hiding inside the snapshot list.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum VersionKind {
    Release,
    Snapshot,
    OldBeta,
    OldAlpha,
    AprilFools,
}
impl VersionKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Release => "正式版",
            Self::Snapshot => "快照",
            Self::OldBeta => "远古 Beta",
            Self::OldAlpha => "远古 Alpha",
            Self::AprilFools => "愚人节",
        }
    }
}

/// One manifest entry plus its derived category, as the version list shows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogVersion {
    pub id: String,
    #[serde(rename = "type")]
    pub version_type: String,
    pub kind: VersionKind,
    #[serde(rename = "releaseTime")]
    pub release_time: String,
    pub url: String,
    pub sha1: String,
}

/// Where a version list came from, so the UI can say so instead of guessing.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManifestSource {
    /// Served from the on-disk copy of `version_manifest_v2.json`.
    Cache,
    /// Fetched from Mojang.
    Official,
    /// Fetched through the configured mirror.
    Mirror,
}
impl ManifestSource {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Cache => "本地缓存",
            Self::Official => "官方源",
            Self::Mirror => "镜像",
        }
    }
}

/// The version list as the UI consumes it: every manifest entry, its derived
/// category, and the provenance of the list itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionCatalog {
    pub latest: LatestVersions,
    /// Newest first, exactly the order of the upstream manifest.
    pub versions: Vec<CatalogVersion>,
    pub total: usize,
    /// True when the list was served from disk rather than the network.
    pub cached: bool,
    pub source: ManifestSource,
    /// Modification time of the cached manifest, when it was read from disk.
    #[serde(default)]
    pub fetched_at: Option<DateTime<Utc>>,
}

/// Java requirement for one version. `source` says whether it came from the
/// version document itself or from the launcher's tested fallback table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JavaRequirement {
    pub major: u32,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DownloadInfo {
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Rule {
    pub action: String,
    #[serde(default)]
    pub os: Option<OsRule>,
    #[serde(default)]
    pub features: Option<HashMap<String, bool>>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OsRule {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arch: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default, rename = "versionRange")]
    pub version_range: Option<VersionRange>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VersionRange {
    #[serde(default)]
    pub min: Option<String>,
    #[serde(default)]
    pub max: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LibraryDownloads {
    #[serde(default)]
    pub artifact: Option<DownloadInfo>,
    #[serde(default)]
    pub classifiers: Option<HashMap<String, DownloadInfo>>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Library {
    pub name: String,
    #[serde(default)]
    pub downloads: Option<LibraryDownloads>,
    #[serde(default)]
    pub rules: Option<Vec<Rule>>,
    #[serde(default)]
    pub natives: Option<HashMap<String, String>>,
    #[serde(default)]
    pub extract: Option<ExtractRule>,
    #[serde(default)]
    pub url: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtractRule {
    #[serde(default)]
    pub exclude: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Arguments {
    #[serde(default)]
    pub game: Option<Vec<ArgumentValue>>,
    #[serde(default)]
    pub jvm: Option<Vec<ArgumentValue>>,
    /// Official defaults for the user's JVM arguments. Parsed for completeness
    /// but deliberately never applied: the launcher already writes `-Xms/-Xmx`
    /// from the instance settings, and appending these would let the metadata's
    /// own memory flags win over the user's choice.
    #[serde(default, rename = "default-user-jvm")]
    pub default_user_jvm: Option<Vec<ArgumentValue>>,
}
/// One element of an argument list (`arguments.game`, `arguments.jvm`,
/// `arguments.default-user-jvm`).
///
/// Upstream uses two shapes: a bare string, or an object carrying a `value` plus
/// an optional `rules` list. `rules` is optional because 26.x metadata publishes
/// its default memory flags as `{"value": [...]}` with no rules at all; requiring
/// the field made the whole version document unparseable.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ArgumentValue {
    Text(String),
    Conditional {
        /// `None` means "no condition": the entry always applies. An empty list
        /// would mean the opposite, because no rule matches and the Mojang rule
        /// evaluation starts from "denied".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rules: Option<Vec<Rule>>,
        value: ArgumentText,
    },
    /// A shape from a metadata revision this launcher does not know yet. It is
    /// preserved so one unknown entry cannot make an entire version impossible to
    /// install; argument expansion logs and skips it rather than guessing.
    Unknown(serde_json::Value),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ArgumentText {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JavaVersion {
    #[serde(default)]
    pub component: Option<String>,
    #[serde(default, rename = "majorVersion")]
    pub major_version: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AssetIndex {
    pub id: String,
    #[serde(default)]
    pub sha1: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default, rename = "totalSize")]
    pub total_size: Option<u64>,
    pub url: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AssetObject {
    pub hash: String,
    pub size: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AssetsFile {
    #[serde(default)]
    pub objects: HashMap<String, AssetObject>,
    #[serde(default)]
    pub map_to_resources: Option<bool>,
    #[serde(default)]
    pub virtual_: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Logging {
    #[serde(default)]
    pub client: Option<LoggingClient>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LoggingClient {
    pub argument: String,
    pub file: DownloadInfo,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VersionMeta {
    pub id: String,
    #[serde(default, rename = "inheritsFrom")]
    pub inherits_from: Option<String>,
    #[serde(default, rename = "mainClass")]
    pub main_class: Option<String>,
    #[serde(default, rename = "minecraftArguments")]
    pub minecraft_arguments: Option<String>,
    #[serde(default)]
    pub arguments: Option<Arguments>,
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(default)]
    pub downloads: Option<HashMap<String, DownloadInfo>>,
    #[serde(default, rename = "assetIndex")]
    pub asset_index: Option<AssetIndex>,
    #[serde(default)]
    pub assets: Option<String>,
    #[serde(default, rename = "javaVersion")]
    pub java_version: Option<JavaVersion>,
    #[serde(default)]
    pub logging: Option<Logging>,
    #[serde(default)]
    pub jar: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallLock {
    pub schema_version: u32,
    pub game_version: String,
    pub loader: Loader,
    #[serde(default)]
    pub loader_version: Option<String>,
    #[serde(default)]
    pub metadata_path: Option<PathBuf>,
    pub installed_at: DateTime<Utc>,
    #[serde(default)]
    pub java_major: Option<u32>,
    #[serde(default)]
    pub mods_managed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallTask {
    pub id: String,
    pub instance_id: String,
    pub phase: String,
    pub current: u64,
    pub total: u64,
    pub message: String,
    pub done: bool,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchPreview {
    pub java: PathBuf,
    pub args: Vec<String>,
    pub working_dir: PathBuf,
    pub classpath_entries: usize,
    pub natives_dir: PathBuf,
    pub missing_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogLine {
    pub stream: String,
    pub line: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoaderVersion {
    pub version: String,
    pub stable: bool,
    #[serde(default)]
    pub game_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstallerProfile {
    #[serde(default)]
    pub spec: Option<u32>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default, rename = "minecraft")]
    pub minecraft: Option<String>,
    #[serde(default)]
    pub json: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub install: Option<LegacyInstall>,
    #[serde(default, rename = "versionInfo")]
    pub version_info: Option<VersionMeta>,
    #[serde(default)]
    pub processors: Vec<Processor>,
    #[serde(default)]
    pub data: HashMap<String, DataEntry>,
    #[serde(default)]
    pub libraries: Vec<Library>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LegacyInstall {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default, rename = "filePath")]
    pub file_path: Option<String>,
    #[serde(default)]
    pub minecraft: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Processor {
    #[serde(default)]
    pub sides: Option<Vec<String>>,
    #[serde(default)]
    pub jar: String,
    #[serde(default)]
    pub classpath: Vec<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub outputs: HashMap<String, String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DataEntry {
    #[serde(default)]
    pub client: Option<String>,
    #[serde(default)]
    pub server: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JavaPackage {
    pub major: u32,
    pub path: PathBuf,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    pub vendor: String,
}
