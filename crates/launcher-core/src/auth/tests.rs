//! Sign-in tests. Every test runs the real HTTP code against a local stub
//! server, so the chain (device code → Xbox Live → XSTS → Minecraft services →
//! profile) is exercised end to end without touching Microsoft's servers.

use super::*;
use chrono::DateTime;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/* --------------------------------------------------------------- stub server */

#[derive(Debug, Clone, Default)]
struct Request {
    method: String,
    path: String,
    body: String,
    headers: Vec<(String, String)>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Debug, Clone)]
struct Response {
    status: u16,
    body: Vec<u8>,
    content_type: &'static str,
}

impl Response {
    fn json(status: u16, body: serde_json::Value) -> Self {
        Self {
            status,
            body: body.to_string().into_bytes(),
            content_type: "application/json",
        }
    }
}

type Hook = Box<dyn Fn(&Request) -> Option<Response> + Send + Sync>;

#[derive(Default)]
struct State {
    requests: Mutex<Vec<Request>>,
    /// Scripted answers per path. The last entry repeats, which is how a test
    /// says "answer `authorization_pending` twice and then succeed".
    routes: Mutex<HashMap<String, Vec<Response>>>,
    hook: Mutex<Option<Hook>>,
}

struct Stub {
    base: String,
    state: Arc<State>,
    task: tokio::task::JoinHandle<()>,
}

impl Stub {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(State::default());
        let shared = state.clone();
        let task = tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let state = shared.clone();
                tokio::spawn(async move {
                    let _ = serve(socket, state).await;
                });
            }
        });
        Self { base, state, task }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn endpoints(&self) -> AuthEndpoints {
        AuthEndpoints {
            device_code: self.url("/devicecode"),
            token: self.url("/token"),
            xbox_live: self.url("/user/authenticate"),
            xsts: self.url("/xsts/authorize"),
            minecraft_login: self.url("/authentication/login_with_xbox"),
            minecraft_profile: self.url("/minecraft/profile"),
            entitlements: self.url("/entitlements/mcstore"),
        }
    }

    fn auth(&self) -> MicrosoftAuth {
        MicrosoftAuth::new(None)
            .unwrap()
            .with_endpoints(self.endpoints())
    }

    /// The single answer this path gives, replacing anything registered before.
    fn route(&self, path: &str, response: Response) {
        self.script(path, vec![response]);
    }

    /// A sequence of answers consumed in order; the last one repeats, so a test
    /// can say "answer `authorization_pending` twice and then succeed".
    fn script(&self, path: &str, responses: Vec<Response>) {
        self.state
            .routes
            .lock()
            .unwrap()
            .insert(path.to_string(), responses);
    }

    fn hook(&self, hook: impl Fn(&Request) -> Option<Response> + Send + Sync + 'static) {
        *self.state.hook.lock().unwrap() = Some(Box::new(hook));
    }

    fn requests(&self) -> Vec<Request> {
        self.state.requests.lock().unwrap().clone()
    }

    fn request(&self, path: &str) -> Request {
        self.requests()
            .into_iter()
            .find(|request| request.path == path)
            .unwrap_or_else(|| panic!("no request was sent to {path}"))
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve(mut socket: TcpStream, state: Arc<State>) -> std::io::Result<()> {
    let mut raw = Vec::new();
    let mut byte = [0u8; 1];
    while raw.len() < 32 * 1024 && !raw.ends_with(b"\r\n\r\n") {
        if socket.read(&mut byte).await? == 0 {
            return Ok(());
        }
        raw.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&raw).to_string();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default().to_string();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_string(), value.trim().to_string()))
        })
        .collect();
    let length: usize = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    if length > 0 {
        socket.read_exact(&mut body).await?;
    }
    let request = Request {
        method,
        path: path.clone(),
        body: String::from_utf8_lossy(&body).to_string(),
        headers,
    };
    state.requests.lock().unwrap().push(request.clone());

    let hook = state
        .hook
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|hook| hook(&request));
    let scripted = hook.or_else(|| {
        let mut routes = state.routes.lock().unwrap();
        let queue = routes.get_mut(&path)?;
        if queue.is_empty() {
            return None;
        }
        Some(if queue.len() > 1 {
            queue.remove(0)
        } else {
            queue[0].clone()
        })
    });
    let response = scripted.unwrap_or(Response {
        status: 404,
        body: b"{\"error\":\"not_found\"}".to_vec(),
        content_type: "application/json",
    });
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        if response.status == 200 {
            "OK"
        } else {
            "Error"
        },
        response.content_type,
        response.body.len()
    );
    socket.write_all(head.as_bytes()).await?;
    socket.write_all(&response.body).await?;
    socket.flush().await?;
    Ok(())
}

