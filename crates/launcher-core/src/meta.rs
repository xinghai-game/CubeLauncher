use crate::rules::{maven_identity, Platform};
use crate::types::*;
use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::path::PathBuf;

/// A version with its whole `inheritsFrom` chain merged into one document.
#[derive(Debug, Clone)]
pub struct ResolvedVersion {
    pub meta: VersionMeta,
    /// Version id that owns the client jar (the root of the inheritance chain).
    pub jar_version: String,
    /// Version ids from parent to child, useful for diagnostics.
    pub chain: Vec<String>,
}

pub struct VersionResolver<'a> {
    pub core: &'a crate::LauncherCore,
}

/// April Fools releases. They are published as ordinary snapshots upstream, but
/// HMCL — and this launcher — list them separately because they are jokes, not
/// playable versions. Kept as an explicit list rather than a heuristic: guessing
/// would either hide real snapshots or promote fake ones.
pub const APRIL_FOOLS_VERSIONS: &[&str] = &[
    "15w14a",
    "1.RV-Pre1",
    "3D Shareware v1.34",
    "20w14infinite",
    "22w13oneblockatatime",
    "23w13a_or_b",
    "24w14potato",
    "25w14craftmine",
];

/// How long a cached version list is considered current. Older copies are
/// refreshed in the background of the next request, and kept as a fallback when
/// the refresh fails.
pub const MANIFEST_TTL_HOURS: i64 = 6;

/// Category of a manifest entry, derived from the upstream `type` plus the
/// known April Fools ids.
pub fn classify_version(id: &str, version_type: &str) -> VersionKind {
    if APRIL_FOOLS_VERSIONS.contains(&id) {
        return VersionKind::AprilFools;
    }
    match version_type {
        "release" => VersionKind::Release,
        "old_beta" => VersionKind::OldBeta,
        "old_alpha" => VersionKind::OldAlpha,
        // Upstream also uses `snapshot`; anything unknown is treated as a
        // development build so it stays visible instead of disappearing.
        _ => VersionKind::Snapshot,
    }
}

impl crate::LauncherCore {
    fn manifest_cache_path(&self) -> PathBuf {
        self.paths.metadata.join("version_manifest_v2.json")
    }

    /// Read the on-disk manifest together with its modification time.
    pub async fn cached_manifest(&self) -> Option<(Manifest, DateTime<Utc>)> {
        let path = self.manifest_cache_path();
        let bytes = tokio::fs::read(&path).await.ok()?;
        let manifest = serde_json::from_slice::<Manifest>(&bytes).ok()?;
        let fetched_at = tokio::fs::metadata(&path)
            .await
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .map(DateTime::<Utc>::from)
            .unwrap_or_else(Utc::now);
        Some((manifest, fetched_at))
    }

    async fn fetch_manifest(&self) -> Result<(Manifest, ManifestSource)> {
        let (bytes, mirrored) = self
            .downloader
            .fetch_bytes_with_source(MANIFEST_URL)
            .await?;
        let manifest: Manifest = serde_json::from_slice(&bytes).context("解析版本清单失败")?;
        crate::atomic_write(&self.manifest_cache_path(), &bytes).await?;
        let source = if mirrored {
            ManifestSource::Mirror
        } else {
            ManifestSource::Official
        };
        Ok((manifest, source))
    }

    /// Version list with its provenance.
    ///
    /// Without `force` a fresh cache is served as-is, a stale cache triggers one
    /// refresh attempt, and a failed refresh still returns the stale copy instead
    /// of breaking the UI. With `force` the network is always used, so the caller
    /// sees the failure.
    pub async fn manifest_source(
        &self,
        force: bool,
    ) -> Result<(Manifest, ManifestSource, Option<DateTime<Utc>>)> {
        if force && self.downloader.offline() {
            bail!("严格离线模式下无法刷新版本目录");
        }
        if let Some((cached, fetched_at)) = self.cached_manifest().await {
            let age = Utc::now().signed_duration_since(fetched_at);
            if !force && age < chrono::Duration::hours(MANIFEST_TTL_HOURS) {
                return Ok((cached, ManifestSource::Cache, Some(fetched_at)));
            }
            match self.fetch_manifest().await {
                Ok((manifest, source)) => return Ok((manifest, source, Some(Utc::now()))),
                Err(error) if !force => {
                    tracing::warn!("刷新版本清单失败，继续使用缓存：{error:#}");
                    return Ok((cached, ManifestSource::Cache, Some(fetched_at)));
                }
                Err(error) => return Err(error),
            }
        }
        let (manifest, source) = self.fetch_manifest().await?;
        Ok((manifest, source, Some(Utc::now())))
    }

