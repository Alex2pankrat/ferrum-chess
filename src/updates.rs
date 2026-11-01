//! An explicit, user-initiated check against the official releases API.
use semver::Version;
use std::time::Duration;

const API: &str = "https://api.github.com/repos/Alex2pankrat/ferrum-chess/releases/latest";
pub const RELEASE_PAGE: &str = "https://github.com/Alex2pankrat/ferrum-chess/releases/latest";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    Available(String),
    Current(String),
}

fn parse_release(body: &str, current: &Version) -> Result<UpdateStatus, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|_| "Invalid release response".to_owned())?;
    if value["draft"].as_bool() != Some(false) || value["prerelease"].as_bool() != Some(false) {
        return Err("The latest release is not a published stable release".into());
    }
    let tag = value["tag_name"]
        .as_str()
        .ok_or("Release has no version tag")?;
    let version = Version::parse(tag.strip_prefix('v').unwrap_or(tag))
        .map_err(|_| "Release has an invalid version tag")?;
    if !version.pre.is_empty() {
        return Err("The latest release is not stable".into());
    }
    if version > *current {
        Ok(UpdateStatus::Available(version.to_string()))
    } else {
        Ok(UpdateStatus::Current(version.to_string()))
    }
}

fn check_url(url: &str, current: &Version) -> Result<UpdateStatus, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(12)))
        .max_redirects(0)
        .build()
        .into();
    let mut response = agent
        .get(url)
        .header(
            "User-Agent",
            concat!("Ferrum-Chess/", env!("CARGO_PKG_VERSION")),
        )
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("Update check failed: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("Update check returned HTTP {}", response.status()));
    }
    let body = response
        .body_mut()
        .with_config()
        .limit(64 * 1024)
        .read_to_string()
        .map_err(|e| format!("Cannot read update response: {e}"))?;
    parse_release(&body, current)
}

/// Performs network I/O only when explicitly invoked by the user.
pub fn check() -> Result<UpdateStatus, String> {
    let current =
        Version::parse(env!("CARGO_PKG_VERSION")).map_err(|_| "Invalid application version")?;
    check_url(API, &current)
}

/// Opens a fixed official URL, never one supplied by the response.
pub fn open_release() -> Result<(), String> {
    open::that_detached(RELEASE_PAGE).map_err(|e| format!("Cannot open browser: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compares_numeric_versions_and_rejects_invalid_or_unstable_releases() {
        let current = Version::parse("0.9.0").unwrap();
        assert_eq!(
            parse_release(
                r#"{"tag_name":"v0.10.0","draft":false,"prerelease":false}"#,
                &current
            )
            .unwrap(),
            UpdateStatus::Available("0.10.0".into())
        );
        assert!(matches!(
            parse_release(
                r#"{"tag_name":"v0.8.0","draft":false,"prerelease":false}"#,
                &current
            ),
            Ok(UpdateStatus::Current(_))
        ));
        for body in [
            "{}",
            "bad",
            r#"{"tag_name":"v1.0.0-rc.1","draft":false,"prerelease":false}"#,
            r#"{"tag_name":"v1.0.0","draft":true,"prerelease":false}"#,
        ] {
            assert!(parse_release(body, &current).is_err());
        }
    }
    #[test]
    fn live_github_check_when_enabled_in_ci() {
        if std::env::var_os("FERRUM_TEST_LIVE_UPDATES").is_some() {
            check().expect("official GitHub update check");
        }
    }
    #[test]
    fn request_is_manual_and_parses_http_response() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut input = Vec::new();
            let mut chunk = [0u8; 1024];
            while !input.ends_with(b"\r\n\r\n") && input.len() < 8192 {
                let count = stream.read(&mut chunk).unwrap();
                if count == 0 {
                    break;
                }
                input.extend_from_slice(&chunk[..count]);
            }
            let request = String::from_utf8_lossy(&input).to_ascii_lowercase();
            assert!(request.contains("user-agent: ferrum-chess/"));
            let body = r#"{"tag_name":"v2.0.0","draft":false,"prerelease":false}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });
        assert_eq!(
            check_url(
                &format!("http://{address}/latest"),
                &Version::parse("1.0.0").unwrap()
            )
            .unwrap(),
            UpdateStatus::Available("2.0.0".into())
        );
        server.join().unwrap();
    }
}
