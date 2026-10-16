//! `mochify auth login`.
//!
//! The flow this replaces opened `/auth/cli?state=…` and polled for whatever key
//! arrived at that state. Anyone could choose a state, so a link with an
//! attacker's state signed the attacker in to the victim's account.
//!
//! Both flows here hold a PKCE verifier that never leaves this process; the
//! worker only hands over a key to whoever proves it.
//!
//! - loopback (default): listen on 127.0.0.1, open the browser with our port
//!   and challenge, and receive a one-time code back on that listener.
//! - device (`--no-browser`, or when no browser opens): print a code for the
//!   user to type on mochify.app, and poll until they approve it.

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use reqwest::Url;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn new() -> Self {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let verifier = URL_SAFE_NO_PAD.encode(bytes);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Pkce { verifier, challenge }
    }
}

fn random_hex() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(serde::Deserialize)]
struct TokenResponse {
    #[serde(rename = "apiKey")]
    api_key: Option<String>,
    error: Option<String>,
}

async fn redeem(client: &reqwest::Client, worker: &str, body: serde_json::Value) -> Result<TokenResponse> {
    let res = client
        .post(format!("{worker}/v1/cli/token"))
        .json(&body)
        .send()
        .await
        .context("could not reach the Mochify login service")?;
    res.json::<TokenResponse>()
        .await
        .context("the login service sent a response that could not be parsed")
}

/// Signs in and returns the new API key.
pub async fn login(worker: &str, auth_url: &str, no_browser: bool) -> Result<String> {
    if !no_browser {
        match loopback(worker, auth_url, |url| open::that(url.as_str()).is_ok()).await? {
            Some(key) => return Ok(key),
            None => println!("Couldn't open a browser here. Using a code instead.\n"),
        }
    }
    device(worker, auth_url).await
}

/// None when no browser could be opened, so the caller can fall back.
/// `open_browser` is the system browser in use and a fake one in tests.
async fn loopback(worker: &str, auth_url: &str, open_browser: impl FnOnce(&Url) -> bool) -> Result<Option<String>> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("could not open a local port to receive the login")?;
    let port = listener.local_addr()?.port();
    let pkce = Pkce::new();
    let state = random_hex();

    let mut url = Url::parse(auth_url)?;
    url.query_pairs_mut()
        .append_pair("port", &port.to_string())
        .append_pair("challenge", &pkce.challenge)
        .append_pair("state", &state);

    if !open_browser(&url) {
        return Ok(None);
    }
    println!("Browser opened. Approve the CLI there.");
    println!("If it didn't open, visit:\n  {url}\n");

    let code = tokio::time::timeout(LOGIN_TIMEOUT, receive_code(&listener, &state))
        .await
        .map_err(|_| anyhow!("Authorization timed out. Run `mochify auth login` to try again."))??;

    let client = reqwest::Client::new();
    let res = redeem(&client, worker, serde_json::json!({ "code": code, "verifier": pkce.verifier })).await?;
    match res.api_key {
        Some(key) => Ok(Some(key)),
        None => bail!(
            "The login was approved but could not be completed ({}). Run `mochify auth login` again.",
            res.error.unwrap_or_else(|| "unknown error".into())
        ),
    }
}

