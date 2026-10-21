// Copyright (C) 2026 James Carthew
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Single sign-on login against a minimal fake homeserver (no Synapse needed): the
//! client is handed the SSO page URL, the "browser" follows the redirect back to the
//! client's loopback listener with a login token, and the token is exchanged for a session.

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

async fn handle(mut s: TcpStream) {
    let mut buf = vec![0u8; 16384];
    let mut n = 0;
    // Read until the headers (and any small body) have arrived.
    loop {
        let r = s.read(&mut buf[n..]).await.unwrap_or(0);
        if r == 0 {
            break;
        }
        n += r;
        let text = String::from_utf8_lossy(&buf[..n]);
        if let Some(h) = text.find("\r\n\r\n") {
            let len = text
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                })
                .unwrap_or(0);
            if n >= h + 4 + len {
                break;
            }
        }
    }
    let req = String::from_utf8_lossy(&buf[..n]).to_string();
    let first = req.lines().next().unwrap_or("");
    let body = if first.contains("/versions") {
        r#"{"versions":["v1.1","v1.5"]}"#.to_owned()
    } else if first.starts_with("GET") && first.contains("/login") {
        r#"{"flows":[{"type":"m.login.sso","identity_providers":[{"id":"google","name":"Google"}]},{"type":"m.login.token"}]}"#.to_owned()
    } else if first.starts_with("POST") && first.contains("/login") {
        assert!(
            req.contains("m.login.token") && req.contains("sekrit"),
            "{req}"
        );
        r#"{"user_id":"@sso:fake.test","access_token":"tok","device_id":"DEV1"}"#.to_owned()
    } else {
        r#"{"errcode":"M_UNRECOGNIZED","error":"unsupported"}"#.to_owned()
    };
    let status = if body.contains("M_UNRECOGNIZED") {
        "404 Not Found"
    } else {
        "200 OK"
    };
    let _ = s
        .write_all(
            format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await;
}

async fn fake_homeserver() -> String {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            if let Ok((s, _)) = l.accept().await {
                tokio::spawn(handle(s));
            }
        }
    });
    url
}

/// What a browser does after the user signs in: GET the client's redirect with the token.
async fn visit(url: &str) {
    let rest = url.strip_prefix("http://").expect("loopback http url");
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let mut s = TcpStream::connect(host).await.unwrap();
    s.write_all(
        format!("GET /{path} HTTP/1.1\r\nhost: {host}\r\nconnection: close\r\n\r\n").as_bytes(),
    )
    .await
    .unwrap();
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out).await;
}

#[tokio::test]
async fn sso_login_exchanges_the_browsers_token_for_a_session() {
    let hs = fake_homeserver().await;
    let dir = std::env::temp_dir().join(format!("msm-sso-{}", std::process::id()));
    let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let seen2 = seen.clone();
    let (client, json) =
        msm::matrix::authentication::login_sso_unsaved(&hs, &dir, |url| async move {
            *seen2.lock().unwrap() = url.clone();
            // The SSO page URL carries the loopback address to come back to.
            let redirect = url
                .split("redirectUrl=")
                .nth(1)
                .expect("redirectUrl parameter")
                .split('&')
                .next()
                .unwrap()
                .replace("%3A", ":")
                .replace("%2F", "/");
            tokio::spawn(async move { visit(&format!("{redirect}?loginToken=sekrit")).await });
            Ok(())
        })
        .await
        .expect("sso login");
    assert!(seen.lock().unwrap().contains("/login/sso/redirect"));
    assert_eq!(client.user_id().unwrap().as_str(), "@sso:fake.test");
    assert!(json.contains("DEV1"));
    let _ = std::fs::remove_dir_all(&dir);
}
