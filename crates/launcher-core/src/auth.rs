//! 正版（Microsoft / Xbox Live）登录。
//!
//! The launcher never asks for a password. It uses the OAuth 2.0 device code
//! flow, so the user types a short code on Microsoft's own page while this code
//! polls for the result. From there the chain is the one every Minecraft client
//! follows: Microsoft account token → Xbox Live → XSTS → Minecraft services →
//! profile. Only the last access token is handed to the game.
//!
//! Two properties are worth keeping when editing this file:
//!
//! * Tokens are never logged. Error messages carry Microsoft's own text and
//!   HTTP status only.
//! * The Microsoft refresh token is the single long-lived secret. It is what
//!   `accounts.json` stores (owner-only on Unix), and the Minecraft access token
//!   is derived again whenever it is close to expiring.

use crate::types::*;
use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use chrono::{Duration, Utc};
use serde::Deserialize;
use std::time::Duration as StdDuration;

/// Refresh a little before the token really expires, so a launch that takes a
/// moment to spin up Java cannot start with a token that dies on the way.
const REFRESH_MARGIN_MINUTES: i64 = 5;
/// A skin sheet is 64×64 (or 64×32 for the old format); anything larger than
/// this is not a skin and is refused instead of being embedded in the UI.
const MAX_TEXTURE_BYTES: usize = 512 * 1024;

/// Every URL the sign-in chain talks to. The defaults are the real endpoints;
/// tests point them at a local stub server, which is why they are data rather
/// than constants inlined at the call sites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthEndpoints {
    pub device_code: String,
    pub token: String,
    pub xbox_live: String,
    pub xsts: String,
    pub minecraft_login: String,
    pub minecraft_profile: String,
    pub entitlements: String,
}

impl Default for AuthEndpoints {
    fn default() -> Self {
        Self {
            device_code: "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode"
                .into(),
            token: "https://login.microsoftonline.com/consumers/oauth2/v2.0/token".into(),
            xbox_live: "https://user.auth.xboxlive.com/user/authenticate".into(),
            xsts: "https://xsts.auth.xboxlive.com/xsts/authorize".into(),
            minecraft_login: "https://api.minecraftservices.com/authentication/login_with_xbox"
                .into(),
            minecraft_profile: "https://api.minecraftservices.com/minecraft/profile".into(),
            entitlements: "https://api.minecraftservices.com/entitlements/mcstore".into(),
        }
    }
}

#[derive(Clone)]
pub struct MicrosoftAuth {
    client: reqwest::Client,
    /// Application id compiled into CubeLauncher.
    client_id: String,
    endpoints: AuthEndpoints,
}

