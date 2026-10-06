use crate::download::{DownloadItem, Progress};
use crate::install::{
    expand_installer_value, maven_path_with_extension, parse_data_value, substitute_data, DataValue,
};
use crate::rules::maven_path;
use crate::types::*;
use anyhow::{anyhow, bail, Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::AsyncReadExt;

/// Loader-specific information needed to fetch an installer.
struct InstallerSource {
    url: String,
    version: String,
}

impl crate::LauncherCore {
    /// Install Forge or NeoForge by interpreting the official installer's client
    /// install list and running its processors with a Java runtime.
    pub async fn install_forge_like(
        &self,
        instance: &Instance,
        progress: &Progress,
        loader: Loader,
    ) -> Result<InstallLock> {
        progress
            .stage("loader", format!("准备 {} 安装", loader.label()))
            .await;
        // The loader needs the vanilla files present before processors run.
        let vanilla = self.install_vanilla(instance, progress).await?;
        let source = self.resolve_installer(instance, loader, progress).await?;
        progress
            .stage(
                "loader",
                format!("下载 {} {} 安装器", loader.label(), source.version),
            )
            .await;
        let installer_dir = self.paths.downloads.join("loaders");
        tokio::fs::create_dir_all(&installer_dir).await?;
        let installer_name = source
            .url
            .rsplit('/')
            .next()
            .unwrap_or("installer.jar")
            .to_string();
        let installer_path = installer_dir.join(&installer_name);
        self.downloader
            .download(
                &DownloadItem::new(source.url.clone(), installer_path.clone())
                    .with_label(format!("{} 安装器", loader.label())),
                progress,
            )
            .await?;

        // Read the profile and the version document that belongs to this installer.
        let installer = installer_path.clone();
        let (profile_bytes, version_bytes) = tokio::task::spawn_blocking(move || {
            let profile = read_jar_entry(&installer, "install_profile.json")?
                .ok_or_else(|| anyhow!("安装器缺少 install_profile.json"))?;
            let profile_value: serde_json::Value = serde_json::from_slice(&profile)?;
            let json_path = profile_value
                .get("json")
                .and_then(|value| value.as_str())
                .unwrap_or("/version.json")
                .trim_start_matches('/')
                .to_string();
            let version = read_jar_entry(&installer, &json_path)?
                .ok_or_else(|| anyhow!("安装器缺少 {json_path}"))?;
            Ok::<_, anyhow::Error>((profile, version))
        })
        .await??;
        let profile: InstallerProfile =
            serde_json::from_slice(&profile_bytes).context("解析安装清单失败")?;
        let version_meta: VersionMeta =
            serde_json::from_slice(&version_bytes).context("解析加载器版本信息失败")?;
        let version_id = if version_meta.id.is_empty() {
            profile
                .version
                .clone()
                .ok_or_else(|| anyhow!("安装清单没有版本 ID"))?
        } else {
            version_meta.id.clone()
        };

        // Persist the loader version document so launches never need the network.
        let metadata_path = self.paths.versions_dir().join(format!("{version_id}.json"));
        crate::atomic_write(&metadata_path, &version_bytes).await?;

        // Embedded Maven artifacts (universal jars and similar) come straight from the installer.
        let libraries = self.paths.libraries.clone();
        let installer_for_extract = installer_path.clone();
        let extracted = tokio::task::spawn_blocking(move || {
            extract_jar_prefix(&installer_for_extract, "maven/", &libraries)
        })
        .await??;
        tracing::info!("{} 内嵌构件 {} 个", loader.label(), extracted.len());

        // Download the installer's own libraries plus the launcher libraries.
        let mut files = Vec::new();
        let mut seen: HashSet<PathBuf> = HashSet::new();
        for library in profile
            .libraries
            .iter()
            .chain(version_meta.libraries.iter())
        {
            let Some(downloads) = &library.downloads else {
                continue;
            };
            let Some(artifact) = &downloads.artifact else {
                continue;
            };
            if artifact.url.is_empty() {
                continue;
            }
            let path = artifact
                .path
                .clone()
                .unwrap_or_else(|| maven_path(&library.name));
            let dest = self.paths.libraries.join(&path);
            if !seen.insert(dest.clone()) {
                continue;
            }
            files.push(
                crate::download::download_item_from(artifact, dest)
                    .with_label(library.name.clone()),
            );
        }
        progress.set_total(files.len() as u64);
        progress
            .stage("loader", format!("下载加载器依赖（{} 个）", files.len()))
            .await;
        self.downloader.download_all(files, progress).await?;

        // Legacy installers ship the loader jar itself and have no processors.
        if profile.processors.is_empty() {
            self.write_lock(
                &instance.id,
                &InstallLock {
                    schema_version: 1,
                    game_version: instance.game_version.clone(),
                    loader,
                    loader_version: Some(source.version.clone()),
                    metadata_path: Some(metadata_path),
                    installed_at: chrono::Utc::now(),
                    java_major: vanilla.java_major,
                    mods_managed: true,
                },
            )
            .await?;
            return self
                .install_lock(&instance.id)
                .await
                .ok_or_else(|| anyhow!("写入安装记录失败"));
        }

        // Processors run on Java and produce the patched client jar.
        let java = self
            .ensure_java(
                vanilla.java_major.unwrap_or(17),
                &crate::rules::Platform::current().arch,
                true,
                progress,
            )
            .await?;
        let mut extra: HashMap<String, String> = HashMap::new();
        extra.insert("ROOT".to_string(), self.paths.root.display().to_string());
        extra.insert(
            "INSTALLER".to_string(),
            installer_path.display().to_string(),
        );
        extra.insert(
            "MINECRAFT_JAR".to_string(),
            self.client_jar_path(&vanilla.game_version)
                .display()
                .to_string(),
        );
        extra.insert(
            "MINECRAFT_VERSION".to_string(),
            instance.game_version.clone(),
        );
        extra.insert(
            "LIBRARY_DIR".to_string(),
            self.paths.libraries.display().to_string(),
        );
        extra.insert("SIDE".to_string(), "client".to_string());

        // Resolve the installer data table, extracting embedded files as needed.
        let work_dir = self.paths.downloads.join(format!("work-{version_id}"));
        tokio::fs::create_dir_all(&work_dir).await?;
        let mut data: HashMap<String, String> = HashMap::new();
        let mut pending: Vec<(String, DataValue)> = Vec::new();
        for (key, entry) in &profile.data {
            let raw = entry
                .client
                .clone()
                .or_else(|| entry.server.clone())
                .ok_or_else(|| anyhow!("安装清单的 {key} 缺少客户端取值"))?;
            let parsed = parse_data_value(&raw);
            match parsed {
                DataValue::Maven(coordinate) => {
                    let path = maven_path_with_extension(&coordinate);
                    data.insert(
                        key.clone(),
                        self.paths.libraries.join(path).display().to_string(),
                    );
                }
                DataValue::Literal(value) => {
                    data.insert(key.clone(), value);
                }
                DataValue::Embedded(_) | DataValue::Variable(_) => {
                    pending.push((key.clone(), parsed));
                }
            }
        }
        let installer_for_data = installer_path.clone();
        let work_for_data = work_dir.clone();
        let embedded_requests: Vec<(String, String)> = pending
            .iter()
            .filter_map(|(key, value)| match value {
                DataValue::Embedded(entry) => Some((key.clone(), entry.clone())),
                _ => None,
            })
            .collect();
        let embedded: Vec<(String, PathBuf)> = tokio::task::spawn_blocking(move || {
            let mut out = Vec::new();
            for (key, entry) in &embedded_requests {
                let file_name = Path::new(entry)
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| key.clone());
                let destination = work_for_data.join(format!("{key}-{file_name}"));
                extract_jar_entry(&installer_for_data, entry, &destination)?;
                out.push((key.clone(), destination));
            }
            Ok::<_, anyhow::Error>(out)
        })
        .await??;
        for (key, path) in embedded {
            data.insert(key, path.display().to_string());
        }
        // Second pass: values that reference other variables.
        for (key, value) in &pending {
            if let DataValue::Variable(template) = value {
                let resolved = substitute_data(template, &data, &extra)?;
                data.insert(key.clone(), resolved);
            }
        }

        let client_processors: Vec<&Processor> = profile
            .processors
            .iter()
            .filter(|processor| match &processor.sides {
                None => true,
                Some(sides) => !sides.is_empty() && sides.iter().any(|side| side == "client"),
            })
            .collect();
        progress.set_total(client_processors.len() as u64);
        for (index, processor) in client_processors.iter().enumerate() {
            progress.check_cancelled()?;
            progress
                .stage(
                    "loader",
                    format!(
                        "运行安装处理器 {}/{}：{}",
                        index + 1,
                        client_processors.len(),
                        processor.jar
                    ),
                )
                .await;
            self.run_processor(processor, &data, &extra, &java.path)
                .await?;
            // Verify declared outputs so a failed patch cannot look like success.
            for (output_var, hash_var) in &processor.outputs {
                let output =
                    expand_installer_value(output_var, &data, &extra, &self.paths.libraries)?;
                let expected = substitute_data(hash_var, &data, &extra)?;
                if output.is_empty() {
                    continue;
                }
                let path = PathBuf::from(&output);
                if !path.is_file() {
                    bail!("处理器没有生成声明的输出：{output}");
                }
                if !expected.is_empty() {
                    let path_for_check = path.clone();
                    let expected_for_check = expected.clone();
                    let ok = tokio::task::spawn_blocking(move || {
                        crate::download::verify_sync(
                            &path_for_check,
                            Some(&expected_for_check),
                            None,
                        )
                    })
                    .await??;
                    if !ok {
                        bail!("处理器输出校验失败：{output}");
                    }
                }
            }
            progress.step("loader", processor.jar.clone()).await;
        }

        // The patched client jar becomes this version's own client jar, matching the
        // official layout where `versions/<id>/<id>.jar` overrides the inherited jar.
        if let Some(patched) = data.get("PATCHED") {
            let source = PathBuf::from(patched);
            if source.is_file() {
                let version_jar_dir = self.paths.versions_dir().join(&version_id);
                tokio::fs::create_dir_all(&version_jar_dir).await?;
                tokio::fs::copy(&source, version_jar_dir.join(format!("{version_id}.jar"))).await?;
            }
        }

        let lock = InstallLock {
            schema_version: 1,
            game_version: instance.game_version.clone(),
            loader,
            loader_version: Some(source.version.clone()),
            metadata_path: Some(metadata_path),
            installed_at: chrono::Utc::now(),
            java_major: vanilla.java_major,
            mods_managed: true,
        };
        self.write_lock(&instance.id, &lock).await?;
        Ok(lock)
    }

    async fn run_processor(
        &self,
        processor: &Processor,
        data: &HashMap<String, String>,
        extra: &HashMap<String, String>,
        java: &Path,
    ) -> Result<()> {
        let jar_path = self.paths.libraries.join(maven_path(&processor.jar));
        if !jar_path.is_file() {
            bail!("缺少安装处理器：{}", processor.jar);
        }
        let main_class = {
            let jar = jar_path.clone();
            tokio::task::spawn_blocking(move || read_main_class(&jar)).await??
        };
        let mut classpath: Vec<PathBuf> = vec![jar_path.clone()];
        for entry in &processor.classpath {
            let path = self.paths.libraries.join(maven_path(entry));
            if !path.is_file() {
                bail!("缺少处理器依赖：{entry}");
            }
            classpath.push(path);
        }
        let separator = if cfg!(windows) { ";" } else { ":" };
        let classpath_value = classpath
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(separator);
        let mut args: Vec<String> = Vec::new();
        for raw in &processor.args {
            args.push(expand_installer_value(
                raw,
                data,
                extra,
                &self.paths.libraries,
            )?);
        }
        let mut command = tokio::process::Command::new(java);
        command
            .arg("-cp")
            .arg(classpath_value)
            .arg(main_class)
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .with_context(|| format!("无法启动安装处理器 {}", processor.jar))?;
        let mut stdout = String::new();
        let mut stderr = String::new();
        if let Some(mut pipe) = child.stdout.take() {
            let _ = pipe.read_to_string(&mut stdout).await;
        }
        if let Some(mut pipe) = child.stderr.take() {
            let _ = pipe.read_to_string(&mut stderr).await;
        }
        let status = child.wait().await?;
        if !status.success() {
            let tail: String = stderr
                .lines()
                .chain(stdout.lines())
                .rev()
                .take(12)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n");
            bail!(
                "安装处理器失败（退出码 {}）：{}\n{tail}",
                status.code().unwrap_or(-1),
                processor.jar
            );
        }
        Ok(())
    }

    /// Resolve which installer to download, honouring an explicit loader version.
    async fn resolve_installer(
        &self,
        instance: &Instance,
        loader: Loader,
        progress: &Progress,
    ) -> Result<InstallerSource> {
        let requested = instance
            .loader_version
            .clone()
            .filter(|version| !version.is_empty() && version != "latest");
        let version = match requested {
            Some(version) => version,
            None => {
                progress
                    .stage("loader", format!("查询 {} 可用版本", loader.label()))
                    .await;
                self.latest_loader_version(&instance.game_version, loader)
                    .await?
            }
        };
        let url = match loader {
            Loader::Forge => {
                // Forge Maven versions carry the game version ("1.20.1-47.4.26"),
                // but a user may type just the build ("47.4.26").
                let maven_version = forge_maven_version(&instance.game_version, &version);
                return Ok(InstallerSource {
                    url: format!(
                        "{FORGE_MAVEN}/net/minecraftforge/forge/{maven_version}/forge-{maven_version}-installer.jar"
                    ),
                    version: maven_version,
                });
            }
            Loader::NeoForge => format!(
                "{NEOFORGE_MAVEN}/net/neoforged/neoforge/{version}/neoforge-{version}-installer.jar"
            ),
            other => bail!("{} 不使用安装器", other.label()),
        };
        Ok(InstallerSource { url, version })
    }

    /// Available loader versions for a game version, newest first.
    ///
    /// The official Maven metadata is authoritative. Some mirrors serve their own
    /// (older) copy of that file, so when a mirror is configured we also ask the
    /// mirror's own loader listing and merge the two, which keeps discovery honest
    /// about what is actually installable.
    pub async fn loader_versions(&self, game_version: &str, loader: Loader) -> Result<Vec<String>> {
        let metadata_url = match loader {
            Loader::Forge => format!("{FORGE_MAVEN}/net/minecraftforge/forge/maven-metadata.xml"),
            Loader::NeoForge => {
                format!("{NEOFORGE_MAVEN}/net/neoforged/neoforge/maven-metadata.xml")
            }
            other => bail!("{} 的版本来自 Fabric API", other.label()),
        };
        let mut versions: Vec<String> = Vec::new();
        let mut metadata_error = None;
        match self.downloader.fetch_text(&metadata_url).await {
            Ok(xml) => versions.extend(xml_versions(&xml)),
            Err(error) => metadata_error = Some(error),
        }
        if matches!(loader, Loader::Forge) {
            if let Some(mirror) = &self.settings.mirror_base_url {
                if let Ok(listed) = self.mirror_forge_versions(mirror, game_version).await {
                    versions.extend(listed);
                }
            }
        }
        let mut filtered: Vec<String> = versions
            .into_iter()
            .filter(|version| match loader {
                Loader::Forge => version.starts_with(&format!("{game_version}-")),
                _ => version.starts_with(&format!("{}.", neoforge_prefix(game_version))),
            })
            .collect();
        filtered.sort_by(|left, right| compare_version_strings(right, left));
        filtered.dedup();
        if filtered.is_empty() {
            if let Some(error) = metadata_error {
                return Err(error.context(format!(
                    "读取 {} 版本列表失败，可以在新建实例时手动填写版本号",
                    loader.label()
                )));
            }
            let hint = if self.settings.mirror_base_url.is_some() {
                "（镜像的版本列表可能不完整，可手动填写版本号）"
            } else {
                ""
            };
            bail!(
                "{} 没有找到支持 Minecraft {game_version} 的版本{hint}",
                loader.label()
            );
        }
        Ok(filtered)
    }

    /// BMCLAPI-style mirrors expose a per-game build list. It is only consulted
    /// when a mirror is configured, and its entries still get verified against
    /// the official installer hash chain when downloaded.
    async fn mirror_forge_versions(&self, mirror: &str, game_version: &str) -> Result<Vec<String>> {
        let url = format!(
            "{}/forge/minecraft/{}",
            mirror.trim_end_matches('/'),
            urlencoding::encode(game_version)
        );
        let entries: Vec<serde_json::Value> = self.downloader.fetch_json(&url).await?;
        let mut out = Vec::new();
        for entry in entries {
            if let Some(version) = entry.get("version").and_then(|value| value.as_str()) {
                if !version.is_empty() {
                    out.push(format!("{game_version}-{version}"));
                }
            }
        }
        Ok(out)
    }

    async fn latest_loader_version(&self, game_version: &str, loader: Loader) -> Result<String> {
        let versions = self.loader_versions(game_version, loader).await?;
        versions.into_iter().next().ok_or_else(|| {
            anyhow!(
                "{} 没有支持 Minecraft {game_version} 的版本",
                loader.label()
            )
        })
    }
}

