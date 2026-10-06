use crate::download::{DownloadItem, Progress};
use crate::rules::normalize_arch;
use crate::types::*;
use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Inspect a Java executable (or a JRE/JDK home directory) and report its version.
pub fn inspect_java(path: &Path) -> Result<JavaRuntime> {
    let executable = resolve_java_executable(path)?;
    let output = Command::new(&executable)
        .args(["-XshowSettings:properties", "-version"])
        .output()
        .with_context(|| format!("无法执行 Java：{}", executable.display()))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let major = parse_java_major(&text)
        .ok_or_else(|| anyhow!("无法识别 Java 版本：{}", executable.display()))?;
    let architecture = parse_property(&text, "os.arch")
        .map(|value| normalize_arch(&value).to_string())
        .unwrap_or_else(|| normalize_arch(std::env::consts::ARCH).to_string());
    let vendor = parse_property(&text, "java.vendor");
    Ok(JavaRuntime {
        path: executable,
        major,
        architecture,
        vendor,
        source: "system".into(),
    })
}

pub fn resolve_java_executable(path: &Path) -> Result<PathBuf> {
    let executable_name = if cfg!(windows) { "java.exe" } else { "java" };
    if path.is_dir() {
        let direct = path.join("bin").join(executable_name);
        if direct.is_file() {
            return Ok(direct);
        }
        // macOS bundles: <home>/Contents/Home/bin/java
        let macos = path
            .join("Contents")
            .join("Home")
            .join("bin")
            .join(executable_name);
        if macos.is_file() {
            return Ok(macos);
        }
        bail!("目录中没有找到 Java 可执行文件：{}", path.display());
    }
    if path.is_file() {
        return Ok(path.to_path_buf());
    }
    // Bare command such as `java`: look it up on PATH.
    if let Some(found) = which(&path.to_string_lossy()) {
        return Ok(found);
    }
    bail!("找不到 Java：{}", path.display())
}

pub fn which(command: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(command);
        if candidate.is_file() {
            return Some(candidate);
        }
        if cfg!(windows) {
            let with_extension = dir.join(format!("{command}.exe"));
            if with_extension.is_file() {
                return Some(with_extension);
            }
        }
    }
    None
}

fn parse_java_major(text: &str) -> Option<u32> {
    if let Some(value) = parse_property(text, "java.specification.version") {
        let trimmed = value.trim();
        if let Ok(major) = trimmed.parse::<u32>() {
            return Some(major);
        }
        if let Some(minor) = trimmed.strip_prefix("1.") {
            return minor.parse::<u32>().ok();
        }
    }
    for line in text.lines() {
        if let Some(index) = line.find("version \"") {
            let rest = &line[index + 9..];
            let value = rest.split('"').next().unwrap_or("");
            let mut parts = value.split(['.', '_', '-']);
            let first = parts.next().unwrap_or("");
            if first == "1" {
                return parts.next().and_then(|v| v.parse::<u32>().ok());
            }
            return first.parse::<u32>().ok();
        }
    }
    None
}

fn parse_property(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(&format!("{key} = ")) {
            return Some(rest.trim().to_string());
        }
    }
    None
}

/// Adoptium binary package description used for managed runtimes.
///
/// `/v3/assets/latest/{major}/hotspot` returns one entry per release with a
/// singular `binary` object; older responses used a `binaries` array, so both
/// shapes are accepted instead of silently finding nothing.
#[derive(Debug, Deserialize)]
struct AdoptiumRelease {
    #[serde(default)]
    binary: Option<AdoptiumBinary>,
    #[serde(default)]
    binaries: Vec<AdoptiumBinary>,
}
impl AdoptiumRelease {
    fn into_binaries(self) -> Vec<AdoptiumBinary> {
        let mut out = self.binaries;
        if let Some(binary) = self.binary {
            out.push(binary);
        }
        out
    }
}
#[derive(Debug, Deserialize)]
struct AdoptiumBinary {
    #[serde(default)]
    architecture: String,
    #[serde(default)]
    image_type: String,
    #[serde(default)]
    os: String,
    #[serde(default)]
    package: Option<AdoptiumPackage>,
}
#[derive(Debug, Deserialize)]
struct AdoptiumPackage {
    #[serde(default)]
    checksum: String,
    #[serde(default)]
    link: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    size: u64,
}