impl MicrosoftAuth {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(format!("{LAUNCHER_NAME}/{LAUNCHER_VERSION}"))
            .connect_timeout(StdDuration::from_secs(20))
            .timeout(StdDuration::from_secs(45))
            .build()?;
        Ok(Self {
            client,
            client_id: MICROSOFT_CLIENT_ID.to_string(),
            endpoints: AuthEndpoints::default(),
        })
    }

    #[cfg(test)]
    pub(crate) fn with_endpoints(mut self, endpoints: AuthEndpoints) -> Self {
        self.endpoints = endpoints;
        self
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /* ------------------------------------------------------------ device code */

    /// Ask Microsoft for a code the user types on their own device.
    pub async fn request_device_code(&self) -> Result<DeviceCodePrompt> {
        let response = self
            .client
            .post(&self.endpoints.device_code)
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("scope", MICROSOFT_SCOPE),
            ])
            .send()
            .await
            .context("无法连接 Microsoft 登录服务")?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!(
                "Microsoft 拒绝了登录请求（HTTP {status}）：{}",
                oauth_message(&body)
            );
        }
        let parsed: DeviceCodeResponse =
            serde_json::from_str(&body).context("Microsoft 返回了无法解析的设备代码响应")?;
        let interval = parsed.interval.clamp(1, 60);
        Ok(DeviceCodePrompt {
            login_id: parsed.device_code,
            user_code: parsed.user_code,
            verification_uri: parsed
                .verification_uri
                .unwrap_or_else(|| "https://microsoft.com/link".to_string()),
            message: if parsed.message.is_empty() {
                "请在浏览器中打开链接并输入代码完成登录".to_string()
            } else {
                parsed.message
            },
            expires_in: if parsed.expires_in == 0 {
                900
            } else {
                parsed.expires_in
            },
            interval,
        })
    }

    /// One poll of a pending device code sign-in.
    ///
    /// Transport failures are reported as `Err` so the caller can retry on the
    /// next tick; everything the flow itself answered is a `LoginPoll`.
    pub async fn poll_device_code(&self, device_code: &str) -> Result<LoginPoll> {
        let response = self
            .client
            .post(&self.endpoints.token)
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", device_code),
            ])
            .send()
            .await
            .context("查询 Microsoft 登录状态失败")?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();

        if status.is_success() {
            // From here on the device code has been redeemed, so nothing can be
            // retried: every remaining failure ends the attempt with a message
            // instead of bubbling up as a transport error the caller might poll
            // again.
            let tokens: TokenResponse = match serde_json::from_str(&body) {
                Ok(tokens) => tokens,
                Err(error) => {
                    return Ok(LoginPoll::Failed {
                        message: format!("Microsoft 返回了无法解析的令牌响应：{error}"),
                    })
                }
            };
            if tokens.access_token.is_empty() {
                return Ok(LoginPoll::Failed {
                    message: "Microsoft 令牌响应中没有访问令牌".into(),
                });
            }
            return match self
                .complete_login(&tokens.access_token, tokens.refresh_token)
                .await
            {
                Ok(account) => Ok(LoginPoll::Ready {
                    account: Box::new(account),
                }),
                Err(error) => Ok(LoginPoll::Failed {
                    message: format!("{error:#}"),
                }),
            };
        }

        let error: OAuthError = serde_json::from_str(&body).unwrap_or_default();
        Ok(match error.error.as_str() {
            "authorization_pending" => LoginPoll::Pending,
            "slow_down" => LoginPoll::SlowDown,
            "expired_token" | "bad_verification_code" => LoginPoll::Expired,
            "authorization_declined" | "access_denied" => LoginPoll::Declined,
            _ => LoginPoll::Failed {
                message: error.describe(status.as_u16()),
            },
        })
    }

    /* ---------------------------------------------------------------- refresh */

    /// True when the stored Minecraft token can still be used.
    pub fn token_valid(account: &Account) -> bool {
        let Some(auth) = account.microsoft.as_ref() else {
            return false;
        };
        if auth.access_token.is_empty() {
            return false;
        }
        auth.access_expires_at
            .is_some_and(|expires| expires > Utc::now() + Duration::minutes(REFRESH_MARGIN_MINUTES))
    }

    /// Hand back an account whose Minecraft token is usable, refreshing it when
    /// it is about to expire. `allow_network` is false in strict offline mode,
    /// where a cached token is fine but a refresh is not attempted.
    pub async fn ensure_fresh(&self, account: &Account, allow_network: bool) -> Result<Account> {
        if !account.is_microsoft() || Self::token_valid(account) {
            return Ok(account.clone());
        }
        let auth = account
            .microsoft
            .as_ref()
            .ok_or_else(|| anyhow!("“{}”缺少正版登录信息", account.name))?;
        if auth.refresh_token.is_empty() {
            bail!("“{}”的登录已失效，请在账户管理中重新登录", account.name);
        }
        if !allow_network {
            bail!(
                "“{}”的 Minecraft 令牌已过期，需要联网刷新；当前是严格离线模式",
                account.name
            );
        }
        self.refresh(account).await
    }

    /// Exchange the stored refresh token for a fresh Minecraft session.
    pub async fn refresh(&self, account: &Account) -> Result<Account> {
        let auth = account
            .microsoft
            .as_ref()
            .filter(|_| account.is_microsoft())
            .ok_or_else(|| anyhow!("“{}”不是正版账户", account.name))?;
        if auth.refresh_token.is_empty() {
            bail!("“{}”缺少刷新令牌，请重新登录", account.name);
        }
        let response = self
            .client
            .post(&self.endpoints.token)
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("grant_type", "refresh_token"),
                ("refresh_token", auth.refresh_token.as_str()),
                ("scope", MICROSOFT_SCOPE),
            ])
            .send()
            .await
            .context("刷新 Microsoft 登录失败")?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            let error: OAuthError = serde_json::from_str(&body).unwrap_or_default();
            match error.error.as_str() {
                "invalid_grant" | "interaction_required" | "invalid_request" => {
                    bail!("“{}”的登录已过期，请在账户管理中重新登录", account.name)
                }
                _ => bail!(
                    "刷新登录失败（HTTP {status}）：{}",
                    error.describe(status.as_u16())
                ),
            }
        }
        let tokens: TokenResponse =
            serde_json::from_str(&body).context("Microsoft 返回了无法解析的令牌响应")?;
        // A refresh response may omit the refresh token, which means "keep the
        // one you have": dropping it here would silently log the user out.
        let refresh_token = tokens
            .refresh_token
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| auth.refresh_token.clone());
        let mut updated = self
            .complete_login(&tokens.access_token, Some(refresh_token))
            .await?;
        updated.created_at = account.created_at;
        tracing::info!("已刷新正版账户 {} 的登录凭证", updated.name);
        Ok(updated)
    }

    /* ----------------------------------------------------------------- chain */

    /// Walk the whole chain from a Microsoft account token to a Minecraft
    /// account: Xbox Live → XSTS → Minecraft services → profile.
    async fn complete_login(
        &self,
        microsoft_token: &str,
        refresh_token: Option<String>,
    ) -> Result<Account> {
        let (xbl_token, uhs) = self.xbox_live_authenticate(microsoft_token).await?;
        let (xsts_token, xuid) = self.xsts_authorize(&xbl_token).await?;
        let session = self.minecraft_login(&uhs, &xsts_token).await?;
        let profile = self.minecraft_profile(&session.access_token).await?;
        let owns_java = match self.entitlements(&session.access_token).await {
            Ok(owns) => owns,
            Err(error) => {
                // The profile already proved ownership; the store lookup only
                // labels the account, so a failure here must not block sign-in.
                tracing::warn!("查询 Minecraft 拥有情况失败：{error:#}");
                true
            }
        };
        let uuid = dashed_uuid(&profile.id);
        let now = Utc::now();
        let expires_at = now + Duration::seconds(session.expires_in as i64);
        tracing::info!("正版账户 {} 登录成功（{}）", profile.name, uuid);
        Ok(Account {
            id: profile.id.replace('-', ""),
            name: profile.name,
            uuid,
            kind: "microsoft".to_string(),
            created_at: now,
            microsoft: Some(MicrosoftAccount {
                xuid,
                refresh_token: refresh_token.unwrap_or_default(),
                access_token: session.access_token,
                access_expires_at: Some(expires_at),
                skin_url: profile.skin_url,
                owns_java,
                last_login: Some(now),
            }),
        })
    }

    /// Xbox Live user authentication. The `d=` prefix marks the token as a
    /// delegated Microsoft account token; older accounts expect `t=`, so a
    /// rejection is retried once with that prefix instead of failing the login.
    async fn xbox_live_authenticate(&self, microsoft_token: &str) -> Result<(String, String)> {
        let mut last = String::new();
        for (attempt, prefix) in ["d", "t"].into_iter().enumerate() {
            let body = serde_json::json!({
                "Properties": {
                    "AuthMethod": "RPS",
                    "SiteName": "user.auth.xboxlive.com",
                    "RpsTicket": format!("{prefix}={microsoft_token}"),
                },
                "RelyingParty": "http://auth.xboxlive.com",
                "TokenType": "JWT",
            });
            let response = self
                .client
                .post(&self.endpoints.xbox_live)
                .json(&body)
                .send()
                .await
                .context("无法连接 Xbox Live")?;
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            if status.is_success() {
                let parsed: XboxResponse =
                    serde_json::from_str(&text).context("Xbox Live 返回了无法解析的响应")?;
                let uhs = parsed
                    .display_claims
                    .xui
                    .first()
                    .map(|entry| entry.uhs.clone())
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow!("Xbox Live 响应缺少用户哈希"))?;
                return Ok((parsed.token, uhs));
            }
            last = format!("HTTP {status}：{}", summarize(&text));
            if attempt == 0 {
                tracing::warn!("Xbox Live 拒绝了 d= 形式的令牌，改用 t= 重试：{last}");
            }
        }
        bail!("Xbox Live 认证失败（{last}）")
    }

    /// XSTS authorization: the step that turns an Xbox Live token into one
    /// Minecraft services accepts. Its error codes are the ones users hit most
    /// often (no Xbox profile, child account, unsupported region), so they are
    /// translated here instead of surfacing as a number.
    async fn xsts_authorize(&self, xbl_token: &str) -> Result<(String, String)> {
        let body = serde_json::json!({
            "Properties": { "SandboxId": "RETAIL", "UserTokens": [xbl_token] },
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT",
        });
        let response = self
            .client
            .post(&self.endpoints.xsts)
            .json(&body)
            .send()
            .await
            .context("无法连接 Xbox 安全令牌服务")?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            let error: XstsError = serde_json::from_str(&text).unwrap_or_default();
            bail!("{}", xsts_message(error.xerr, &error.message));
        }
        let parsed: XboxResponse =
            serde_json::from_str(&text).context("XSTS 返回了无法解析的响应")?;
        let xuid = parsed
            .display_claims
            .xui
            .first()
            .map(|entry| entry.uhs.clone())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("XSTS 响应缺少用户哈希"))?;
        Ok((parsed.token, xuid))
    }

    async fn minecraft_login(&self, uhs: &str, xsts_token: &str) -> Result<McSession> {
        let body = serde_json::json!({ "identityToken": format!("XBL3.0 x={uhs};{xsts_token}") });
        let response = self
            .client
            .post(&self.endpoints.minecraft_login)
            .json(&body)
            .send()
            .await
            .context("无法连接 Minecraft 登录服务")?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("Minecraft 登录失败（HTTP {status}）：{}", summarize(&text));
        }
        let parsed: McLoginResponse =
            serde_json::from_str(&text).context("Minecraft 登录返回了无法解析的响应")?;
        if parsed.access_token.is_empty() {
            bail!("Minecraft 登录响应中没有访问令牌");
        }
        Ok(McSession {
            access_token: parsed.access_token,
            expires_in: parsed.expires_in.unwrap_or(86_400).max(60),
        })
    }

    /// Profile lookup, which doubles as the ownership check: an account that
    /// does not own the game has no profile to read.
    async fn minecraft_profile(&self, access_token: &str) -> Result<McProfile> {
        let response = self
            .client
            .get(&self.endpoints.minecraft_profile)
            .bearer_auth(access_token)
            .send()
            .await
            .context("无法读取 Minecraft 档案")?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if status.as_u16() == 404 {
            bail!("这个 Microsoft 账户没有 Minecraft Java 版（没有游戏档案）。请确认购买时使用的正是该账户");
        }
        if !status.is_success() {
            bail!(
                "读取 Minecraft 档案失败（HTTP {status}）：{}",
                summarize(&text)
            );
        }
        let mut profile: McProfile =
            serde_json::from_str(&text).context("Minecraft 档案无法解析")?;
        if profile.id.is_empty() || profile.name.is_empty() {
            bail!("Minecraft 档案缺少角色信息，请稍后重试");
        }
        let skin_url = profile
            .skins
            .iter()
            .find(|skin| skin.state.eq_ignore_ascii_case("ACTIVE") && !skin.url.is_empty())
            .or_else(|| profile.skins.iter().find(|skin| !skin.url.is_empty()))
            .map(|skin| skin.url.clone());
        profile.skin_url = skin_url;
        Ok(profile)
    }

    /// Game ownership list. `product_minecraft` is Java Edition; the
    /// `game_minecraft*` entries are what newer store responses return.
    async fn entitlements(&self, access_token: &str) -> Result<bool> {
        let response = self
            .client
            .get(&self.endpoints.entitlements)
            .bearer_auth(access_token)
            .send()
            .await
            .context("无法查询 Minecraft 拥有情况")?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("查询拥有情况失败（HTTP {status}）");
        }
        let parsed: Entitlements = serde_json::from_str(&text).context("拥有情况无法解析")?;
        Ok(parsed.items.iter().any(|item| {
            item.name == "product_minecraft" || item.name.starts_with("game_minecraft")
        }))
    }

    /* ------------------------------------------------------------------ skin */

    /// Download an official skin texture and hand it back as a data URL, so the
    /// account list can show the real head without asking a third-party service
    /// (and without a network round trip every time the modal opens).
    pub async fn skin_texture(&self, url: &str) -> Result<String> {
        if !url.starts_with("https://") && !url.starts_with("http://") {
            bail!("皮肤地址不是 HTTP(S)：{url}");
        }
        let response = self.client.get(url).send().await.context("下载皮肤失败")?;
        if !response.status().is_success() {
            bail!("下载皮肤失败（HTTP {}）", response.status());
        }
        let bytes = response.bytes().await.context("读取皮肤数据失败")?;
        if bytes.len() > MAX_TEXTURE_BYTES {
            bail!("皮肤文件过大（{} 字节）", bytes.len());
        }
        if bytes.len() < 8 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
            bail!("皮肤不是 PNG 图片");
        }
        Ok(format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&bytes)
        ))
    }
}

