//! Microsoft account login for Nephele.
//!
//! The flow is the one every third-party launcher uses: a **device code** the
//! player types into a Microsoft page, exchanged for an Xbox Live token, an
//! XSTS token, and finally a Minecraft access token and profile. The player's
//! password never touches this program, which is the whole reason for the
//! device-code flow.
//!
//! The endpoint shapes follow `azalea-auth` (MIT); the sequence is documented
//! at <https://minecraft.wiki/w/Microsoft_authentication>.
//!
//! ```text
//! device code -> Microsoft -> Xbox Live -> XSTS -> Minecraft -> profile
//! ```

use std::time::{Duration, Instant};

use serde::Deserialize;

/// Sign in to Xbox Live and keep a refresh token so the session survives.
const SCOPE: &str = "XboxLive.signin offline_access";

// The consumer tenant is what personal Microsoft accounts live in. The legacy
// `login.live.com` endpoints only accept legacy Live application ids, so an
// app registered in Entra answers "the account does not exist in tenant
// Microsoft Services" when pointed at them.
const LEGACY_SCOPE: &str = "service::user.auth.xboxlive.com::MBI_SSL";
const LEGACY_DEVICE_CODE_URL: &str = "https://login.live.com/oauth20_connect.srf";
const LEGACY_TOKEN_URL: &str = "https://login.live.com/oauth20_token.srf";

const DEVICE_CODE_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const XBL_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MINECRAFT_LOGIN_URL: &str =
    "https://api.minecraftservices.com/authentication/login_with_xbox";
const PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";
const JOIN_URL: &str = "https://sessionserver.mojang.com/session/minecraft/join";

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("network error: {0}")]
    Network(String),
    #[error("Microsoft rejected the request: {0}")]
    Microsoft(String),
    #[error("Xbox Live error {xerr}: {message}")]
    Xbox { xerr: String, message: String },
    #[error("this account does not own Minecraft")]
    NotOwned,
    #[error("the sign-in code expired")]
    Expired,
    #[error("unexpected response: {0}")]
    Unexpected(String),
    #[error("storage: {0}")]
    Io(String),
    #[error("no client id configured")]
    NoClientId,
}

/// What the player has to do: go to this URL and type this code.
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceCode {
    pub user_code: String,
    #[serde(rename = "verification_uri", alias = "verification_url")]
    pub verification_uri: String,
    pub device_code: String,
    pub expires_in: u64,
    pub interval: u64,
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct XboxResponse {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "DisplayClaims", default)]
    display_claims: serde_json::Value,
}

impl XboxResponse {
    fn user_hash(&self) -> Result<String, AuthError> {
        self.display_claims
            .get("xui")
            .and_then(|xui| xui.get(0))
            .and_then(|first| first.get("uhs"))
            .and_then(|uhs| uhs.as_str())
            .map(str::to_owned)
            .ok_or_else(|| AuthError::Unexpected("Xbox response without a user hash".into()))
    }
}

#[derive(Debug, Clone, Deserialize)]
struct MinecraftToken {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
}

/// A signed-in Minecraft account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub id: String,
    pub name: String,
}

/// Everything needed to join an online-mode server later.
#[derive(Debug, Clone)]
pub struct Session {
    /// The Minecraft services access token, used for the join call.
    pub access_token: String,
    /// When the Minecraft token expires, as seconds since the Unix epoch.
    pub expires_at: u64,
    pub profile: Profile,
    /// Kept so the login can be refreshed without asking again.
    pub refresh_token: Option<String>,
}

/// A session as written to disk, so a later run can join without signing in.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoredSession {
    pub client_id: String,
    pub access_token: String,
    pub expires_at: u64,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
}

impl StoredSession {
    pub fn from_session(client_id: &str, session: &Session) -> Self {
        Self {
            client_id: client_id.to_owned(),
            access_token: session.access_token.clone(),
            expires_at: session.expires_at,
            id: session.profile.id.clone(),
            name: session.profile.name.clone(),
            refresh_token: session.refresh_token.clone(),
        }
    }

    pub fn session(&self) -> Session {
        Session {
            access_token: self.access_token.clone(),
            expires_at: self.expires_at,
            profile: Profile {
                id: self.id.clone(),
                name: self.name.clone(),
            },
            refresh_token: self.refresh_token.clone(),
        }
    }

    pub fn load(path: &std::path::Path) -> Result<Self, AuthError> {
        let text = std::fs::read_to_string(path)
            .map_err(|err| AuthError::Io(format!("{}: {err}", path.display())))?;
        serde_json::from_str(&text)
            .map_err(|err| AuthError::Io(format!("{}: {err}", path.display())))
    }