fn adoptium_os() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "mac"
    } else {
        "linux"
    }
}

fn adoptium_arch(arch: &str) -> &'static str {
    match normalize_arch(arch) {
        "arm64" => "aarch64",
        "x86" => "x86",
        _ => "x64",
    }
}

impl crate::LauncherCore {
    /// Managed runtimes installed by the launcher itself.
    pub async fn managed_runtimes(&self) -> Vec<JavaRuntime> {
        let mut found = Vec::new();
        let Ok(mut entries) = tokio::fs::read_dir(&self.paths.runtimes).await else {
            return found;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            if !entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let home = find_java_home(&entry.path()).await;
            if let Some(home) = home {
                if let Ok(mut runtime) = inspect_java(&home) {
                    runtime.source = "managed".into();
                    found.push(runtime);
                }
            }
        }
        found.sort_by_key(|runtime| runtime.major);
        found
    }

    /// Every Java runtime we can reasonably find on this machine.
    pub async fn discover_java(&self) -> Vec<JavaRuntime> {
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(configured) = &self.settings.default_java {
            candidates.push(configured.clone());
        }
        if let Some(home) = std::env::var_os("JAVA_HOME") {
            candidates.push(PathBuf::from(home));
        }
        candidates.push(PathBuf::from(if cfg!(windows) {
            "java.exe"
        } else {
            "java"
        }));
        for dir in common_java_dirs() {
            if let Ok(mut entries) = tokio::fs::read_dir(&dir).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    candidates.push(entry.path());
                }
            }
        }
        let mut found: Vec<JavaRuntime> = Vec::new();
        for candidate in candidates {
            let resolved = match resolve_java_executable(&candidate) {
                Ok(path) => path,
                Err(_) => continue,
            };
            if found.iter().any(|runtime| runtime.path == resolved) {
                continue;
            }
            match inspect_java(&resolved) {
                Ok(runtime) => found.push(runtime),
                Err(error) => tracing::debug!("忽略 Java {}：{error}", resolved.display()),
            }
        }
        for runtime in self.managed_runtimes().await {
            if !found.iter().any(|item| item.path == runtime.path) {
                found.push(runtime);
            }
        }
        found.sort_by_key(|runtime| (runtime.major, runtime.path.clone()));
        found
    }

    /// Pick a runtime able to run the requested Java major version, preferring
    /// an exact major match, then a newer one, and preferring managed runtimes
    /// of the requested architecture.
    pub async fn select_java(&self, major: u32, arch: &str) -> Option<JavaRuntime> {
        let mut runtimes = self.discover_java().await;
        runtimes.retain(|runtime| runtime.major >= major);
        runtimes.sort_by_key(|runtime| {
            let exact = runtime.major != major;
            let arch_mismatch = runtime.architecture != normalize_arch(arch);
            let managed = runtime.source != "managed";
            (exact, arch_mismatch, managed, runtime.major)
        });
        runtimes.into_iter().next()
    }

    /// Download and unpack a matching runtime from Adoptium into the launcher's
    /// private directory. Nothing is installed system-wide.
    pub async fn install_java(
        &self,
        major: u32,
        arch: &str,
        progress: &Progress,
    ) -> Result<JavaRuntime> {
        progress
            .stage("java", format!("查询 Java {major} 运行时"))
            .await;
        let architecture = adoptium_arch(arch);
        let os = adoptium_os();
        let mut last_error = None;
        for image_type in ["jre", "jdk"] {
            let url = format!(
                "{ADOPTIUM_API}/assets/latest/{major}/hotspot?architecture={architecture}&image_type={image_type}&os={os}&vendor=eclipse"
            );
            let releases: Vec<AdoptiumRelease> = match self.downloader.fetch_json(&url).await {
                Ok(value) => value,
                Err(error) => {
                    last_error = Some(error);
                    continue;
                }
            };
            let binaries: Vec<AdoptiumBinary> = releases
                .into_iter()
                .flat_map(|release| release.into_binaries())
                .collect();
            let Some(package) = binaries
                .iter()
                .filter(|binary| {
                    binary.image_type == image_type
                        && binary.os == os
                        && normalize_arch(&binary.architecture) == normalize_arch(arch)
                })
                .filter_map(|binary| binary.package.as_ref())
                .next()
            else {
                last_error = Some(anyhow!(
                    "Adoptium 没有适用于 {os}/{architecture} 的 Java {major} {image_type}"
                ));
                continue;
            };
            let archive_name = if package.name.is_empty() {
                let extension = if cfg!(windows) { "zip" } else { "tar.gz" };
                format!("java-{major}-{os}-{architecture}.{extension}")
            } else {
                package.name.clone()
            };
            let download_dir = self.paths.downloads.join("java");
            tokio::fs::create_dir_all(&download_dir).await?;
            let archive_path = download_dir.join(&archive_name);
            // The upstream link redirects to a release asset that is slow or
            // blocked in some networks; an explicitly configured mirror is tried
            // first. The SHA-256 from the API is checked either way.
            let mut candidates: Vec<String> = Vec::new();
            if let Some(base) = &self.settings.java_mirror_base_url {
                let base = base.trim_end_matches('/');
                candidates.push(format!(
                    "{base}/{major}/{image_type}/{architecture}/{os}/{archive_name}"
                ));
            }
            candidates.push(package.link.clone());
            progress
                .stage("java", format!("下载 Java {major}（{image_type}）"))
                .await;
            let mut last_error = None;
            let mut downloaded = false;
            for (index, url) in candidates.iter().enumerate() {
                let item = DownloadItem {
                    url: url.clone(),
                    dest: archive_path.clone(),
                    sha1: None,
                    sha256: Some(package.checksum.clone()),
                    size: Some(package.size),
                    label: format!("Java {major}"),
                };
                match self.downloader.download(&item, progress).await {
                    Ok(()) => {
                        downloaded = true;
                        if index > 0 {
                            tracing::info!("镜像不可用，已回退上游 Java 下载地址");
                        }
                        break;
                    }
                    Err(error) => {
                        tracing::warn!("Java 下载地址失败（{url}）：{error:#}");
                        last_error = Some(error);
                        // A stale partial from a failed source must not poison the retry.
                        let _ = tokio::fs::remove_file(
                            archive_path.with_file_name(format!("{archive_name}.part")),
                        )
                        .await;
                    }
                }
            }
            if !downloaded {
                return Err(last_error
                    .unwrap_or_else(|| anyhow!("无法下载 Java {major} 运行时"))
                    .context(format!("下载 Java {major} 运行时失败")));
            }
            let target = self
                .paths
                .runtimes
                .join(format!("{major}-{architecture}-{image_type}"));
            if target.exists() {
                let _ = tokio::fs::remove_dir_all(&target).await;
            }
            tokio::fs::create_dir_all(&target).await?;
            progress.stage("java", format!("解压 Java {major}")).await;
            extract_archive(&archive_path, &target).await?;
            let _ = tokio::fs::remove_file(&archive_path).await;
            let home = find_java_home(&target).await.ok_or_else(|| {
                anyhow!("解压后的 Java 运行时缺少 bin/java：{}", target.display())
            })?;
            let mut runtime = inspect_java(&home)?;
            runtime.source = "managed".into();
            crate::atomic_write(
                &target.join("runtime.json"),
                &serde_json::to_vec_pretty(&runtime)?,
            )
            .await?;
            return Ok(runtime);
        }
        Err(last_error.unwrap_or_else(|| anyhow!("无法获取 Java {major} 运行时")))
    }

    /// Ensure a runtime for the version is available, installing one if allowed.
    pub async fn ensure_java(
        &self,
        major: u32,
        arch: &str,
        allow_download: bool,
        progress: &Progress,
    ) -> Result<JavaRuntime> {
        if let Some(runtime) = self.select_java(major, arch).await {
            return Ok(runtime);
        }
        if !allow_download {
            bail!("没有找到可用的 Java {major} 运行时，请在设置中选择或允许自动下载");
        }
        self.install_java(major, arch, progress).await
    }
}