/* ----------------------------------------------------------------- fixtures */

const UUIDS: &str = "069a79f444e94726a5befca90e38aaf5";

/// The answers a successful sign-in gets from every hop.
fn happy_routes(stub: &Stub) {
    stub.route(
        "/devicecode",
        Response::json(
            200,
            json!({
                "device_code": "device-code-1",
                "user_code": "ABCD-EFGH",
                "verification_uri": "https://microsoft.com/link",
                "message": "To sign in, use a web browser…",
                "expires_in": 900,
                "interval": 5,
            }),
        ),
    );
    stub.route(
        "/token",
        Response::json(
            200,
            json!({
                "access_token": "msa-access",
                "refresh_token": "msa-refresh",
                "expires_in": 3600,
                "token_type": "Bearer",
            }),
        ),
    );
    stub.route(
        "/user/authenticate",
        Response::json(
            200,
            json!({
                "IssueInstant": "2026-01-01T00:00:00Z",
                "NotAfter": "2026-01-02T00:00:00Z",
                "Token": "xbl-token",
                "DisplayClaims": { "xui": [{ "uhs": "uhs-value" }] },
            }),
        ),
    );
    stub.route(
        "/xsts/authorize",
        Response::json(
            200,
            json!({
                "Token": "xsts-token",
                "DisplayClaims": { "xui": [{ "uhs": "1234567890123456" }] },
            }),
        ),
    );
    stub.route(
        "/authentication/login_with_xbox",
        Response::json(
            200,
            json!({ "username": UUIDS, "access_token": "mc-access", "expires_in": 86400 }),
        ),
    );
    stub.route(
        "/minecraft/profile",
        Response::json(
            200,
            json!({
                "id": UUIDS,
                "name": "Steve",
                "skins": [
                    { "id": "s1", "state": "INACTIVE", "url": "http://textures.example/old.png" },
                    { "id": "s2", "state": "ACTIVE", "url": "http://textures.example/current.png" },
                ],
            }),
        ),
    );
    stub.route(
        "/entitlements/mcstore",
        Response::json(200, json!({ "items": [{ "name": "product_minecraft" }] })),
    );
}

fn oauth_error(error: &str) -> Response {
    Response::json(400, json!({ "error": error }))
}

/* -------------------------------------------------------------------- tests */

#[tokio::test]
async fn device_code_prompt_is_parsed_and_clamped() {
    let stub = Stub::start().await;
    stub.route(
        "/devicecode",
        Response::json(
            200,
            json!({
                "device_code": "dc",
                "user_code": "WXYZ-1234",
                "verification_uri": "https://microsoft.com/link",
                "expires_in": 900,
                "interval": 0,
            }),
        ),
    );
    let prompt = stub.auth().request_device_code().await.unwrap();
    assert_eq!(prompt.login_id, "dc");
    assert_eq!(prompt.user_code, "WXYZ-1234");
    assert_eq!(prompt.verification_uri, "https://microsoft.com/link");
    // A zero interval would busy-poll Microsoft, so it is clamped.
    assert_eq!(prompt.interval, 1);
    assert_eq!(prompt.expires_in, 900);
    assert!(!prompt.message.is_empty());
    let sent = stub.request("/devicecode");
    assert_eq!(sent.method, "POST");
    let sent = sent.body;
    assert!(sent.contains("scope=XboxLive.signin"), "{sent}");
    assert!(
        sent.contains(&format!("client_id={MICROSOFT_CLIENT_ID}")),
        "{sent}"
    );
}