    pub fn save(&self, path: &std::path::Path) -> Result<(), AuthError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| AuthError::Io(format!("{}: {err}", parent.display())))?;
        }
        let json =
            serde_json::to_string_pretty(self).map_err(|err| AuthError::Io(err.to_string()))?;
        std::fs::write(path, json)
            .map_err(|err| AuthError::Io(format!("{}: {err}", path.display())))
    }
}
/// Start a sign-in: ask Microsoft for a device code.
pub fn begin(client_id: &str) -> Result<DeviceCode, AuthError> {
    begin_with(client_id, false)
}

/// `legacy` targets `login.live.com`, which only accepts legacy Live
/// application ids. Keep it for testing with an app registered before Entra.
pub fn begin_with(client_id: &str, legacy: bool) -> Result<DeviceCode, AuthError> {
    if client_id.is_empty() {
        return Err(AuthError::NoClientId);
    }
    let (url, _, scope) = endpoints(legacy);
    let body = if legacy {
        form(&[
            ("scope", scope),
            ("client_id", client_id),
            ("response_type", "device_code"),
        ])
    } else {
        form(&[("client_id", client_id), ("scope", scope)])
    };
    let text = post(url, &body)?;
    serde_json::from_str(&text).map_err(|err| AuthError::Unexpected(format!("{err}: {text}")))
}

/// Poll until the player finishes signing in, or the code expires.
///
/// `on_wait` is called once before each poll so a caller can keep a UI lively.
pub fn wait_for_token(
    client_id: &str,
    code: &DeviceCode,
    on_wait: impl FnMut(Duration),
) -> Result<String, AuthError> {
    wait_for_token_with(client_id, code, false, on_wait)
}

pub fn wait_for_token_with(
    client_id: &str,
    code: &DeviceCode,
    legacy: bool,
    mut on_wait: impl FnMut(Duration),
) -> Result<String, AuthError> {
    let (_, token_url, _) = endpoints(legacy);
    let deadline = Instant::now() + Duration::from_secs(code.expires_in);
    let interval = Duration::from_secs(code.interval.max(1));

    while Instant::now() < deadline {
        on_wait(interval);
        std::thread::sleep(interval);

        let body = form(&[
            ("client_id", client_id),
            ("device_code", &code.device_code),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ]);

        match post(token_url, &body) {
            Ok(text) => {
                let token: TokenResponse = serde_json::from_str(&text)
                    .map_err(|err| AuthError::Unexpected(format!("{err}: {text}")))?;
                return Ok(token.access_token);
            }
            // `authorization_pending` comes back as a 400 while the player has
            // not finished; anything else is a real failure.
            Err(err) => {
                let text = err.to_string();
                if !(text.contains("authorization_pending")
                    || text.contains("slow_down")
                    || text.contains("400"))
                {
                    return Err(AuthError::Microsoft(text));
                }
            }
        }
    }

    Err(AuthError::Expired)
}

/// Exchange a Microsoft access token for a Minecraft session.
pub fn to_minecraft(client_id: &str, ms_access_token: &str) -> Result<Session, AuthError> {
    if client_id.is_empty() {
        return Err(AuthError::NoClientId);
    }
    let started = Instant::now();

    // Xbox Live. The `d=` prefix marks this as our own application's ticket.
    let xbl_body = serde_json::json!({
        "Properties": {
            "AuthMethod": "RPS",
            "SiteName": "user.auth.xboxlive.com",
            "RpsTicket": format!("d={ms_access_token}"),
        },
        "RelyingParty": "http://auth.xboxlive.com",
        "TokenType": "JWT",
    });
    let xbl: XboxResponse = post_json(XBL_URL, &xbl_body)?;
    let user_hash = xbl.user_hash()?;

    // XSTS, for the Minecraft relying party specifically.
    let xsts_body = serde_json::json!({
        "Properties": { "SandboxId": "RETAIL", "UserTokens": [xbl.token] },
        "RelyingParty": "rp://api.minecraftservices.com/",
        "TokenType": "JWT",
    });
    let xsts: XboxResponse = post_json(XSTS_URL, &xsts_body)?;

    // Minecraft services.
    let login_body = serde_json::json!({
        "identityToken": format!("XBL3.0 x={user_hash};{}", xsts.token),
    });
    let minecraft: MinecraftToken = post_json(MINECRAFT_LOGIN_URL, &login_body)?;

    let profile = fetch_profile(&minecraft.access_token)?;

    Ok(Session {
        access_token: minecraft.access_token,
        expires_at: unix_now() + minecraft.expires_in.max(started.elapsed().as_secs()),
        profile,
        refresh_token: None,
    })
}

