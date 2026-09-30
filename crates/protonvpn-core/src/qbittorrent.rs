//! qBittorrent port push — sanctioned exception #3, **off by default**.
//!
//! `docs/architecture.md` §10.4. The point is to save the user a copy-paste, not to manage a
//! remote client:
//!
//! * localhost only — a non-local host is refused outright, no matter what the config says,
//! * disabled until the user turns it on, because enabling it changes another program's settings,
//! * every push is recorded in the console as an **honest pseudo-invocation**
//!   (`POST http://localhost:8080/…`), never dressed up as a `protonvpn` command.
//!
//! It speaks HTTP directly rather than pulling in an HTTP client: three requests to localhost do
//! not justify a dependency, and the shape of what we send is then visible in one place.

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::config::QBittorrent;

/// Localhost calls should be instant; anything slower is a misconfiguration worth reporting.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const IO_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QBittorrentError {
    /// The configured host is not a loopback address. Refused, always.
    NotLocalhost(String),
    /// Disabled in the config; a push would be a change nobody asked for.
    Disabled,
    Connect(String),
    Io(String),
    /// The server answered, but not with success.
    Http {
        request: String,
        status: u16,
        reason: String,
    },
    /// Login failed, so no push was attempted.
    AuthFailed {
        status: u16,
    },
}

impl std::fmt::Display for QBittorrentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotLocalhost(host) => write!(
                f,
                "хост `{host}` не является локальным: интеграция работает только с localhost"
            ),
            Self::Disabled => write!(f, "интеграция с qBittorrent выключена"),
            Self::Connect(e) => write!(f, "не удалось подключиться: {e}"),
            Self::Io(e) => write!(f, "ошибка ввода-вывода: {e}"),
            Self::Http {
                request,
                status,
                reason,
            } => write!(f, "{request} → {status} {reason}"),
            Self::AuthFailed { status } => {
                write!(f, "вход в qBittorrent не удался (HTTP {status})")
            }
        }
    }
}

impl std::error::Error for QBittorrentError {}

/// What to show in the console. Kept structured so the caller can render it verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushReport {
    /// e.g. `POST http://localhost:8080/api/v2/app/setPreferences {listen_port: 39949}`
    pub display: String,
    /// e.g. `→ 200 OK`
    pub result: String,
    pub port: u16,
}

/// Is this host loopback? The only host we will ever talk to.
pub fn is_localhost(host: &str) -> bool {
    let host = host.trim().trim_matches(|c| c == '[' || c == ']');
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => v4.is_loopback(),
        Ok(IpAddr::V6(v6)) => v6.is_loopback(),
        Err(_) => false,
    }
}

fn resolve(config: &QBittorrent) -> Result<SocketAddr, QBittorrentError> {
    if !is_localhost(&config.host) {
        return Err(QBittorrentError::NotLocalhost(config.host.clone()));
    }
    let host = config.host.trim().trim_matches(|c| c == '[' || c == ']');
    let address = if host.eq_ignore_ascii_case("localhost") {
        SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), config.port)
    } else {
        (host, config.port)
            .to_socket_addrs()
            .map_err(|e| QBittorrentError::Connect(e.to_string()))?
            .next()
            .ok_or_else(|| QBittorrentError::Connect("no address".into()))?
    };
    Ok(address)
}

/// The request line, shown to the user exactly as it is sent.
pub fn request_display(config: &QBittorrent, port: u16) -> String {
    format!(
        "POST http://{}:{}/api/v2/app/setPreferences {{listen_port: {port}}}",
        config.host, config.port
    )
}

