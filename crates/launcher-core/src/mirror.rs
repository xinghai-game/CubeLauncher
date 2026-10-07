//! BMCLAPI-compatible URL routing using exact hosts and path boundaries.
use anyhow::{bail, Context, Result};
use reqwest::Url;

pub(crate) fn normalize_base(value: Option<&str>) -> Result<Option<String>> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let url = Url::parse(value).context("镜像地址必须是完整的 HTTP 或 HTTPS URL")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("镜像地址必须使用 HTTP 或 HTTPS，且不能包含账号、查询参数或片段");
    }
    Ok(Some(url.as_str().trim_end_matches('/').to_string()))
}

pub(crate) fn map_url(mirror: Option<&str>, raw: &str) -> String {
    let Some(mirror) = mirror.map(str::trim).filter(|base| !base.is_empty()) else {
        return raw.to_string();
    };
    let (Ok(source), Ok(base)) = (Url::parse(raw), Url::parse(mirror)) else {
        return raw.to_string();
    };
    if !matches!(source.scheme(), "http" | "https")
        || source.port().is_some()
        || !source.username().is_empty()
        || source.password().is_some()
        || source.origin() == base.origin()
    {
        return raw.to_string();
    }
    let path = source.path();
    let (prefix, path) = match source.host_str().unwrap_or_default() {
        "piston-meta.mojang.com"
        | "launchermeta.mojang.com"
        | "piston-data.mojang.com"
        | "launcher.mojang.com" => ("", path),
        "libraries.minecraft.net" | "maven.minecraftforge.net" | "maven.fabricmc.net" => {
            ("/maven", path)
        }
        "files.minecraftforge.net" if path.starts_with("/maven/") => ("", path),
        "maven.neoforged.net" if path.starts_with("/releases/") => {
            ("/maven", &path["/releases".len()..])
        }
        "resources.download.minecraft.net" => ("/assets", path),
        "meta.fabricmc.net" => ("/fabric-meta", path),
        _ => return raw.to_string(),
    };
    let mut mapped = format!("{}{prefix}{path}", mirror.trim_end_matches('/'));
    if let Some(query) = source.query() {
        mapped.push('?');
        mapped.push_str(query);
    }
    if let Some(fragment) = source.fragment() {
        mapped.push('#');
        mapped.push_str(fragment);
    }
    mapped
}

#[cfg(test)]
mod tests {
    use super::*;
    const BMCLAPI: &str = "https://bmclapi2.bangbang93.com";

    #[test]
    fn routes_metadata_binaries_assets_and_loaders() {
        for (source, target) in [
            (
                "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json",
                "/mc/game/version_manifest_v2.json",
            ),
            (
                "https://launchermeta.mojang.com/v1/packages/abc/1.12.json",
                "/v1/packages/abc/1.12.json",
            ),
            (
                "https://piston-data.mojang.com/v1/objects/abc/client.jar",
                "/v1/objects/abc/client.jar",
            ),
            (
                "https://launcher.mojang.com/v1/objects/abc/client.jar",
                "/v1/objects/abc/client.jar",
            ),
            ("https://libraries.minecraft.net/a/b.jar", "/maven/a/b.jar"),
            (
                "https://maven.minecraftforge.net/net/a.jar",
                "/maven/net/a.jar",
            ),
            (
                "https://files.minecraftforge.net/maven/net/a.jar",
                "/maven/net/a.jar",
            ),
            (
                "https://maven.neoforged.net/releases/net/a.jar",
                "/maven/net/a.jar",
            ),
            ("https://maven.fabricmc.net/net/a.jar", "/maven/net/a.jar"),
            (
                "https://meta.fabricmc.net/v2/versions/loader",
                "/fabric-meta/v2/versions/loader",
            ),
            (
                "http://resources.download.minecraft.net/ab/abcdef",
                "/assets/ab/abcdef",
            ),
        ] {
            assert_eq!(map_url(Some(BMCLAPI), source), format!("{BMCLAPI}{target}"));
        }
    }

    #[test]
    fn preserves_unknown_origins_and_path_boundaries() {
        for source in [
            "https://piston-data.mojang.com.example.org/v1/objects/abc/client.jar",
            "https://libraries.minecraft.net:8443/a.jar",
            "https://libraries.minecraft.net@evil.example/a.jar",
            "https://maven.neoforged.net/releases-other/a.jar",
            "https://example.org/library.jar",
            "https://bmclapi2.bangbang93.com/maven/a.jar",
            "not a URL",
        ] {
            assert_eq!(map_url(Some(BMCLAPI), source), source);
        }
    }

    #[test]
    fn retains_custom_base_path_and_query() {
        assert_eq!(
            map_url(
                Some("https://mirror.example/minecraft/"),
                "https://libraries.minecraft.net/a.jar?x=1"
            ),
            "https://mirror.example/minecraft/maven/a.jar?x=1"
        );
        assert_eq!(
            map_url(None, "https://libraries.minecraft.net/a.jar"),
            "https://libraries.minecraft.net/a.jar"
        );
        assert_eq!(
            map_url(Some(" "), "https://libraries.minecraft.net/a.jar"),
            "https://libraries.minecraft.net/a.jar"
        );
    }

    #[test]
    fn normalizes_and_validates_mirror_settings() {
        assert_eq!(normalize_base(Some("  ")).unwrap(), None);
        assert_eq!(
            normalize_base(Some(" https://mirror.example/base/// ")).unwrap(),
            Some("https://mirror.example/base".into())
        );
        for value in [
            "file:///tmp/mirror",
            "mirror.example",
            "https://x/?q=1",
            "https://x/#fragment",
            "https://user:pass@x/",
        ] {
            assert!(normalize_base(Some(value)).is_err(), "{value}");
        }
    }
}
