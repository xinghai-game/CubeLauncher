use crate::types::{Library, Rule};
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::process::Command;

/// The platform the launcher targets. `arch` describes the JVM that will run the game,
/// not the process running the launcher, because library rules follow the chosen runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    pub os: String,
    pub arch: String,
    pub os_version: String,
}

impl Platform {
    /// Platform derived from the running launcher process.
    pub fn current() -> Self {
        Self {
            os: current_os().to_string(),
            arch: current_arch().to_string(),
            os_version: detect_os_version(),
        }
    }

    /// Platform as seen by a specific Java runtime, used when the user picks a
    /// runtime whose architecture differs from the launcher's (for example Rosetta).
    pub fn for_java(java_arch: Option<&str>) -> Self {
        let mut platform = Self::current();
        if let Some(arch) = java_arch {
            platform.arch = normalize_arch(arch).to_string();
        }
        platform
    }
}

pub fn current_os() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    }
}

pub fn current_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86" => "x86",
        _ => "x86_64",
    }
}

pub fn normalize_arch(arch: &str) -> &'static str {
    match arch {
        "aarch64" | "arm64" => "arm64",
        "x86" | "i386" | "i686" | "x86_32" => "x86",
        "x86_64" | "amd64" | "x64" => "x86_64",
        _ => "x86_64",
    }
}

pub fn split_maven_coordinate(name: &str) -> (String, String, String, Option<String>) {
    let parts: Vec<&str> = name.split(':').collect();
    let group = parts.first().copied().unwrap_or_default().to_string();
    let artifact = parts.get(1).copied().unwrap_or_default().to_string();
    let version = parts.get(2).copied().unwrap_or_default().to_string();
    let classifier = parts.get(3).map(|v| v.to_string());
    (group, artifact, version, classifier)
}

pub fn maven_path(name: &str) -> String {
    let (group, artifact, version, classifier) = split_maven_coordinate(name);
    if group.is_empty() || artifact.is_empty() || version.is_empty() {
        return format!("{}.jar", name.replace(':', "-"));
    }
    let suffix = classifier.map(|c| format!("-{c}")).unwrap_or_default();
    format!(
        "{}/{}/{}/{}-{}{}.jar",
        group.replace('.', "/"),
        artifact,
        version,
        artifact,
        version,
        suffix
    )
}

/// Maven identity used for inheritance conflict resolution.
pub fn maven_identity(name: &str) -> String {
    let (group, artifact, _version, classifier) = split_maven_coordinate(name);
    format!("{group}:{artifact}:{}", classifier.unwrap_or_default())
}

fn detect_os_version() -> String {
    if cfg!(windows) {
        if let Ok(out) = Command::new("cmd").args(["/c", "ver"]).output() {
            let text = String::from_utf8_lossy(&out.stdout).to_string();
            if let Some(start) = text.find(|c: char| c.is_ascii_digit()) {
                let digits: String = text[start..]
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '.')
                    .collect();
                if !digits.is_empty() {
                    return digits;
                }
            }
        }
        String::new()
    } else if cfg!(target_os = "macos") {
        if let Ok(out) = Command::new("sw_vers").arg("-productVersion").output() {
            return String::from_utf8_lossy(&out.stdout).trim().to_string();
        }
        String::new()
    } else {
        String::new()
    }
}