/// Pushes `port` into qBittorrent's listen port. `password` is a parameter, never stored.
pub fn push_port(
    config: &QBittorrent,
    port: u16,
    password: &str,
) -> Result<PushReport, QBittorrentError> {
    if !config.enabled {
        return Err(QBittorrentError::Disabled);
    }
    let address = resolve(config)?;

    // One connection per request, and `Connection: close` on each: a client that pipelines onto a
    // stream the server has already closed is a bug waiting for the wrong qBittorrent version.
    let mut cookie = None;
    if !config.username.is_empty() {
        let body = form_encode(&[
            ("username", config.username.as_str()),
            ("password", password),
        ]);
        let response = request(address, "POST", "/api/v2/auth/login", &body, None)?;
        if response.status != 200 {
            return Err(QBittorrentError::AuthFailed {
                status: response.status,
            });
        }
        // qBittorrent answers "Fails." with a 200 when the credentials are wrong.
        if response.body.trim() == "Fails." {
            return Err(QBittorrentError::AuthFailed { status: 200 });
        }
        cookie = response.cookie;
    }

    let json = format!("{{\"listen_port\":{port}}}");
    let body = form_encode(&[("json", json.as_str())]);
    let display = request_display(config, port);
    let response = request(
        address,
        "POST",
        "/api/v2/app/setPreferences",
        &body,
        cookie.as_deref(),
    )?;

    if response.status != 200 {
        return Err(QBittorrentError::Http {
            request: display,
            status: response.status,
            reason: response.reason,
        });
    }

    Ok(PushReport {
        display,
        result: format!("→ {} {}", response.status, response.reason),
        port,
    })
}

struct HttpResponse {
    status: u16,
    reason: String,
    body: String,
    cookie: Option<String>,
}

fn request(
    address: SocketAddr,
    method: &str,
    path: &str,
    body: &str,
    cookie: Option<&str>,
) -> Result<HttpResponse, QBittorrentError> {
    let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)
        .map_err(|e| QBittorrentError::Connect(e.to_string()))?;
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|e| QBittorrentError::Io(e.to_string()))?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|e| QBittorrentError::Io(e.to_string()))?;

    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: \
         application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(cookie) = cookie {
        head.push_str(&format!("Cookie: {cookie}\r\n"));
    }
    head.push_str("\r\n");

    stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.write_all(body.as_bytes()))
        .map_err(|e| QBittorrentError::Io(e.to_string()))?;

    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|e| QBittorrentError::Io(e.to_string()))?;
    let text = String::from_utf8_lossy(&raw);

    let mut lines = text.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| QBittorrentError::Io("empty response".into()))?;
    let mut parts = status_line.splitn(3, ' ');
    let _http = parts.next();
    let status: u16 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| QBittorrentError::Io(format!("bad status line: {status_line}")))?;
    let reason = parts.next().unwrap_or("").to_string();

    let mut cookie = None;
    for line in lines.by_ref() {
        if line.is_empty() {
            break;
        }
        if let Some(value) = line
            .strip_prefix("Set-Cookie: ")
            .or_else(|| line.strip_prefix("set-cookie: "))
        {
            let pair = value.split(';').next().unwrap_or("").trim().to_string();
            if !pair.is_empty() {
                cookie = Some(pair);
            }
        }
    }

    // The body is what follows the blank line; treating the status line as the body is how a
    // "Fails." answer gets mistaken for success.
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default();

    Ok(HttpResponse {
        status,
        reason,
        body,
        cookie,
    })
}