async fn find_java_home(root: &Path) -> Option<PathBuf> {
    let executable_name = if cfg!(windows) { "java.exe" } else { "java" };
    let direct = root.join("bin").join(executable_name);
    if direct.is_file() {
        return direct
            .parent()
            .and_then(|bin| bin.parent())
            .map(|p| p.to_path_buf());
    }
    let macos = root
        .join("Contents")
        .join("Home")
        .join("bin")
        .join(executable_name);
    if macos.is_file() {
        return macos
            .parent()
            .and_then(|bin| bin.parent())
            .map(|p| p.to_path_buf());
    }
    // Archives unpack into a nested directory such as jdk-21.0.1+12-jre.
    let mut stack = vec![root.to_path_buf()];
    let mut visited = 0;
    while let Some(directory) = stack.pop() {
        visited += 1;
        if visited > 200 {
            break;
        }
        let candidate = directory.join("bin").join(executable_name);
        if candidate.is_file() {
            return Some(directory);
        }
        if let Ok(mut entries) = tokio::fs::read_dir(&directory).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
                    stack.push(entry.path());
                }
            }
        }
    }
    None
}

fn common_java_dirs() -> Vec<PathBuf> {
    if cfg!(windows) {
        let mut dirs = Vec::new();
        for base in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
            if let Some(root) = std::env::var_os(base) {
                let root = PathBuf::from(root);
                dirs.push(root.join("Java"));
                dirs.push(root.join("Eclipse Adoptium"));
                dirs.push(root.join("Microsoft"));
                dirs.push(root.join("Zulu"));
                dirs.push(root.join("BellSoft"));
            }
        }
        dirs
    } else if cfg!(target_os = "macos") {
        vec![
            PathBuf::from("/Library/Java/JavaVirtualMachines"),
            PathBuf::from("/System/Library/Java/JavaVirtualMachines"),
        ]
    } else {
        vec![
            PathBuf::from("/usr/lib/jvm"),
            PathBuf::from("/usr/java"),
            PathBuf::from("/opt/java"),
        ]
    }
}