fn compare_versions(left: &str, right: &str) -> std::cmp::Ordering {
    let parse = |value: &str| -> Vec<u32> {
        value
            .split(|c: char| !c.is_ascii_digit())
            .filter(|part| !part.is_empty())
            .map(|part| part.parse::<u32>().unwrap_or(0))
            .collect()
    };
    let a = parse(left);
    let b = parse(right);
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

/// Mojang writes `os.version` as a Java regular expression (`^10\.`). We support the
/// subset actually used by published metadata instead of pulling in a regex engine.
fn matches_version_pattern(pattern: &str, version: &str) -> bool {
    let mut pattern = pattern.trim();
    let anchored_start = pattern.starts_with('^');
    let anchored_end = pattern.ends_with('$');
    if anchored_start {
        pattern = &pattern[1..];
    }
    if anchored_end && !pattern.is_empty() {
        pattern = &pattern[..pattern.len() - 1];
    }
    let literal = pattern.replace("\\\\", "\\").replace("\\.", ".");
    if anchored_start && anchored_end {
        literal == version
    } else if anchored_start {
        version.starts_with(&literal)
    } else if anchored_end {
        version.ends_with(&literal)
    } else {
        version.contains(&literal)
    }
}

/// Evaluate the Mojang rules list. No rules means allowed; otherwise the last
/// matching rule wins, and every condition inside one rule must hold.
pub fn rules_allow(
    rules: Option<&[Rule]>,
    platform: &Platform,
    features: &HashMap<String, bool>,
    os_known: bool,
) -> bool {
    let Some(rules) = rules else { return true };
    let mut allowed = false;
    for rule in rules {
        let mut matched = true;
        if let Some(os) = &rule.os {
            if let Some(name) = &os.name {
                matched &= name == &platform.os;
            }
            if let Some(arch) = &os.arch {
                matched &= normalize_arch(arch) == platform.arch;
            }
            if let Some(pattern) = &os.version {
                // Without a reliable OS version we cannot honestly satisfy the
                // condition, so the rule is treated as not matching.
                matched &= os_known && matches_version_pattern(pattern, &platform.os_version);
            }
            if let Some(range) = &os.version_range {
                let known = os_known && !platform.os_version.is_empty();
                if let Some(min) = &range.min {
                    matched &= known && compare_versions(&platform.os_version, min).is_ge();
                }
                if let Some(max) = &range.max {
                    matched &= known && compare_versions(&platform.os_version, max).is_le();
                }
            }
        }
        if let Some(features_rule) = &rule.features {
            for (key, value) in features_rule {
                matched &= features.get(key).copied().unwrap_or(false) == *value;
            }
        }
        if matched {
            allowed = rule.action == "allow";
        }
    }
    allowed
}

pub fn library_allowed(library: &Library, platform: &Platform, env: &RuleEnv) -> bool {
    rules_allow(
        library.rules.as_deref(),
        platform,
        &env.features,
        env.os_known,
    )
}

#[derive(Debug, Clone)]
pub struct RuleEnv {
    pub features: HashMap<String, bool>,
    pub os_known: bool,
}

impl RuleEnv {
    pub fn new(has_custom_resolution: bool, os_known: bool) -> Self {
        let mut features = HashMap::new();
        features.insert("is_demo_user".to_string(), false);
        features.insert("has_custom_resolution".to_string(), has_custom_resolution);
        features.insert("has_quick_plays_support".to_string(), false);
        features.insert("is_quick_play_singleplayer".to_string(), false);
        features.insert("is_quick_play_multiplayer".to_string(), false);
        features.insert("is_quick_play_realms".to_string(), false);
        Self { features, os_known }
    }
}

/// The classpath entry for a library: only `downloads.artifact` belongs on the
/// classpath. Native classifier jars are extracted or loaded separately.
pub fn classpath_entry(library: &Library, platform: &Platform) -> Option<(String, String)> {
    if let Some(downloads) = &library.downloads {
        if let Some(artifact) = &downloads.artifact {
            let path = artifact
                .path
                .clone()
                .unwrap_or_else(|| maven_path(&library.name));
            return Some((path, artifact.url.clone()));
        }
    }
    if library.downloads.is_none() && library.natives.is_none() {
        let path = maven_path(&library.name);
        let base = library
            .url
            .clone()
            .unwrap_or_else(|| "https://libraries.minecraft.net/".to_string());
        let url = format!("{}/{}", base.trim_end_matches('/'), path);
        return Some((path, url));
    }
    let _ = platform;
    None
}

/// Legacy native selection: `natives` maps an OS to a classifier key.
///
/// The returned download info is the classifier entry itself, so callers verify
/// the native jar against its own hash instead of the main artifact's.
pub fn native_classifier(
    library: &Library,
    platform: &Platform,
) -> Option<(String, crate::types::DownloadInfo)> {
    let natives = library.natives.as_ref()?;
    let key_template = natives.get(&platform.os)?;
    let arch_bits = match platform.arch.as_str() {
        "x86" => "32",
        _ => "64",
    };
    let key = key_template.replace("${arch}", arch_bits);
    let downloads = library.downloads.as_ref()?;
    let info = downloads.classifiers.as_ref()?.get(&key)?.clone();
    let path = info
        .path
        .clone()
        .unwrap_or_else(|| maven_path(&format!("{}:{}", library.name, key)));
    Some((path, info))
}

pub fn require_java_major(game_version: &str, declared: Option<u32>) -> u32 {
    if let Some(major) = declared {
        return major;
    }
    // Only used for old metadata that predates `javaVersion`.
    let base = game_version
        .split(['-', '+'])
        .next()
        .unwrap_or(game_version);
    let mut parts = base.split('.');
    let major = parts.next().unwrap_or("1").parse::<u32>().unwrap_or(1);
    let minor = parts.next().unwrap_or("0").parse::<u32>().unwrap_or(0);
    let patch = parts.next().unwrap_or("0").parse::<u32>().unwrap_or(0);
    if major != 1 {
        // Modern calendar-style versions always declare `javaVersion`; this is a
        // conservative floor rather than a claim about unverified releases.
        return 21;
    }
    match minor {
        0..=16 => 8,
        17 => 16,
        18..=20 => match (minor, patch) {
            (20, 5..) => 21,
            _ => 17,
        },
        21..=22 => 21,
        _ => 21,
    }
}

pub fn ensure_within(base: &std::path::Path, candidate: &std::path::Path) -> Result<()> {
    if !candidate.starts_with(base) {
        return Err(anyhow!(
            "拒绝写入数据目录之外的路径：{}",
            candidate.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{OsRule, VersionRange};

    fn platform() -> Platform {
        Platform {
            os: "linux".into(),
            arch: "x86_64".into(),
            os_version: "6.8.0".into(),
        }
    }

    #[test]
    fn no_rules_allows_library() {
        assert!(rules_allow(None, &platform(), &HashMap::new(), false));
    }

    #[test]
    fn last_matching_rule_wins() {
        let rules = vec![
            Rule {
                action: "allow".into(),
                os: None,
                features: None,
            },
            Rule {
                action: "disallow".into(),
                os: Some(OsRule {
                    name: Some("linux".into()),
                    ..Default::default()
                }),
                features: None,
            },
        ];
        assert!(!rules_allow(
            Some(&rules),
            &platform(),
            &HashMap::new(),
            false
        ));
    }

    #[test]
    fn disallowed_platform_keeps_library_out() {
        let rules = vec![Rule {
            action: "allow".into(),
            os: Some(OsRule {
                name: Some("osx".into()),
                ..Default::default()
            }),
            features: None,
        }];
        assert!(!rules_allow(
            Some(&rules),
            &platform(),
            &HashMap::new(),
            false
        ));
    }

    #[test]
    fn version_range_requires_known_version() {
        let rules = vec![Rule {
            action: "allow".into(),
            os: Some(OsRule {
                name: Some("linux".into()),
                version_range: Some(VersionRange {
                    min: Some("5.0".into()),
                    max: None,
                }),
                ..Default::default()
            }),
            features: None,
        }];
        assert!(rules_allow(
            Some(&rules),
            &platform(),
            &HashMap::new(),
            true
        ));
        assert!(!rules_allow(
            Some(&rules),
            &platform(),
            &HashMap::new(),
            false
        ));
    }

    #[test]
    fn maven_path_includes_classifier() {
        assert_eq!(
            maven_path("org.lwjgl:lwjgl:3.3.1:natives-linux"),
            "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-linux.jar"
        );
    }

    #[test]
    fn legacy_java_major_matches_known_versions() {
        assert_eq!(require_java_major("1.12.2", None), 8);
        assert_eq!(require_java_major("1.16.5", None), 8);
        assert_eq!(require_java_major("1.17.1", None), 16);
        assert_eq!(require_java_major("1.20.1", None), 17);
        assert_eq!(require_java_major("1.20.4", None), 17);
        assert_eq!(require_java_major("1.20.5", None), 21);
        assert_eq!(require_java_major("26.3", None), 21);
        assert_eq!(require_java_major("1.20.1", Some(25)), 25);
    }

    #[test]
    fn path_escape_is_rejected() {
        let base = std::path::Path::new("/tmp/base");
        assert!(ensure_within(base, &base.join("ok/file")).is_ok());
        assert!(ensure_within(base, std::path::Path::new("/tmp/other/file")).is_err());
    }
}