/// Percent-encodes a form body. Enough for the two fields we send.
fn form_encode(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(key, value)| format!("{}={}", percent_encode(key), percent_encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    fn config(port: u16) -> QBittorrent {
        QBittorrent {
            enabled: true,
            host: "localhost".into(),
            port,
            username: String::new(),
        }
    }

    /// A stand-in qBittorrent: real HTTP on localhost, no external dependency.
    fn fake_server(
        expected_requests: usize,
        status: &'static str,
        answer_login: bool,
    ) -> (u16, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            let mut requests = Vec::new();
            for _ in 0..expected_requests {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let mut raw = Vec::new();
                let mut buffer = [0u8; 4096];
                // Read headers and (once declared) the body.
                while let Ok(n) = stream.read(&mut buffer) {
                    if n == 0 {
                        break;
                    }
                    raw.extend_from_slice(&buffer[..n]);
                    let text = String::from_utf8_lossy(&raw).to_string();
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let length = head
                            .lines()
                            .find_map(|l| l.strip_prefix("Content-Length: "))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        if body.len() >= length {
                            break;
                        }
                    }
                }
                let text = String::from_utf8_lossy(&raw).to_string();
                let is_login = text.starts_with("POST /api/v2/auth/login");
                let body = if is_login && answer_login { "Ok." } else { "" };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                requests.push(text);
            }
            requests
        });
        (port, handle)
    }

    #[test]
    fn refuses_any_host_that_is_not_loopback() {
        let mut config = config(8080);
        config.host = "192.168.1.50".into();
        let error = push_port(&config, 39949, "").unwrap_err();
        assert_eq!(error, QBittorrentError::NotLocalhost("192.168.1.50".into()));
        assert!(error.to_string().contains("только с localhost"));
    }

    #[test]
    fn refuses_to_do_anything_while_disabled() {
        let mut config = config(8080);
        config.enabled = false;
        assert_eq!(
            push_port(&config, 39949, "").unwrap_err(),
            QBittorrentError::Disabled
        );
    }

    #[test]
    fn recognises_loopback_spellings() {
        for host in ["localhost", "127.0.0.1", "::1", "[::1]", "127.2.3.4"] {
            assert!(is_localhost(host), "{host} should be local");
        }
        for host in ["example.com", "10.0.0.1", "8.8.8.8", "192.168.1.1"] {
            assert!(!is_localhost(host), "{host} should not be local");
        }
    }

    #[test]
    fn posts_the_port_and_reports_the_status() {
        let (port, server) = fake_server(1, "200 OK", false);
        let report = push_port(&config(port), 39949, "").unwrap();
        assert_eq!(report.port, 39949);
        assert_eq!(report.result, "→ 200 OK");
        assert_eq!(
            report.display,
            format!(
                "POST http://localhost:{port}/api/v2/app/setPreferences {{listen_port: 39949}}"
            )
        );

        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("POST /api/v2/app/setPreferences"));
        assert!(requests[0].contains("listen_port"), "{}", requests[0]);
        assert!(requests[0].contains("39949"));
    }

    #[test]
    fn logs_in_first_when_credentials_are_configured() {
        let (port, server) = fake_server(2, "200 OK", true);
        let mut config = config(port);
        config.username = "admin".into();
        let report = push_port(&config, 50000, "secret").unwrap();
        assert_eq!(report.port, 50000);

        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("POST /api/v2/auth/login"));
        assert!(requests[0].contains("username=admin"));
        assert!(requests[0].contains("password=secret"));
        assert!(requests[1].starts_with("POST /api/v2/app/setPreferences"));
    }

    #[test]
    fn a_non_200_is_an_error_and_not_a_success_story() {
        let (port, server) = fake_server(1, "403 Forbidden", false);
        let error = push_port(&config(port), 39949, "").unwrap_err();
        match error {
            QBittorrentError::Http { status, .. } => assert_eq!(status, 403),
            other => panic!("expected an HTTP error, got {other:?}"),
        }
        let _ = server.join();
    }

    #[test]
    fn qbittorrents_200_fails_answer_is_treated_as_an_auth_failure() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut raw = Vec::new();
            let mut buffer = [0u8; 4096];
            let mut declared = None;
            while let Ok(n) = stream.read(&mut buffer) {
                if n == 0 {
                    break;
                }
                raw.extend_from_slice(&buffer[..n]);
                let text = String::from_utf8_lossy(&raw).to_string();
                if declared.is_none() {
                    declared = text
                        .lines()
                        .find_map(|l| l.strip_prefix("Content-Length: "))
                        .and_then(|v| v.trim().parse::<usize>().ok());
                }
                if let Some((_, body)) = text.split_once("\r\n\r\n")
                    && body.len() >= declared.unwrap_or(0)
                {
                    break;
                }
            }
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nFails.",
            );
        });

        let mut config = config(port);
        config.username = "admin".into();
        let error = push_port(&config, 39949, "wrong").unwrap_err();
        assert_eq!(error, QBittorrentError::AuthFailed { status: 200 });
        handle.join().unwrap();
    }

    #[test]
    fn form_encoding_escapes_what_http_needs() {
        assert_eq!(percent_encode("hello world"), "hello+world");
        assert_eq!(
            percent_encode(r#"{"listen_port":39949}"#),
            "%7B%22listen_port%22%3A39949%7D"
        );
    }
}