    /// Version list, from cache unless `force` is set. Works fully offline from cache.
    pub async fn manifest(&self, force: bool) -> Result<Manifest> {
        Ok(self.manifest_source(force).await?.0)
    }

    /// The version list as the UI consumes it: categories, counts and provenance.
    pub async fn catalog(&self, force: bool) -> Result<VersionCatalog> {
        let (manifest, source, fetched_at) = self.manifest_source(force).await?;
        let versions: Vec<CatalogVersion> = manifest
            .versions
            .iter()
            .map(|version| CatalogVersion {
                id: version.id.clone(),
                version_type: version.version_type.clone(),
                kind: classify_version(&version.id, &version.version_type),
                release_time: version.release_time.clone(),
                url: version.url.clone(),
                sha1: version.sha1.clone(),
            })
            .collect();
        Ok(VersionCatalog {
            total: versions.len(),
            latest: manifest.latest,
            versions,
            cached: source == ManifestSource::Cache,
            source,
            fetched_at,
        })
    }

    /// Java major a version needs. Reads the version document only when it is
    /// already cached, so this never blocks on the network; otherwise it falls
    /// back to the tested version table and says so.
    pub async fn required_java_major(&self, id: &str) -> JavaRequirement {
        let path = self.paths.versions_dir().join(format!("{id}.json"));
        if let Ok(meta) = self.version_meta_from_path(&path).await {
            if let Some(major) = meta
                .java_version
                .as_ref()
                .and_then(|value| value.major_version)
            {
                return JavaRequirement {
                    major,
                    source: "metadata".into(),
                };
            }
        }
        JavaRequirement {
            major: crate::rules::require_java_major(id, None),
            source: "fallback".into(),
        }
    }

    pub async fn version_url(&self, id: &str) -> Result<(String, Option<String>)> {
        let manifest = self.manifest(false).await?;
        let entry = manifest
            .versions
            .iter()
            .find(|version| version.id == id)
            .ok_or_else(|| anyhow!("版本清单中找不到 {id}"))?;
        Ok((entry.url.clone(), Some(entry.sha1.clone())))
    }

    /// Raw version document for one id, cached on disk and verified against the manifest sha1.
    pub async fn version_meta(&self, id: &str, force: bool) -> Result<VersionMeta> {
        let path = self
            .paths
            .metadata
            .join("versions")
            .join(format!("{id}.json"));
        let cached = tokio::fs::read(&path).await.ok();
        if !force {
            if let Some(bytes) = &cached {
                if let Ok(meta) = serde_json::from_slice::<VersionMeta>(bytes) {
                    return Ok(meta);
                }
            }
        }
        let (url, sha1) = self.version_url(id).await?;
        let bytes = self.downloader.fetch_bytes(&url).await?;
        if let Some(expected) = sha1 {
            use sha1::{Digest, Sha1};
            let mut hasher = Sha1::new();
            hasher.update(&bytes);
            if hex::encode(hasher.finalize()) != expected.to_lowercase() {
                bail!("版本 {id} 的元数据校验失败，请重试或检查网络");
            }
        }
        let meta: VersionMeta = serde_json::from_slice(&bytes)
            .with_context(|| format!("解析版本 {id} 的元数据失败"))?;
        crate::atomic_write(&path, &bytes).await?;
        Ok(meta)
    }

    /// Read a version document that is already on disk (loader profiles, installed versions).
    pub async fn version_meta_from_path(&self, path: &std::path::Path) -> Result<VersionMeta> {
        let bytes = tokio::fs::read(path)
            .await
            .with_context(|| format!("读取版本文件失败：{}", path.display()))?;
        serde_json::from_slice(&bytes)
            .with_context(|| format!("解析本地版本文件失败：{}", path.display()))
    }