/// Everything the launch command line needs, derived from an account whose
/// tokens are already fresh.
pub fn session_for(account: &Account) -> AuthSession {
    let uuid = account.undashed_uuid();
    match account
        .microsoft
        .as_ref()
        .filter(|_| account.is_microsoft())
    {
        Some(auth) => AuthSession {
            name: account.name.clone(),
            uuid,
            session: format!("token:{}:{}", auth.access_token, account.uuid),
            access_token: auth.access_token.clone(),
            user_type: "msa".to_string(),
            xuid: auth.xuid.clone(),
            client_id: launcher_client_id(),
        },
        None => AuthSession {
            name: account.name.clone(),
            uuid,
            session: format!("token:0:{}", account.uuid),
            access_token: "0".to_string(),
            user_type: "legacy".to_string(),
            xuid: String::new(),
            client_id: String::new(),
        },
    }
}

/// Value for the game's `--clientId`. The official launcher sends a UUID
/// identifying the launcher installation; a constant derived from the launcher
/// name is enough here and stays stable across releases.
pub fn launcher_client_id() -> String {
    crate::instance::offline_uuid(&format!("{LAUNCHER_NAME}:client"))
}

/// Insert the dashes of the canonical UUID form. Mojang reports profile ids
/// undashed, while instances, skins and servers all use the dashed one.
pub fn dashed_uuid(value: &str) -> String {
    let compact: String = value.chars().filter(|c| *c != '-').collect();
    if compact.len() != 32 {
        return value.to_string();
    }
    format!(
        "{}-{}-{}-{}-{}",
        &compact[0..8],
        &compact[8..12],
        &compact[12..16],
        &compact[16..20],
        &compact[20..32]
    )
}