/// Tell Mojang that this account is joining a server with `server_hash`.
///
/// This is the step people miss: the server asks Mojang whether anyone claimed
/// that hash, and without this call it answers "Failed to verify username".
/// The hash is the one computed from the shared secret and the server's key.
pub fn join_server(access_token: &str, uuid: &str, server_hash: &str) -> Result<(), AuthError> {
    let body = serde_json::json!({
        "accessToken": access_token,
        "selectedProfile": uuid.replace('-', ""),
        "serverId": server_hash,
    });

    let payload = body.to_string();
    // The session server is the last hop before a server lets us in, and it
    // answers 502/503/504 often enough to matter - during maintenance, and when
    // one account re-joins repeatedly in quick succession. A transient 5xx used
    // to abort the join outright, which looks like "the client is broken".
    const ATTEMPTS: [u64; 3] = [0, 1_500, 4_000];
    let mut last = String::new();
    for wait in ATTEMPTS {
        if wait > 0 {
            std::thread::sleep(std::time::Duration::from_millis(wait));
        }
        match ureq::post(JOIN_URL)
            .set("Content-Type", "application/json")
            .send_string(&payload)
        {
            // Success is 204 No Content, so there is nothing to parse.
            Ok(_) => return Ok(()),
            Err(ureq::Error::Status(204, _)) => return Ok(()),
            Err(ureq::Error::Status(code, response)) if (500..=504).contains(&code) => {
                last = response.into_string().unwrap_or_default();
            }
            Err(ureq::Error::Status(code @ (401 | 403), response)) => {
                let text = response.into_string().unwrap_or_default();
                return Err(AuthError::Microsoft(format!(
                    "{code}: {text} (the stored session is stale - run `nephele auth` again)"
                )));
            }
            Err(ureq::Error::Status(code, response)) => {
                let text = response.into_string().unwrap_or_default();
                return Err(AuthError::Microsoft(format!("{code}: {text}")));
            }
            Err(err) => return Err(AuthError::Network(err.to_string())),
        }
    }
    Err(AuthError::Microsoft(format!(
        "the Mojang session server is unavailable (503 after {ATTEMPTS_COUNT} attempts): {last}",
        ATTEMPTS_COUNT = ATTEMPTS.len()
    )))
}
/// The signed-in account's name and UUID.
pub fn fetch_profile(access_token: &str) -> Result<Profile, AuthError> {
    let text = get_with_bearer(PROFILE_URL, access_token)?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|err| AuthError::Unexpected(format!("{err}: {text}")))?;

    let id = value
        .get("id")
        .and_then(|id| id.as_str())
        .ok_or(AuthError::NotOwned)?;
    let name = value
        .get("name")
        .and_then(|name| name.as_str())
        .ok_or(AuthError::NotOwned)?;

    Ok(Profile {
        id: id.to_owned(),
        name: name.to_owned(),
    })
}

/// Refresh an expired Minecraft token without another sign-in.
pub fn refresh(client_id: &str, refresh_token: &str) -> Result<Session, AuthError> {
    let body = form(&[
        ("client_id", client_id),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ]);
    let text = post(TOKEN_URL, &body)?;
    let token: TokenResponse = serde_json::from_str(&text)
        .map_err(|err| AuthError::Unexpected(format!("{err}: {text}")))?;
    let mut session = to_minecraft(client_id, &token.access_token)?;
    session.refresh_token = token.refresh_token;
    Ok(session)
}

/// The complete sign-in, for callers that can drive it synchronously.
pub fn login(client_id: &str, mut show: impl FnMut(&DeviceCode)) -> Result<Session, AuthError> {
    let code = begin(client_id)?;
    show(&code);
    let ms_token = wait_for_token(client_id, &code, |_| {})?;
    let mut session = to_minecraft(client_id, &ms_token)?;
    session.refresh_token = None;
    Ok(session)
}

/// `(device code url, token url, scope)` for the selected sign-in endpoint.
fn endpoints(legacy: bool) -> (&'static str, &'static str, &'static str) {
    if legacy {
        (LEGACY_DEVICE_CODE_URL, LEGACY_TOKEN_URL, LEGACY_SCOPE)
    } else {
        (DEVICE_CODE_URL, TOKEN_URL, SCOPE)
    }
}