#[tokio::test]
async fn full_chain_produces_a_microsoft_account() {
    let stub = Stub::start().await;
    happy_routes(&stub);
    // Microsoft answers "not yet" twice before the user finishes.
    stub.script(
        "/token",
        vec![
            oauth_error("authorization_pending"),
            oauth_error("authorization_pending"),
            Response::json(
                200,
                json!({ "access_token": "msa-access", "refresh_token": "msa-refresh" }),
            ),
        ],
    );

    let auth = stub.auth();
    assert!(matches!(
        auth.poll_device_code("device-code-1").await.unwrap(),
        LoginPoll::Pending
    ));
    assert!(matches!(
        auth.poll_device_code("device-code-1").await.unwrap(),
        LoginPoll::Pending
    ));
    let LoginPoll::Ready { account } = auth.poll_device_code("device-code-1").await.unwrap() else {
        panic!("third poll should have produced the account");
    };

    assert_eq!(account.id, UUIDS);
    assert_eq!(account.uuid, "069a79f4-44e9-4726-a5be-fca90e38aaf5");
    assert_eq!(account.name, "Steve");
    assert!(account.is_microsoft());
    let microsoft = account.microsoft.as_ref().expect("microsoft block");
    assert_eq!(microsoft.xuid, "1234567890123456");
    assert_eq!(microsoft.refresh_token, "msa-refresh");
    assert_eq!(microsoft.access_token, "mc-access");
    assert!(microsoft.owns_java);
    assert_eq!(
        microsoft.skin_url.as_deref(),
        Some("http://textures.example/current.png")
    );
    let expires = microsoft.access_expires_at.expect("expiry");
    assert!(expires > Utc::now() + Duration::hours(23), "{expires}");

    // Every hop saw the shape it expects.
    let xbl = stub.request("/user/authenticate").body;
    assert!(xbl.contains("\"RpsTicket\":\"d=msa-access\""), "{xbl}");
    let xsts = stub.request("/xsts/authorize").body;
    assert!(xsts.contains("\"UserTokens\":[\"xbl-token\"]"), "{xsts}");
    assert!(xsts.contains("rp://api.minecraftservices.com/"), "{xsts}");
    let login = stub.request("/authentication/login_with_xbox").body;
    assert!(
        login.contains("\"identityToken\":\"XBL3.0 x=uhs-value;xsts-token\""),
        "{login}"
    );
    // The two Minecraft calls are authenticated with the Minecraft token, not
    // with a Microsoft one.
    for path in ["/minecraft/profile", "/entitlements/mcstore"] {
        assert_eq!(
            stub.request(path).header("authorization"),
            Some("Bearer mc-access"),
            "{path}"
        );
    }
    assert_eq!(stub.requests().len(), 8, "three polls and five chain hops");
}

#[tokio::test]
async fn slow_down_and_terminal_states_are_reported() {
    let stub = Stub::start().await;
    stub.route("/token", oauth_error("slow_down"));
    let auth = stub.auth();
    assert!(matches!(
        auth.poll_device_code("dc").await.unwrap(),
        LoginPoll::SlowDown
    ));

    for (error, expected) in [
        ("expired_token", "expired"),
        ("bad_verification_code", "expired"),
        ("authorization_declined", "declined"),
    ] {
        let stub = Stub::start().await;
        stub.route("/token", oauth_error(error));
        let poll = stub.auth().poll_device_code("dc").await.unwrap();
        let label = match poll {
            LoginPoll::Expired => "expired",
            LoginPoll::Declined => "declined",
            other => panic!("unexpected {other:?} for {error}"),
        };
        assert_eq!(label, expected);
    }
}