/// NeoForge version prefixes follow the game version: 1.21.1 -> 21.1, 1.20.2 -> 20.2.
pub fn neoforge_prefix(game_version: &str) -> String {
    let parts: Vec<&str> = game_version.split('.').collect();
    match parts.as_slice() {
        ["1", minor, patch, ..] => format!("{minor}.{patch}"),
        ["1", minor] => format!("{minor}.0"),
        [minor, patch, ..] => format!("{minor}.{patch}"),
        _ => game_version.to_string(),
    }
}

/// Forge Maven versions embed the game version: "1.20.1" + "47.4.26" -> "1.20.1-47.4.26".
pub fn forge_maven_version(game_version: &str, requested: &str) -> String {
    let prefix = format!("{game_version}-");
    if requested.starts_with(&prefix) || requested.contains('-') {
        requested.to_string()
    } else {
        format!("{prefix}{requested}")
    }
}

fn compare_version_strings(left: &str, right: &str) -> std::cmp::Ordering {
    let split = |value: &str| -> Vec<u32> {
        value
            .split(|character: char| !character.is_ascii_digit())
            .filter(|part| !part.is_empty())
            .map(|part| part.parse::<u32>().unwrap_or(0))
            .collect()
    };
    let a = split(left);
    let b = split(right);
    for index in 0..a.len().max(b.len()) {
        let x = a.get(index).copied().unwrap_or(0);
        let y = b.get(index).copied().unwrap_or(0);
        match x.cmp(&y) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

// ---------------------------------------------------------------------------
// Jar helpers
// ---------------------------------------------------------------------------

pub fn xml_versions(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<version>") {
        let after = &rest[start + "<version>".len()..];
        let Some(end) = after.find("</version>") else {
            break;
        };
        out.push(after[..end].trim().to_string());
        rest = &after[end + "</version>".len()..];
    }
    out
}

pub fn read_jar_entry(jar: &Path, name: &str) -> Result<Option<Vec<u8>>> {
    let file = std::fs::File::open(jar)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let wanted = name.trim_start_matches('/');
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        if entry.name().trim_start_matches('/') == wanted {
            let mut buffer = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut buffer)?;
            return Ok(Some(buffer));
        }
    }
    Ok(None)
}

