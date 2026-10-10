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
/// Application id compiled into the launcher for Microsoft device-code sign-in.
/// It is public OAuth metadata and does not contain a secret.
pub const MICROSOFT_CLIENT_ID: &str = "29f74a3b-a543-4aac-86cc-0f59d719bbe5";
/// `offline_access` is what makes the sign-in survive a restart: it yields the
/// refresh token the launcher stores instead of a password.
pub const MICROSOFT_SCOPE: &str = "XboxLive.signin offline_access";

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

/// Tokens and identity of one Microsoft (正版) account.
///
/// Only the refresh token is long-lived; the Minecraft access token is derived
/// again whenever it is about to expire, so a stale `access_token` here is
/// harmless. Both live in `accounts.json`, which is written owner-only on Unix.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MicrosoftAccount {
    /// Xbox user id, passed to the game as `--xuid`.
    #[serde(default)]
    pub xuid: String,
    #[serde(default)]
    pub refresh_token: String,
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub access_expires_at: Option<DateTime<Utc>>,
    /// Official skin texture, so the account list can show the real head.
    #[serde(default)]
    pub skin_url: Option<String>,
    /// Result of the last store lookup; informational, the profile is what
    /// actually proves the account owns the game.
    #[serde(default)]
    pub owns_java: bool,
    #[serde(default)]
    pub last_login: Option<DateTime<Utc>>,
}

/// One selectable identity: an offline role or a Microsoft account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    /// Stable id: the undashed UUID for Microsoft accounts, the derived offline
    /// UUID for offline roles. Instances reference it through `account_id`.
    pub id: String,
    pub name: String,
    /// Dashed UUID, as Mojang reports it.
    pub uuid: String,
    /// `offline` or `microsoft`.
    pub kind: String,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub microsoft: Option<MicrosoftAccount>,
}

impl Account {
    pub fn is_microsoft(&self) -> bool {
        self.kind == "microsoft"
    }
    /// Undashed UUID, which is what the game expects on the command line.
    pub fn undashed_uuid(&self) -> String {
        self.uuid.replace('-', "")
    }
}

/// What the user has to do to finish a device code sign-in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCodePrompt {
    /// Handle for polling; it is the device code, and it never leaves the launcher.
    pub login_id: String,
    pub user_code: String,
    pub verification_uri: String,
    pub message: String,
    pub expires_in: u64,
    /// Seconds the caller must wait between two polls.
    pub interval: u64,
}

/// One poll of a pending device code sign-in.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LoginPoll {
    /// The user has not finished on Microsoft's page yet.
    Pending,
    /// Microsoft asked for a slower poll; the caller should wait longer.
    SlowDown,
    Ready {
        account: Box<Account>,
    },
    Expired,
    Declined,
    Failed {
        message: String,
    },
}

/// Everything the launch command line needs about the identity being used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthSession {
    pub name: String,
    /// Undashed UUID, for `--uuid`.
    pub uuid: String,
    /// Value for the legacy `--session` argument.
    pub session: String,
    /// Minecraft access token, for `--accessToken`.
    pub access_token: String,
    /// `msa` for Microsoft accounts, `legacy` for offline roles.
    pub user_type: String,
    /// Xbox user id, for `--xuid`.
    pub xuid: String,
    /// Value for `--clientId`; empty for offline roles.
    pub client_id: String,
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

/// One game version found inside a `.minecraft` directory during an import scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameDirVersion {
    /// Version folder name, e.g. `1.20.1-forge-47.4.26`.
    pub id: String,
    /// Vanilla version this one builds on.
    pub game_version: String,
    pub loader: Loader,
    #[serde(default)]
    pub loader_version: Option<String>,
    /// Version document this one inherits from, when any.
    #[serde(default)]
    pub inherits_from: Option<String>,
    /// True when `<id>.jar` sits next to the version document.
    pub jar: bool,
    /// True when the document names a main class, so it can be started directly.
    pub launchable: bool,
    /// What the launcher recognised, when that is worth telling the user.
    #[serde(default)]
    pub note: Option<String>,
    /// True when this version is already registered as an instance.
    pub imported: bool,
}

/// What one directory looks like before it is imported as an instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameDirScan {
    /// The `.minecraft` directory that was actually read.
    pub game_dir: PathBuf,
    /// True when a nested `.minecraft` inside the picked folder was used.
    pub nested: bool,
    /// Versions found, newest first.
    pub versions: Vec<GameDirVersion>,
    pub mod_count: usize,
    pub save_count: usize,
    pub has_libraries: bool,
    pub has_assets: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JavaRuntime {
    pub path: PathBuf,
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
    #[serde(default, rename = "virtual")]
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

/// One file transferring right now. Segment workers of the same file share one
/// entry, so `downloaded` covers every byte of that file already on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveDownload {
    /// Display label from the plan, or the destination path when it has none.
    pub name: String,
    pub downloaded: u64,
    /// Declared size; `0` when upstream never published one.
    pub total: u64,
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
    /// Bytes of the current batch already on disk, and its known total. Files
    /// without a published size are counted in neither.
    #[serde(default)]
    pub downloaded_bytes: u64,
    #[serde(default)]
    pub total_bytes: u64,
    /// Files transferring right now, largest first.
    #[serde(default)]
    pub active: Vec<ActiveDownload>,
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