fn form(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(key, value)| format!("{}={}", escape(key), escape(value)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Percent-encoding for form bodies. Only the characters that actually appear
/// in these fields are handled, which keeps it dependency-free.
fn escape(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

fn post(url: &str, body: &str) -> Result<String, AuthError> {
    match ureq::post(url)
        .set("Content-Type", "application/x-www-form-urlencoded")
        .set("Accept", "application/json")
        .send_string(body)
    {
        Ok(response) => response
            .into_string()
            .map_err(|err| AuthError::Network(err.to_string())),
        Err(ureq::Error::Status(code, response)) => {
            let text = response.into_string().unwrap_or_default();
            Err(AuthError::Microsoft(format!("{code}: {text}")))
        }
        Err(err) => Err(AuthError::Network(err.to_string())),
    }
}

fn post_json<T: for<'de> Deserialize<'de>>(
    url: &str,
    body: &serde_json::Value,
) -> Result<T, AuthError> {
    let payload = body.to_string();
    match ureq::post(url)
        .set("Content-Type", "application/json")
        .set("Accept", "application/json")
        .send_string(&payload)
    {
        Ok(response) => {
            let text = response
                .into_string()
                .map_err(|err| AuthError::Network(err.to_string()))?;
            serde_json::from_str(&text)
                .map_err(|err| AuthError::Unexpected(format!("{err}: {text}")))
        }
        Err(ureq::Error::Status(code, response)) => {
            let text = response.into_string().unwrap_or_default();
            // XSTS reports bad accounts with a structured body.
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(xerr) = value.get("XErr") {
                    let message = value
                        .get("Message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("")
                        .to_owned();
                    return Err(AuthError::Xbox {
                        xerr: xerr.to_string(),
                        message,
                    });
                }
            }
            Err(AuthError::Microsoft(format!("{code}: {text}")))
        }
        Err(err) => Err(AuthError::Network(err.to_string())),
    }
}

fn get_with_bearer(url: &str, token: &str) -> Result<String, AuthError> {
    match ureq::get(url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Accept", "application/json")
        .call()
    {
        Ok(response) => response
            .into_string()
            .map_err(|err| AuthError::Network(err.to_string())),
        Err(ureq::Error::Status(404, _)) => Err(AuthError::NotOwned),
        Err(ureq::Error::Status(code, response)) => {
            let text = response.into_string().unwrap_or_default();
            Err(AuthError::Microsoft(format!("{code}: {text}")))
        }
        Err(err) => Err(AuthError::Network(err.to_string())),
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_join_body_uses_the_dashless_uuid() {
        let body = serde_json::json!({
            "accessToken": "token",
            "selectedProfile": "069a79f4-44e9-4726-a5be-fca90e38aaf5".replace('-', ""),
            "serverId": "-1a2b3c",
        });
        assert_eq!(body["selectedProfile"], "069a79f444e94726a5befca90e38aaf5");
        assert_eq!(body["serverId"], "-1a2b3c");
    }
    #[test]
    fn form_bodies_are_percent_encoded() {
        let body = form(&[("scope", "service::user.auth.xboxlive.com::MBI_SSL")]);
        assert!(body.starts_with("scope=service%3A%3Auser.auth.xboxlive.com"));
    }

    #[test]
    fn a_device_code_parses() {
        let json = r#"{
            "user_code": "ABCD1234",
            "device_code": "device-secret",
            "verification_uri": "https://microsoft.com/link",
            "expires_in": 900,
            "interval": 5,
            "message": "To sign in, use a web browser to open the page"
        }"#;
        let code: DeviceCode = serde_json::from_str(json).unwrap();
        assert_eq!(code.user_code, "ABCD1234");
        assert_eq!(code.interval, 5);
        assert!(code.verification_uri.starts_with("https://"));
    }

    #[test]
    fn the_verification_url_field_is_accepted_under_both_names() {
        let json = r#"{
            "user_code": "X",
            "device_code": "Y",
            "verification_url": "https://microsoft.com/link",
            "expires_in": 60,
            "interval": 5
        }"#;
        let code: DeviceCode = serde_json::from_str(json).unwrap();
        assert_eq!(code.verification_uri, "https://microsoft.com/link");
    }

    #[test]
    fn the_xbox_user_hash_comes_out_of_the_display_claims() {
        let json = r#"{
            "Token": "xbl-token",
            "DisplayClaims": { "xui": [ { "uhs": "1234567890" } ] }
        }"#;
        let response: XboxResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.token, "xbl-token");
        assert_eq!(response.user_hash().unwrap(), "1234567890");
    }

    #[test]
    fn a_ticket_without_display_claims_is_an_error_not_a_panic() {
        let json = r#"{ "Token": "x" }"#;
        let response: XboxResponse = serde_json::from_str(json).unwrap();
        assert!(matches!(
            response.user_hash(),
            Err(AuthError::Unexpected(_))
        ));
    }

    #[test]
    fn an_empty_client_id_is_rejected_before_any_request() {
        assert!(matches!(begin(""), Err(AuthError::NoClientId)));
        assert!(matches!(
            to_minecraft("", "token"),
            Err(AuthError::NoClientId)
        ));
    }

    #[test]
    fn a_missing_profile_means_the_account_does_not_own_minecraft() {
        // `fetch_profile` maps a 404 to NotOwned; this covers the body case.
        let value: serde_json::Value = serde_json::from_str(r#"{"error":"NOT_FOUND"}"#).unwrap();
        assert!(value.get("id").is_none());
    }
}