/// Waits for the browser to arrive at /callback with our state, answers it, and
/// returns the code. Anything else that connects is answered and ignored.
async fn receive_code(listener: &TcpListener, state: &str) -> Result<String> {
    loop {
        let (mut stream, _) = listener.accept().await?;
        let mut buf = vec![0u8; 8192];
        let mut len = 0;
        // Only the request line matters; stop at the end of the headers.
        while len < buf.len() {
            let n = match stream.read(&mut buf[len..]).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            len += n;
            if buf[..len].windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let request = String::from_utf8_lossy(&buf[..len]);
        let outcome = parse_callback(&request, state);
        let (status, body) = match &outcome {
            Callback::Code(_) => ("200 OK", "Mochify CLI is signed in. You can close this tab and return to your terminal."),
            Callback::WrongState => ("400 Bad Request", "This sign-in link is not for this terminal. Run `mochify auth login` again."),
            Callback::NotOurs => ("404 Not Found", "Not found."),
        };
        let page = format!(
            "<!doctype html><meta charset=utf-8><title>Mochify CLI</title>\
             <body style=\"font:16px system-ui,sans-serif;max-width:32em;margin:4em auto;padding:0 1em\">\
             <p>{body}</p></body>"
        );
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
             Cache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n{page}",
            page.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.shutdown().await;
        if let Callback::Code(code) = outcome {
            return Ok(code);
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Callback {
    Code(String),
    WrongState,
    NotOurs,
}

pub fn parse_callback(request: &str, state: &str) -> Callback {
    let Some(target) = request
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("GET "))
        .and_then(|rest| rest.split(' ').next())
    else {
        return Callback::NotOurs;
    };
    let Ok(url) = Url::parse(&format!("http://127.0.0.1{target}")) else {
        return Callback::NotOurs;
    };
    if url.path() != "/callback" {
        return Callback::NotOurs;
    }
    let get = |name: &str| url.query_pairs().find(|(k, _)| k == name).map(|(_, v)| v.into_owned());
    if get("state").as_deref() != Some(state) {
        return Callback::WrongState;
    }
    match get("code") {
        Some(code) if !code.is_empty() => Callback::Code(code),
        _ => Callback::NotOurs,
    }
}

#[derive(serde::Deserialize)]
struct DeviceStart {
    #[serde(rename = "deviceId")]
    device_id: String,
    #[serde(rename = "userCode")]
    user_code: String,
    #[serde(rename = "expiresIn")]
    expires_in: u64,
    interval: u64,
}

async fn device(worker: &str, auth_url: &str) -> Result<String> {
    device_with(worker, auth_url, |_| {}).await
}

/// `on_code` sees the user code as it is printed; tests use it to "type" it.
async fn device_with(worker: &str, auth_url: &str, on_code: impl FnOnce(&str)) -> Result<String> {
    let pkce = Pkce::new();
    let client = reqwest::Client::new();
    let start = client
        .post(format!("{worker}/v1/cli/device"))
        .json(&serde_json::json!({ "challenge": pkce.challenge }))
        .send()
        .await
        .context("could not reach the Mochify login service")?
        .error_for_status()
        .context("the Mochify login service refused to start a login")?
        .json::<DeviceStart>()
        .await
        .context("the login service sent a response that could not be parsed")?;

    println!("On any device, open:\n  {auth_url}\nand enter the code:\n  {}\n", start.user_code);
    println!("Only enter it on mochify.app, and only if you ran this command yourself.");
    on_code(&start.user_code);

    let deadline = Instant::now() + Duration::from_secs(start.expires_in.min(LOGIN_TIMEOUT.as_secs()));
    let interval = Duration::from_secs(start.interval.clamp(1, 30));
    loop {
        if Instant::now() >= deadline {
            bail!("Authorization timed out. Run `mochify auth login` to try again.");
        }
        tokio::time::sleep(interval).await;
        let Ok(res) = redeem(&client, worker, serde_json::json!({ "deviceId": start.device_id, "verifier": pkce.verifier })).await
        else {
            continue; // a dropped poll is not a failed login
        };
        if let Some(key) = res.api_key {
            return Ok(key);
        }
        match res.error.as_deref() {
            Some("authorization_pending") => continue,
            Some(other) => bail!("The login could not be completed ({other}). Run `mochify auth login` again."),
            None => continue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // End to end against the worker's real src/cliAuth.ts, served by a small
    // harness (MOCHIFY_TEST_WORKER=http://127.0.0.1:PORT). The harness exposes
    // the two internal approval calls as /test/approve and /test/device-approve
    // so a test can play the signed-in user. Skipped when the variable is unset.
    fn harness() -> Option<String> {
        std::env::var("MOCHIFY_TEST_WORKER").ok()
    }

    #[tokio::test]
    async fn loopback_login_against_the_worker() {
        let Some(worker) = harness() else { return };
        let w = worker.clone();
        let key = loopback(&worker, "https://mochify.app/auth/cli", move |url| {
            // The browser: the user approves, the frontend gets a code from the
            // worker, and the browser is redirected to the CLI's listener.
            let get = |n: &str| url.query_pairs().find(|(k, _)| k == n).unwrap().1.into_owned();
            let (port, challenge, state) = (get("port"), get("challenge"), get("state"));
            tokio::spawn(async move {
                let c = reqwest::Client::new();
                let approved: serde_json::Value = c
                    .post(format!("{w}/test/approve"))
                    .json(&serde_json::json!({ "userId": "user-1", "challenge": challenge }))
                    .send().await.unwrap().json().await.unwrap();
                let code = approved["code"].as_str().unwrap().to_owned();
                let page = c
                    .get(format!("http://127.0.0.1:{port}/callback?code={code}&state={state}"))
                    .send().await.unwrap();
                assert_eq!(page.status(), 200);
            });
            true
        })
        .await
        .unwrap()
        .expect("browser was 'opened'");
        assert_eq!(key.len(), 64);
    }

    #[tokio::test]
    async fn device_login_against_the_worker() {
        let Some(worker) = harness() else { return };
        let w = worker.clone();
        let key = device_with(&worker, "https://mochify.app/auth/cli", move |user_code| {
            let user_code = user_code.to_lowercase(); // typed, not pasted
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(1500)).await; // a poll or two first
                let res = reqwest::Client::new()
                    .post(format!("{w}/test/device-approve"))
                    .json(&serde_json::json!({ "userId": "user-2", "userCode": user_code }))
                    .send().await.unwrap();
                assert_eq!(res.status(), 200);
            });
        })
        .await
        .unwrap();
        assert_eq!(key.len(), 64);
    }

    #[test]
    fn challenge_is_s256_of_the_verifier() {
        let p = Pkce::new();
        assert_eq!(p.verifier.len(), 43);
        assert_eq!(p.challenge, URL_SAFE_NO_PAD.encode(Sha256::digest(p.verifier.as_bytes())));
        assert_eq!(p.challenge.len(), 43);
    }

    #[test]
    fn rfc7636_example() {
        let c = URL_SAFE_NO_PAD.encode(Sha256::digest(b"dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"));
        assert_eq!(c, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn callback_is_accepted_only_with_our_state() {
        let req = |target: &str| format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
        assert_eq!(parse_callback(&req("/callback?code=abc&state=s1"), "s1"), Callback::Code("abc".into()));
        assert_eq!(parse_callback(&req("/callback?state=s1&code=abc"), "s1"), Callback::Code("abc".into()));
        assert_eq!(parse_callback(&req("/callback?code=abc&state=other"), "s1"), Callback::WrongState);
        assert_eq!(parse_callback(&req("/callback?code=abc"), "s1"), Callback::WrongState);
        assert_eq!(parse_callback(&req("/callback?state=s1"), "s1"), Callback::NotOurs);
        assert_eq!(parse_callback(&req("/favicon.ico"), "s1"), Callback::NotOurs);
        assert_eq!(parse_callback("POST /callback?code=abc&state=s1 HTTP/1.1\r\n\r\n", "s1"), Callback::NotOurs);
        assert_eq!(parse_callback("", "s1"), Callback::NotOurs);
    }
}