fn extract_jar_entry(jar: &Path, name: &str, destination: &Path) -> Result<()> {
    let bytes = read_jar_entry(jar, name)?.ok_or_else(|| anyhow!("安装器缺少内嵌文件：{name}"))?;
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(destination, bytes)?;
    Ok(())
}

/// Extract every entry under a prefix, mapping it onto the destination root.
fn extract_jar_prefix(jar: &Path, prefix: &str, destination: &Path) -> Result<Vec<PathBuf>> {
    let file = std::fs::File::open(jar)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let mut written = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        if !name.starts_with(prefix) {
            continue;
        }
        let relative = name.trim_start_matches(prefix);
        if relative.is_empty() {
            continue;
        }
        let target = destination.join(relative);
        if !target.starts_with(destination) {
            bail!("安装器包含越界路径：{name}");
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&target)?;
        std::io::copy(&mut entry, &mut out)?;
        written.push(target);
    }
    Ok(written)
}

fn read_main_class(jar: &Path) -> Result<String> {
    let bytes = read_jar_entry(jar, "META-INF/MANIFEST.MF")?
        .ok_or_else(|| anyhow!("处理器缺少 MANIFEST.MF：{}", jar.display()))?;
    let text = String::from_utf8_lossy(&bytes).to_string();
    // Manifest lines are folded at 72 bytes with a leading space.
    let mut unfolded = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix(' ') {
            unfolded.push_str(rest);
        } else {
            unfolded.push('\n');
            unfolded.push_str(line);
        }
    }
    for line in unfolded.lines() {
        if let Some(value) = line.strip_prefix("Main-Class:") {
            let value = value.trim();
            if !value.is_empty() {
                return Ok(value.to_string());
            }
        }
    }
    bail!("处理器没有声明 Main-Class：{}", jar.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neoforge_prefixes_match_official_naming() {
        assert_eq!(neoforge_prefix("1.21.1"), "21.1");
        assert_eq!(neoforge_prefix("1.20.2"), "20.2");
        assert_eq!(neoforge_prefix("1.20.6"), "20.6");
        assert_eq!(neoforge_prefix("1.21"), "21.0");
    }

    #[test]
    fn forge_versions_are_normalised_once() {
        assert_eq!(forge_maven_version("1.20.1", "47.4.26"), "1.20.1-47.4.26");
        assert_eq!(
            forge_maven_version("1.20.1", "1.20.1-47.4.26"),
            "1.20.1-47.4.26"
        );
        assert_eq!(
            forge_maven_version("1.12.2", "14.23.5.2864"),
            "1.12.2-14.23.5.2864"
        );
    }

    #[test]
    fn xml_versions_are_extracted_in_order() {
        let xml = "<metadata><versioning><versions><version>1.20.1-47.4.0</version><version>1.20.1-47.4.26</version></versions></versioning></metadata>";
        assert_eq!(
            xml_versions(xml),
            vec!["1.20.1-47.4.0".to_string(), "1.20.1-47.4.26".to_string()]
        );
    }

    #[test]
    fn version_comparison_is_numeric() {
        assert_eq!(
            compare_version_strings("1.20.1-47.4.26", "1.20.1-47.4.9"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            compare_version_strings("21.1.255", "21.1.9"),
            std::cmp::Ordering::Greater
        );
    }
}