    /// Read a version document that is already on disk, preferring the extra
    /// directories an imported `.minecraft` brings along. Falling back to the
    /// launcher's own cache and then the network is what keeps an imported
    /// installation launchable offline.
    async fn version_meta_in(&self, local: &[PathBuf], id: &str) -> Result<VersionMeta> {
        for root in local {
            let path = root.join(id).join(format!("{id}.json"));
            if let Ok(meta) = self.version_meta_from_path(&path).await {
                return Ok(meta);
            }
        }
        self.version_meta(id, false).await
    }

    /// Merge a version with its inheritance chain.
    pub async fn resolve_version(&self, id: &str) -> Result<ResolvedVersion> {
        self.resolve_version_in(&[], id).await
    }

    /// Merge a version with its inheritance chain, reading documents out of `local`
    /// (an imported game directory) before the launcher's own metadata.
    pub async fn resolve_version_in(&self, local: &[PathBuf], id: &str) -> Result<ResolvedVersion> {
        let root = self.version_meta_in(local, id).await?;
        self.resolve_from_meta_in(local, root, id.to_string()).await
    }

    /// Merge a version document that may inherit from official versions.
    pub async fn resolve_from_meta(
        &self,
        entry: VersionMeta,
        entry_id: String,
    ) -> Result<ResolvedVersion> {
        self.resolve_from_meta_in(&[], entry, entry_id).await
    }

    /// Merge a version document with its chain, resolving parents through `local`
    /// first so an imported installation never needs the network to start.
    pub async fn resolve_from_meta_in(
        &self,
        local: &[PathBuf],
        entry: VersionMeta,
        entry_id: String,
    ) -> Result<ResolvedVersion> {
        let mut chain = vec![(entry_id, entry)];
        let mut seen: Vec<String> = chain.iter().map(|(id, _)| id.clone()).collect();
        while let Some(parent_id) = chain
            .last()
            .and_then(|(_, meta)| meta.inherits_from.clone())
        {
            if seen.contains(&parent_id) {
                bail!("版本继承出现循环：{}", seen.join(" -> "));
            }
            if seen.len() > 16 {
                bail!("版本继承层级过深：{}", seen.join(" -> "));
            }
            let parent = self.version_meta_in(local, &parent_id).await?;
            seen.push(parent_id.clone());
            chain.push((parent_id, parent));
        }
        let jar_version = chain
            .iter()
            .rev()
            .find(|(_, meta)| {
                meta.downloads
                    .as_ref()
                    .map(|downloads| downloads.contains_key("client"))
                    .unwrap_or(false)
            })
            .map(|(id, _)| id.clone())
            .unwrap_or_else(|| chain.last().map(|(id, _)| id.clone()).unwrap_or_default());
        let mut order: Vec<String> = chain.iter().map(|(id, _)| id.clone()).collect();
        order.reverse();
        let mut documents: Vec<VersionMeta> = chain.into_iter().map(|(_, meta)| meta).collect();
        documents.reverse();
        let mut merged = documents
            .first()
            .cloned()
            .ok_or_else(|| anyhow!("版本链为空"))?;
        for child in documents.into_iter().skip(1) {
            merged = merge_documents(merged, child);
        }
        merged.inherits_from = None;
        Ok(ResolvedVersion {
            meta: merged,
            jar_version,
            chain: order,
        })
    }
}