/// Extract a `.tar.gz` or `.zip` runtime archive, refusing paths that escape the target.
pub async fn extract_archive(archive: &Path, target: &Path) -> Result<()> {
    let archive = archive.to_path_buf();
    let target = target.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let file = std::fs::File::open(&archive)
            .with_context(|| format!("打开压缩包失败：{}", archive.display()))?;
        let name = archive
            .file_name()
            .map(|value| value.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if name.ends_with(".zip") {
            let mut zip = zip::ZipArchive::new(file)?;
            for index in 0..zip.len() {
                let mut entry = zip.by_index(index)?;
                let Some(relative) = entry.enclosed_name() else {
                    bail!("压缩包包含越界路径：{}", entry.name());
                };
                let destination = target.join(relative);
                if entry.is_dir() {
                    std::fs::create_dir_all(&destination)?;
                    continue;
                }
                if let Some(parent) = destination.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut out = std::fs::File::create(&destination)?;
                std::io::copy(&mut entry, &mut out)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Some(mode) = entry.unix_mode() {
                        std::fs::set_permissions(
                            &destination,
                            std::fs::Permissions::from_mode(mode),
                        )?;
                    }
                }
            }
        } else {
            let decoder = flate2::read::GzDecoder::new(file);
            let mut tar = tar::Archive::new(decoder);
            tar.set_preserve_permissions(true);
            tar.unpack(&target)?;
        }
        Ok(())
    })
    .await?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modern_and_legacy_versions() {
        assert_eq!(
            parse_java_major("openjdk version \"25.0.3\" 2026-04-21 LTS\n"),
            Some(25)
        );
        assert_eq!(parse_java_major("java version \"1.8.0_402\"\n"), Some(8));
        assert_eq!(
            parse_java_major("java.specification.version = 17\n"),
            Some(17)
        );
    }

    #[test]
    fn parses_properties_section() {
        let text = "    os.arch = amd64\n    java.vendor = Eclipse Adoptium\n";
        assert_eq!(parse_property(text, "os.arch").as_deref(), Some("amd64"));
        assert_eq!(
            parse_property(text, "java.vendor").as_deref(),
            Some("Eclipse Adoptium")
        );
    }

    #[test]
    fn arch_normalisation_covers_upstream_names() {
        assert_eq!(normalize_arch("arm64"), "arm64");
        assert_eq!(normalize_arch("aarch64"), "arm64");
        assert_eq!(normalize_arch("amd64"), "x86_64");
        assert_eq!(normalize_arch("x86_64"), "x86_64");
    }

    #[test]
    fn adoptium_arch_matches_api_values() {
        assert_eq!(adoptium_arch("x86_64"), "x64");
        assert_eq!(adoptium_arch("arm64"), "aarch64");
        assert_eq!(adoptium_arch("x86"), "x86");
    }
}