#[tokio::test]
async fn unknown_oauth_errors_carry_microsofts_message() {
    let stub = Stub::start().await;
    stub.route(
        "/token",
        Response::json(
            400,
            json!({
                "error": "unauthorized_client",
                "error_description": "AADSTS700016: Application not found.\nTrace ID: abc",
            }),
        ),
    );
    let LoginPoll::Failed { message } = stub.auth().poll_device_code("dc").await.unwrap() else {
        panic!("expected a failure");
    };
    assert!(message.contains("unauthorized_client"), "{message}");
    assert!(message.contains("AADSTS700016"), "{message}");
    // Only the first line of Microsoft's description is kept: it is a toast.
    assert!(!message.contains("Trace ID"), "{message}");
}

#[tokio::test]
async fn xsts_error_codes_are_translated() {
    let cases: [(i64, &str); 4] = [
        (2_148_916_233, "Xbox 档案"),
        (2_148_916_235, "地区"),
        (2_148_916_238, "未成年账户"),
        (2_148_916_227, "封禁"),
    ];
    for (code, needle) in cases {
        let stub = Stub::start().await;
        stub.route(
            "/token",
            Response::json(200, json!({ "access_token": "msa" })),
        );
        stub.route(
            "/user/authenticate",
            Response::json(
                200,
                json!({ "Token": "xbl", "DisplayClaims": { "xui": [{ "uhs": "uhs" }] } }),
            ),
        );
        stub.route(
            "/xsts/authorize",
            Response::json(
                401,
                json!({ "Identity": "0", "XErr": code, "Message": "nope" }),
            ),
        );
        // The device code is already redeemed, so a refusal ends the attempt
        // with a message: there is nothing left to poll for.
        let LoginPoll::Failed { message } = stub.auth().poll_device_code("dc").await.unwrap()
        else {
            panic!("XSTS refusal must fail the login");
        };
        assert!(message.contains(needle), "{code} → {message}");
    }
}

#[test]
fn unknown_xsts_codes_keep_microsofts_wording() {
    assert_eq!(xsts_message(999, ""), "Xbox 授权失败（错误码 999）");
    assert!(xsts_message(999, "boom").contains("boom"));
    // A known code wins over whatever text came along with it.
    assert!(xsts_message(2_148_916_238, "boom").contains("未成年账户"));
}

#[tokio::test]
async fn an_account_without_the_game_is_reported_clearly() {
    let stub = Stub::start().await;
    happy_routes(&stub);
    stub.route(
        "/minecraft/profile",
        Response::json(
            404,
            json!({ "path": "/minecraft/profile", "error": "NOT_FOUND" }),
        ),
    );
    let LoginPoll::Failed { message } = stub.auth().poll_device_code("dc").await.unwrap() else {
        panic!("no profile means no Java Edition");
    };
    assert!(message.contains("没有 Minecraft Java 版"), "{message}");
}

#[tokio::test]
async fn xbox_live_retries_with_the_legacy_ticket_prefix() {
    let stub = Stub::start().await;
    happy_routes(&stub);
    stub.route(
        "/token",
        Response::json(200, json!({ "access_token": "msa-access" })),
    );
    // Old accounts reject the modern `d=` ticket and accept `t=`.
    stub.hook(|request| {
        (request.path == "/user/authenticate" && request.body.contains("\"RpsTicket\":\"d="))
            .then(|| Response::json(400, json!({ "Message": "invalid ticket" })))
    });
    let LoginPoll::Ready { account } = stub.auth().poll_device_code("dc").await.unwrap() else {
        panic!("the t= retry should have succeeded");
    };
    assert_eq!(account.name, "Steve");
    let tickets: Vec<String> = stub
        .requests()
        .into_iter()
        .filter(|request| request.path == "/user/authenticate")
        .map(|request| request.body)
        .collect();
    assert_eq!(tickets.len(), 2, "one rejected attempt and one retry");
    assert!(
        tickets[1].contains("\"RpsTicket\":\"t=msa-access\""),
        "{}",
        tickets[1]
    );
}