/// Merge a child version document over its parent, following launcher semantics:
/// scalars are overridden, arguments are appended parent-then-child, and libraries
/// are merged with the child winning for the same Maven identity and classifier.
pub fn merge_documents(mut parent: VersionMeta, child: VersionMeta) -> VersionMeta {
    if child.id != parent.id && !child.id.is_empty() {
        parent.id = child.id;
    }
    if child.main_class.is_some() {
        parent.main_class = child.main_class.clone();
    }
    if child.assets.is_some() {
        parent.assets = child.assets.clone();
    }
    if child.asset_index.is_some() {
        parent.asset_index = child.asset_index.clone();
    }
    if child.java_version.is_some() {
        parent.java_version = child.java_version.clone();
    }
    if child.logging.is_some() {
        parent.logging = child.logging.clone();
    }
    if child.downloads.is_some() {
        match (&mut parent.downloads, child.downloads.clone()) {
            (Some(existing), Some(incoming)) => {
                for (key, value) in incoming {
                    existing.insert(key, value);
                }
            }
            (slot, incoming) => *slot = incoming,
        }
    }
    if child.jar.is_some() {
        parent.jar = child.jar.clone();
    }
    // Legacy argument string is replaced wholesale when the child declares it.
    if child.minecraft_arguments.is_some() {
        parent.minecraft_arguments = child.minecraft_arguments.clone();
    }
    match (&mut parent.arguments, child.arguments.clone()) {
        (Some(existing), Some(incoming)) => {
            for (slot, extra) in [
                (&mut existing.game, incoming.game),
                (&mut existing.jvm, incoming.jvm),
                (&mut existing.default_user_jvm, incoming.default_user_jvm),
            ] {
                match (slot, extra) {
                    (Some(current), Some(more)) => current.extend(more),
                    (slot, more) => *slot = more,
                }
            }
        }
        (slot, incoming) => {
            if incoming.is_some() {
                *slot = incoming;
            }
        }
    }
    let mut libraries = parent.libraries;
    for library in child.libraries {
        let identity = maven_identity(&library.name);
        if let Some(index) = libraries
            .iter()
            .position(|existing| maven_identity(&existing.name) == identity)
        {
            libraries[index] = library;
        } else {
            libraries.push(library);
        }
    }
    parent.libraries = libraries;
    parent
}

/// Values substituted into `${...}` placeholders in launch arguments.
#[derive(Debug, Clone, Default)]
pub struct LaunchVariables(pub HashMap<String, String>);

impl LaunchVariables {
    pub fn insert(&mut self, key: &str, value: impl Into<String>) {
        self.0.insert(key.to_string(), value.into());
    }
    pub fn get(&self, key: &str) -> Option<&String> {
        self.0.get(key)
    }
}

/// Expand an argument list, dropping entries whose rules do not match and
/// substituting known placeholders. Unknown placeholders are reported instead of
/// silently passing a broken argument to the game.
pub fn expand_arguments(
    values: &[ArgumentValue],
    platform: &Platform,
    env: &crate::rules::RuleEnv,
    variables: &LaunchVariables,
) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for value in values {
        match value {
            ArgumentValue::Text(text) => out.push(substitute(text, variables)?),
            ArgumentValue::Conditional { rules, value } => {
                if crate::rules::rules_allow(
                    rules.as_deref(),
                    platform,
                    &env.features,
                    env.os_known,
                ) {
                    match value {
                        ArgumentText::One(text) => out.push(substitute(text, variables)?),
                        ArgumentText::Many(items) => {
                            for text in items {
                                out.push(substitute(text, variables)?);
                            }
                        }
                    }
                }
            }
            // Unknown shapes are reported but do not stop a launch: dropping one
            // unrecognized entry is better than refusing to start the game.
            ArgumentValue::Unknown(raw) => {
                tracing::warn!("忽略无法识别的启动参数条目：{raw}");
            }
        }
    }
    Ok(out)
}

/// Replace `${name}` placeholders, erroring on anything we do not know so the
/// failure is actionable instead of a mysterious game crash.
pub fn substitute(text: &str, variables: &LaunchVariables) -> Result<String> {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        result.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find('}')
            .ok_or_else(|| anyhow!("参数中的占位符缺少右花括号：{text}"))?;
        let key = &after[..end];
        match variables.get(key) {
            Some(value) => result.push_str(value),
            None => bail!("版本元数据使用了启动器尚不支持的占位符 ${{{key}}}"),
        }
        rest = &after[end + 1..];
    }
    result.push_str(rest);
    Ok(result)
}