fn xsts_message(code: i64, message: &str) -> String {
    match code {
        2148916233 => {
            "这个 Microsoft 账户还没有 Xbox 档案，请先在 xbox.com 登录一次以创建档案".into()
        }
        2148916235 => "该账户所在地区不支持 Xbox Live，无法登录 Minecraft".into(),
        2148916236 | 2148916237 => "该账户需要完成成人验证（韩国地区要求）".into(),
        2148916238 => "该账户是未成年账户，需要先在 Xbox 家庭组中由成年人添加".into(),
        2148916227 | 2148916228 => "该 Xbox 账户已被封禁，无法登录".into(),
        _ if !message.is_empty() => format!("Xbox 授权失败（{code}）：{message}"),
        _ => format!("Xbox 授权失败（错误码 {code}）"),
    }
}

/// Microsoft's own explanation when it has one, otherwise whatever the body
/// carried. Kept short: it is shown in a toast.
fn oauth_message(body: &str) -> String {
    let error: OAuthError = serde_json::from_str(body).unwrap_or_default();
    if error.error.is_empty() {
        return summarize(body);
    }
    error.describe(0)
}

fn summarize(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "响应为空".to_string();
    }
    let clipped: String = trimmed.chars().take(240).collect();
    if trimmed.chars().count() > 240 {
        format!("{clipped}…")
    } else {
        clipped
    }
}