/// A failure after the device code was redeemed must come back as a result the
/// dialog can show, not as an error that looks like a dropped connection.
#[tokio::test]
async fn a_broken_chain_ends_the_attempt_with_a_message() {
    let stub = Stub::start().await;
    happy_routes(&stub);
    stub.route(
        "/token",
        Response::json(200, json!({ "access_token": "msa-access" })),
    );
    stub.route(
        "/xsts/authorize",
        Response::json(401, json!({ "XErr": 2_148_916_233_i64, "Message": "" })),
    );
    let poll = stub.auth().poll_device_code("dc").await.unwrap();
    let LoginPoll::Failed { message } = poll else {
        panic!("expected a terminal failure, got {poll:?}");
    };
    assert!(message.contains("Xbox 档案"), "{message}");
}

#[tokio::test]
async fn a_refresh_keeps_the_old_refresh_token_when_none_is_returned() {
    let stub = Stub::start().await;
    happy_routes(&stub);
    // Microsoft is allowed to answer without a new refresh token.
    stub.route(
        "/token",
        Response::json(200, json!({ "access_token": "msa-second" })),
    );
    let account = microsoft_account(
        "msa-first",
        "msa-refresh",
        Some(Utc::now() - Duration::hours(1)),
    );

    let refreshed = stub.auth().refresh(&account).await.unwrap();
    assert_eq!(refreshed.created_at, account.created_at);
    assert_eq!(
        refreshed.microsoft.as_ref().unwrap().refresh_token,
        "msa-refresh"
    );
    let sent = stub.request("/token").body;
    assert!(sent.contains("grant_type=refresh_token"), "{sent}");
    assert!(sent.contains("refresh_token=msa-refresh"), "{sent}");
}

#[tokio::test]
async fn an_expired_grant_asks_for_a_new_sign_in() {
    let stub = Stub::start().await;
    stub.route(
        "/token",
        Response::json(
            400,
            json!({ "error": "invalid_grant", "error_description": "expired" }),
        ),
    );
    let account = microsoft_account(
        "stale",
        "stale-refresh",
        Some(Utc::now() - Duration::days(30)),
    );
    let error = stub.auth().refresh(&account).await.expect_err("must fail");
    assert!(format!("{error:#}").contains("重新登录"), "{error:#}");
}

#[tokio::test]
async fn a_fresh_token_is_reused_and_offline_mode_refuses_to_refresh() {
    let stub = Stub::start().await;
    happy_routes(&stub);
    let auth = stub.auth();

    // Still valid: nothing is refreshed and no request is made.
    let valid = microsoft_account("live", "refresh", Some(Utc::now() + Duration::hours(20)));
    let unchanged = auth.ensure_fresh(&valid, true).await.unwrap();
    assert_eq!(unchanged.microsoft.unwrap().access_token, "live");
    assert!(stub.requests().is_empty());

    // Expired, but strict offline mode may not reach out to Microsoft.
    let expired = microsoft_account("dead", "refresh", Some(Utc::now() - Duration::minutes(1)));
    let error = auth
        .ensure_fresh(&expired, false)
        .await
        .expect_err("offline mode must not refresh");
    assert!(format!("{error:#}").contains("严格离线模式"), "{error:#}");
    assert!(stub.requests().is_empty());

    // Expired and allowed online: refreshed in place.
    let refreshed = auth.ensure_fresh(&expired, true).await.unwrap();
    assert_eq!(refreshed.microsoft.unwrap().access_token, "mc-access");
}