/// Split a legacy `minecraftArguments` string, honouring single and double quotes.
pub fn split_legacy_arguments(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for character in input.chars() {
        match (quote, character) {
            (None, '"') | (None, '\'') => quote = Some(character),
            (Some(open), value) if value == open => quote = None,
            (None, value) if value.is_whitespace() => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            (_, value) => current.push(value),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Arguments, JavaVersion, LibraryDownloads};

    fn library(name: &str, sha1: &str) -> Library {
        Library {
            name: name.into(),
            downloads: Some(LibraryDownloads {
                artifact: Some(DownloadInfo {
                    sha1: Some(sha1.into()),
                    ..Default::default()
                }),
                classifiers: None,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn child_library_replaces_parent_same_identity() {
        let parent = VersionMeta {
            id: "parent".into(),
            libraries: vec![library("org.example:lib:1.0", "aaa")],
            ..Default::default()
        };
        let child = VersionMeta {
            id: "child".into(),
            libraries: vec![library("org.example:lib:2.0", "bbb")],
            ..Default::default()
        };
        let merged = merge_documents(parent, child);
        assert_eq!(merged.libraries.len(), 1);
        assert_eq!(merged.libraries[0].name, "org.example:lib:2.0");
    }

    #[test]
    fn different_classifier_is_not_treated_as_a_conflict() {
        let parent = VersionMeta {
            id: "parent".into(),
            libraries: vec![library("org.example:lib:1.0", "aaa")],
            ..Default::default()
        };
        let child = VersionMeta {
            id: "child".into(),
            libraries: vec![library("org.example:lib:1.0:natives-linux", "bbb")],
            ..Default::default()
        };
        let merged = merge_documents(parent, child);
        assert_eq!(merged.libraries.len(), 2);
    }

    #[test]
    fn arguments_are_appended_parent_then_child() {
        let parent = VersionMeta {
            id: "parent".into(),
            arguments: Some(Arguments {
                jvm: Some(vec![ArgumentValue::Text("-XstartOnFirstThread".into())]),
                game: Some(vec![ArgumentValue::Text("--username".into())]),
                default_user_jvm: Some(vec![ArgumentValue::Text("-Xms2G".into())]),
            }),
            ..Default::default()
        };
        let child = VersionMeta {
            id: "child".into(),
            arguments: Some(Arguments {
                jvm: Some(vec![ArgumentValue::Text("-Dfoo=bar".into())]),
                game: Some(vec![ArgumentValue::Text("--version".into())]),
                default_user_jvm: Some(vec![ArgumentValue::Text("-Xmx4G".into())]),
            }),
            ..Default::default()
        };
        let merged = merge_documents(parent, child);
        let arguments = merged.arguments.expect("arguments");
        let jvm = arguments.jvm.expect("jvm");
        assert_eq!(jvm.len(), 2);
        match &jvm[0] {
            ArgumentValue::Text(text) => assert_eq!(text, "-XstartOnFirstThread"),
            _ => panic!("expected text argument"),
        }
        match &jvm[1] {
            ArgumentValue::Text(text) => assert_eq!(text, "-Dfoo=bar"),
            _ => panic!("expected text argument"),
        }
        let defaults = arguments.default_user_jvm.expect("default-user-jvm");
        assert_eq!(defaults.len(), 2);
        match &defaults[1] {
            ArgumentValue::Text(text) => assert_eq!(text, "-Xmx4G"),
            _ => panic!("expected text argument"),
        }
    }

    fn platform() -> crate::rules::Platform {
        crate::rules::Platform {
            os: "linux".into(),
            arch: "x86_64".into(),
            os_version: String::new(),
        }
    }

    fn environment() -> crate::rules::RuleEnv {
        crate::rules::RuleEnv::new(false, false)
    }

    /// Regression for the 26.x metadata: `default-user-jvm[0]` is published as
    /// `{"value": [...]}` without `rules`, which used to reject the whole
    /// document with "data did not match any variant of untagged enum
    /// ArgumentValue at line 1 column 155".
    #[test]
    fn modern_metadata_without_rules_parses() {
        let document = serde_json::json!({
            "id": "26.3",
            "javaVersion": { "component": "java-runtime-epsilon", "majorVersion": 25 },
            "arguments": {
                "default-user-jvm": [
                    { "value": ["-Xms2G", "-Xmx4G", "-XX:+UseCompactObjectHeaders"] },
                    { "rules": [{ "action": "allow", "os": { "name": "osx" } }],
                      "value": ["-XX:+UseZGC"] }
                ],
                "game": ["--username", "${auth_player_name}"],
                "jvm": [
                    { "rules": [{ "action": "allow", "os": { "name": "linux" } }], "value": "-Xss1M" },
                    { "rules": [{ "action": "allow", "os": { "name": "windows" } }],
                      "value": ["-Dos.name=Windows"] }
                ]
            }
        });
        let meta: VersionMeta = serde_json::from_value(document).expect("parse 26.x metadata");
        assert_eq!(meta.id, "26.3");
        assert_eq!(
            meta.java_version.and_then(|value| value.major_version),
            Some(25)
        );
        let arguments = meta.arguments.expect("arguments");
        assert_eq!(
            arguments.default_user_jvm.as_ref().map(Vec::len),
            Some(2),
            "default-user-jvm must survive parsing"
        );
        let mut variables = LaunchVariables::default();
        variables.insert("auth_player_name", "Steve");
        let jvm = expand_arguments(
            arguments.jvm.as_deref().unwrap_or_default(),
            &platform(),
            &environment(),
            &variables,
        )
        .expect("expand jvm arguments");
        assert_eq!(jvm, vec!["-Xss1M"]);
        let game = expand_arguments(
            arguments.game.as_deref().unwrap_or_default(),
            &platform(),
            &environment(),
            &variables,
        )
        .expect("expand game arguments");
        assert_eq!(game, vec!["--username", "Steve"]);
    }

    /// A rule-less entry has no condition to evaluate, so it always applies.
    #[test]
    fn rule_less_arguments_are_always_included() {
        let values: Vec<ArgumentValue> = serde_json::from_value(serde_json::json!([
            { "value": ["-Xms2G", "-Xmx4G"] },
            { "value": "-XX:+UseStringDeduplication" }
        ]))
        .expect("rule-less entries");
        let expanded = expand_arguments(
            &values,
            &platform(),
            &environment(),
            &LaunchVariables::default(),
        )
        .expect("expand");
        assert_eq!(
            expanded,
            vec!["-Xms2G", "-Xmx4G", "-XX:+UseStringDeduplication"]
        );
    }

    /// An unknown shape must not stop the launch; it is skipped and logged.
    #[test]
    fn unknown_argument_shape_is_skipped_not_fatal() {
        let values: Vec<ArgumentValue> = serde_json::from_value(serde_json::json!([
            { "when": "some-future-revision", "args": [1, 2] },
            "-Xss1M"
        ]))
        .expect("unknown shapes must still parse");
        assert!(matches!(values[0], ArgumentValue::Unknown(_)));
        let expanded = expand_arguments(
            &values,
            &platform(),
            &environment(),
            &LaunchVariables::default(),
        )
        .expect("expand");
        assert_eq!(expanded, vec!["-Xss1M"]);
    }

    #[test]
    fn java_version_from_child_wins() {
        let parent = VersionMeta {
            id: "parent".into(),
            java_version: Some(JavaVersion {
                component: None,
                major_version: Some(17),
            }),
            ..Default::default()
        };
        let child = VersionMeta {
            id: "child".into(),
            java_version: Some(JavaVersion {
                component: Some("java-runtime-epsilon".into()),
                major_version: Some(25),
            }),
            ..Default::default()
        };
        let merged = merge_documents(parent, child);
        assert_eq!(merged.java_version.and_then(|v| v.major_version), Some(25));
    }

    #[test]
    fn legacy_arguments_split_respects_quotes() {
        assert_eq!(
            split_legacy_arguments("--username Steve --width \"1280\" --height '720'"),
            vec!["--username", "Steve", "--width", "1280", "--height", "720"]
        );
    }

    #[test]
    fn unknown_placeholder_is_an_error() {
        let variables = LaunchVariables::default();
        assert!(substitute("${auth_player_name}", &variables).is_err());
        let mut variables = LaunchVariables::default();
        variables.insert("auth_player_name", "Steve");
        assert_eq!(
            substitute("${auth_player_name}", &variables).expect("known"),
            "Steve"
        );
    }

    #[test]
    fn version_kinds_follow_the_manifest_type() {
        assert_eq!(classify_version("1.21.1", "release"), VersionKind::Release);
        assert_eq!(
            classify_version("26.4-snapshot-2", "snapshot"),
            VersionKind::Snapshot
        );
        assert_eq!(classify_version("b1.8.1", "old_beta"), VersionKind::OldBeta);
        assert_eq!(
            classify_version("a1.2.6", "old_alpha"),
            VersionKind::OldAlpha
        );
        // April Fools releases are plain snapshots upstream.
        assert_eq!(
            classify_version("24w14potato", "snapshot"),
            VersionKind::AprilFools
        );
        assert_eq!(
            classify_version("3D Shareware v1.34", "snapshot"),
            VersionKind::AprilFools
        );
        // Unknown upstream types stay visible as development builds.
        assert_eq!(
            classify_version("mystery", "experiment"),
            VersionKind::Snapshot
        );
    }

    #[test]
    fn april_fools_ids_are_unique() {
        let mut ids = APRIL_FOOLS_VERSIONS.to_vec();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), APRIL_FOOLS_VERSIONS.len());
        assert!(APRIL_FOOLS_VERSIONS.contains(&"1.RV-Pre1"));
    }

    #[tokio::test]
    async fn required_java_major_prefers_cached_metadata() {
        let root = std::env::temp_dir().join(format!("cube-java-{}", std::process::id()));
        let settings = AppSettings {
            data_dir: root.clone(),
            offline_mode: true,
            ..Default::default()
        };
        let core = crate::LauncherCore::new(settings).await.expect("core");
        // Nothing on disk: the tested fallback table answers.
        let fallback = core.required_java_major("1.20.1").await;
        assert_eq!(fallback.major, 17);
        assert_eq!(fallback.source, "fallback");
        // A cached version document wins, without touching the network.
        let path = core.paths.versions_dir().join("1.20.1.json");
        let document = serde_json::json!({
            "id": "1.20.1",
            "javaVersion": { "majorVersion": 21 }
        });
        crate::atomic_write(&path, document.to_string().as_bytes())
            .await
            .expect("write version document");
        let declared = core.required_java_major("1.20.1").await;
        assert_eq!(declared.major, 21);
        assert_eq!(declared.source, "metadata");
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn catalog_classifies_a_cached_manifest() {
        let root = std::env::temp_dir().join(format!("cube-catalog-{}", std::process::id()));
        let settings = AppSettings {
            data_dir: root.clone(),
            offline_mode: true,
            ..Default::default()
        };
        let core = crate::LauncherCore::new(settings).await.expect("core");
        let manifest = serde_json::json!({
            "latest": { "release": "1.21.1", "snapshot": "24w14potato" },
            "versions": [
                { "id": "1.21.1", "type": "release", "releaseTime": "2024-08-08T12:00:00+00:00",
                  "url": "https://example.invalid/1.21.1.json", "sha1": "aa" },
                { "id": "24w14potato", "type": "snapshot", "releaseTime": "2024-04-01T12:00:00+00:00",
                  "url": "https://example.invalid/potato.json", "sha1": "bb" },
                { "id": "b1.8.1", "type": "old_beta", "releaseTime": "2011-09-15T12:00:00+00:00",
                  "url": "https://example.invalid/b1.8.1.json", "sha1": "cc" }
            ]
        });
        let path = core.paths.metadata.join("version_manifest_v2.json");
        crate::atomic_write(&path, manifest.to_string().as_bytes())
            .await
            .expect("write manifest");
        // A fresh cache is served from disk even in strict offline mode.
        let catalog = core.catalog(false).await.expect("catalog");
        assert_eq!(catalog.total, 3);
        assert!(catalog.cached);
        assert_eq!(catalog.source, ManifestSource::Cache);
        assert_eq!(catalog.latest.release, "1.21.1");
        assert_eq!(catalog.versions[0].kind, VersionKind::Release);
        assert_eq!(catalog.versions[1].kind, VersionKind::AprilFools);
        assert_eq!(catalog.versions[2].kind, VersionKind::OldBeta);
        assert!(catalog.fetched_at.is_some());
        // Forcing a refresh in offline mode must fail loudly instead of lying.
        assert!(core.catalog(true).await.is_err());
        let _ = tokio::fs::remove_dir_all(&root).await;
    }
}