/* ------------------------------------------------------------- wire formats */

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    #[serde(default)]
    device_code: String,
    #[serde(default)]
    user_code: String,
    #[serde(default)]
    verification_uri: Option<String>,
    #[serde(default)]
    message: String,
    #[serde(default)]
    expires_in: u64,
    #[serde(default)]
    interval: u64,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct OAuthError {
    #[serde(default)]
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

impl OAuthError {
    fn describe(&self, status: u16) -> String {
        let description = self
            .error_description
            .as_deref()
            .map(|value| value.split('\n').next().unwrap_or(value).trim().to_string())
            .filter(|value| !value.is_empty());
        match (self.error.is_empty(), description) {
            (false, Some(description)) => format!("{}：{description}", self.error),
            (false, None) => self.error.clone(),
            (true, Some(description)) => description,
            (true, None) => format!("HTTP {status}"),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct XboxResponse {
    /// Xbox Live and XSTS both capitalise this field; serde keys are case
    /// sensitive, so without the rename the token silently parses as empty.
    #[serde(default, rename = "Token")]
    token: String,
    #[serde(default, rename = "DisplayClaims")]
    display_claims: DisplayClaims,
}

#[derive(Debug, Default, Deserialize)]
struct DisplayClaims {
    #[serde(default)]
    xui: Vec<Xui>,
}

#[derive(Debug, Default, Deserialize)]
struct Xui {
    #[serde(default)]
    uhs: String,
}

#[derive(Debug, Default, Deserialize)]
struct XstsError {
    #[serde(default, rename = "XErr")]
    xerr: i64,
    #[serde(default, rename = "Message")]
    message: String,
}

#[derive(Debug, Deserialize)]
struct McLoginResponse {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    expires_in: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct McProfile {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    skins: Vec<McSkin>,
    #[serde(skip)]
    skin_url: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct McSkin {
    #[serde(default)]
    state: String,
    #[serde(default)]
    url: String,
}

#[derive(Debug, Default, Deserialize)]
struct Entitlements {
    #[serde(default)]
    items: Vec<Entitlement>,
}

#[derive(Debug, Default, Deserialize)]
struct Entitlement {
    #[serde(default)]
    name: String,
}

struct McSession {
    access_token: String,
    expires_in: u64,
}

#[cfg(test)]
mod tests;