#[tokio::test]
async fn offline_roles_keep_the_legacy_identity() {
    let account = Account {
        id: crate::instance::offline_uuid("Alice"),
        name: "Alice".into(),
        uuid: crate::instance::offline_uuid("Alice"),
        kind: "offline".into(),
        created_at: Utc::now(),
        microsoft: None,
    };
    let session = session_for(&account);
    assert_eq!(session.user_type, "legacy");
    assert_eq!(session.access_token, "0");
    assert!(session.xuid.is_empty());
    assert!(session.client_id.is_empty());
    assert_eq!(session.uuid, account.uuid.replace('-', ""));
    assert_eq!(session.session, format!("token:0:{}", account.uuid));

    // A stored Microsoft token never leaks into an offline session, and the
    // other way round: only `kind` decides.
    let mut impostor = account.clone();
    impostor.microsoft = Some(MicrosoftAccount {
        access_token: "mc-access".into(),
        xuid: "1234".into(),
        ..Default::default()
    });
    assert_eq!(session_for(&impostor).access_token, "0");
}

#[tokio::test]
async fn microsoft_accounts_hand_the_game_the_real_token() {
    let account = microsoft_account(
        "mc-access",
        "refresh",
        Some(Utc::now() + Duration::hours(1)),
    );
    let session = session_for(&account);
    assert_eq!(session.user_type, "msa");
    assert_eq!(session.access_token, "mc-access");
    assert_eq!(session.xuid, "1234567890123456");
    assert_eq!(session.uuid, "069a79f444e94726a5befca90e38aaf5");
    assert_eq!(session.session, format!("token:mc-access:{}", account.uuid));
    assert_eq!(session.client_id, launcher_client_id());
}

#[tokio::test]
async fn skin_textures_come_back_as_png_data_urls() {
    let stub = Stub::start().await;
    stub.route(
        "/skin.png",
        Response {
            status: 200,
            body: [b"\x89PNG\r\n\x1a\n".as_slice(), b"payload"].concat(),
            content_type: "image/png",
        },
    );
    stub.route(
        "/not-a-skin.png",
        Response {
            status: 200,
            body: b"<html>nope".to_vec(),
            content_type: "text/html",
        },
    );
    let data = stub
        .auth()
        .skin_texture(&stub.url("/skin.png"))
        .await
        .unwrap();
    assert!(data.starts_with("data:image/png;base64,"), "{data}");
    assert!(stub
        .auth()
        .skin_texture(&stub.url("/not-a-skin.png"))
        .await
        .is_err());
    assert!(stub
        .auth()
        .skin_texture("file:///etc/passwd")
        .await
        .is_err());
    assert!(stub
        .auth()
        .skin_texture(&stub.url("/missing.png"))
        .await
        .is_err());
}

#[test]
fn client_id_defaults_and_overrides_behave() {
    assert_eq!(normalize_client_id(None), MICROSOFT_CLIENT_ID);
    assert_eq!(normalize_client_id(Some("   ".into())), MICROSOFT_CLIENT_ID);
    assert_eq!(
        normalize_client_id(Some(" 11111111-2222-3333-4444-555555555555 ".into())),
        "11111111-2222-3333-4444-555555555555"
    );
}

#[test]
fn profile_ids_are_dashed() {
    assert_eq!(
        dashed_uuid("069a79f444e94726a5befca90e38aaf5"),
        "069a79f4-44e9-4726-a5be-fca90e38aaf5"
    );
    // Already dashed, or not a UUID at all: handed back untouched.
    assert_eq!(
        dashed_uuid("069a79f4-44e9-4726-a5be-fca90e38aaf5"),
        "069a79f4-44e9-4726-a5be-fca90e38aaf5"
    );
    assert_eq!(dashed_uuid("offline"), "offline");
    assert_eq!(dashed_uuid(""), "");
}

/* -------------------------------------------------------------------- helpers */

fn microsoft_account(access: &str, refresh: &str, expires: Option<DateTime<Utc>>) -> Account {
    Account {
        id: UUIDS.to_string(),
        name: "Steve".into(),
        uuid: dashed_uuid(UUIDS),
        kind: "microsoft".into(),
        created_at: Utc::now() - Duration::days(3),
        microsoft: Some(MicrosoftAccount {
            xuid: "1234567890123456".into(),
            refresh_token: refresh.into(),
            access_token: access.into(),
            access_expires_at: expires,
            skin_url: None,
            owns_java: true,
            last_login: None,
        }),
    }
}
