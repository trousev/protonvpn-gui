//! Ground-truth probe — sanctioned exception #1 (`docs/architecture.md` §0, §8).
//!
//! The CLI's self-report is not usable: measured, it printed `Your new IP address is
//! 149.88.27.213` while the actual egress was `149.22.89.89`. The one thing that can tell us
//! whether traffic is really going through the tunnel is asking an echo service what it sees.
//!
//! What this module is *not* allowed to become:
//!
//! * it never runs a program other than `curl`, and never with anything but a URL,
//! * **country and city are display-only.** Measured: the single address `205.147.16.120` was
//!   reported as NL by `ipinfo.io` and US by `ifconfig.co`. A "geo consistency check" would be a
//!   false-alarm generator, so there isn't one.
//! * **the reading is a fact, not a verdict.** The application keeps one reading — where traffic
//!   leaves by, and from which service — and draws no conclusion from it. Comparing it with an
//!   earlier address was measured to be useless in the case that matters: an application started
//!   while the tunnel is already up reads the tunnel's own address as its first, so "did it
//!   change?" answers "no" about a tunnel that is working perfectly. Whether the address moved is
//!   for the reader to see (`docs/architecture.md` §8).
//! * the endpoint is picked **once** for the session, so two readings come from the same service
//!   and a database disagreement cannot look like a state change to the person comparing them.

use std::net::IpAddr;
use std::process::Command;

use crate::i18n::I18n;
use crate::model::{EgressReading, ProbeEndpoint};

/// Which address family to ask about. Both are queried separately: an IPv6 leak is invisible to
/// an IPv4-only probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    V4,
    V6,
}

impl Family {
    pub fn flag(self) -> &'static str {
        match self {
            Self::V4 => "--ipv4",
            Self::V6 => "--ipv6",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::V4 => "IPv4",
            Self::V6 => "IPv6",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeError {
    /// `curl` is not installed, or could not be run.
    CurlMissing(String),
    /// `curl` ran and failed: no route, timeout, IPv6 unsupported by the server.
    CurlFailed { code: Option<i32>, stderr: String },
    /// `curl` succeeded but the body is not what we expect.
    Unparseable(String),
}

impl ProbeError {
    /// One line under the `curl …` invocation in the console.
    ///
    /// A catalogue rather than `Display`, for the reason [`crate::socks5::Closed::describe`]
    /// gives: these sentences are read by a person, and `curl`'s own name, its exit code and its
    /// stderr are data.
    pub fn describe(&self, i18n: &I18n) -> String {
        match self {
            Self::CurlMissing(detail) => i18n.probe_curl_missing(detail),
            Self::CurlFailed { code, stderr } if stderr.trim().is_empty() => {
                i18n.probe_curl_failed(code.unwrap_or(-1) as i64)
            }
            Self::CurlFailed { code, stderr } => {
                i18n.probe_curl_failed_detail(code.unwrap_or(-1) as i64, stderr.trim())
            }
            Self::Unparseable(body) => i18n.probe_unparseable(body),
        }
    }
}

/// A probe pinned to one endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    endpoint: ProbeEndpoint,
}

impl Probe {
    pub fn endpoint(&self) -> ProbeEndpoint {
        self.endpoint
    }

    /// Picks the first endpoint that answers, and sticks with it for the whole session.
    ///
    /// `read` is injected so the fallback chain can be tested without a network.
    pub fn choose_with(
        mut read: impl FnMut(ProbeEndpoint) -> Result<EgressReading, ProbeError>,
    ) -> Option<Self> {
        for endpoint in [
            ProbeEndpoint::IfConfigCo,
            ProbeEndpoint::IpInfoIo,
            ProbeEndpoint::IfConfigMe,
        ] {
            if read(endpoint).is_ok() {
                return Some(Self { endpoint });
            }
        }
        None
    }

    /// Picks an endpoint for real, using `curl`.
    pub fn choose() -> Option<Self> {
        Self::choose_with(|endpoint| read(endpoint, Family::V4))
    }

    pub fn read(&self, family: Family) -> Result<EgressReading, ProbeError> {
        read(self.endpoint, family)
    }

    /// Both families, for the leak check. IPv6 support varies by server, so a failure there is
    /// reported as its own result rather than failing the whole probe.
    pub fn read_both(
        &self,
    ) -> (
        Result<EgressReading, ProbeError>,
        Result<EgressReading, ProbeError>,
    ) {
        (self.read(Family::V4), self.read(Family::V6))
    }
}

/// Asks one endpoint, for real, through `curl`.
pub fn read(endpoint: ProbeEndpoint, family: Family) -> Result<EgressReading, ProbeError> {
    let body = curl(endpoint.url(), family)?;
    parse(endpoint, &body).ok_or(ProbeError::Unparseable(body))
}

/// Runs `curl <url>`. Nothing else is ever passed to it.
pub fn curl(url: &str, family: Family) -> Result<String, ProbeError> {
    let output = Command::new("curl")
        .args(["-sS", "--max-time", "8", family.flag(), url])
        .output()
        .map_err(|e| ProbeError::CurlMissing(e.to_string()))?;

    if !output.status.success() {
        return Err(ProbeError::CurlFailed {
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Parses the endpoint's JSON into the fields we use.
///
/// Tolerant in the usual way: all three services report different field names for the same facts,
/// and any of them may omit a field. The IP is the only required one, because it is the only one
/// that means anything.
pub fn parse(endpoint: ProbeEndpoint, body: &str) -> Option<EgressReading> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let object = value.as_object()?;

    let ip_text = match endpoint {
        ProbeEndpoint::IfConfigCo => object.get("ip")?.as_str()?,
        ProbeEndpoint::IpInfoIo => object.get("ip")?.as_str()?,
        ProbeEndpoint::IfConfigMe => object
            .get("ip_addr")
            .or_else(|| object.get("ip"))?
            .as_str()?,
    };
    let ip: IpAddr = ip_text.trim().parse().ok()?;

    let country = match endpoint {
        ProbeEndpoint::IfConfigCo => object.get("country_iso"),
        ProbeEndpoint::IpInfoIo => object.get("country"),
        ProbeEndpoint::IfConfigMe => None,
    }
    .and_then(|v| v.as_str())
    .map(str::to_string);

    let asn_org = match endpoint {
        ProbeEndpoint::IfConfigCo => object
            .get("asn_org")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| {
                object
                    .get("asn")
                    .map(|v| v.to_string().trim_matches('"').to_string())
            }),
        ProbeEndpoint::IpInfoIo => object
            .get("org")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        ProbeEndpoint::IfConfigMe => None,
    };

    Some(EgressReading {
        endpoint,
        ip,
        country,
        asn_org,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Locale;

    fn english() -> I18n {
        I18n::new(Locale::SOURCE)
    }

    #[test]
    fn parses_ifconfig_co() {
        let body =
            r#"{"ip":"149.22.89.89","country_iso":"CH","asn":60068,"asn_org":"Datacamp Limited"}"#;
        let reading = parse(ProbeEndpoint::IfConfigCo, body).unwrap();
        assert_eq!(reading.ip, "149.22.89.89".parse::<IpAddr>().unwrap());
        assert_eq!(reading.country.as_deref(), Some("CH"));
        assert_eq!(reading.asn_org.as_deref(), Some("Datacamp Limited"));
    }

    #[test]
    fn parses_ipinfo_io_and_its_different_field_names() {
        let body = r#"{"ip":"205.147.16.120","city":"Amsterdam","country":"NL","org":"AS208172 Proton AG"}"#;
        let reading = parse(ProbeEndpoint::IpInfoIo, body).unwrap();
        assert_eq!(reading.country.as_deref(), Some("NL"));
        assert_eq!(reading.asn_org.as_deref(), Some("AS208172 Proton AG"));
    }

    #[test]
    fn parses_ifconfig_me_ipv6() {
        let body = r#"{"ip_addr":"2a02:6ea0:c041:6652::34","user_agent":"curl","port":443}"#;
        let reading = parse(ProbeEndpoint::IfConfigMe, body).unwrap();
        assert!(reading.ip.is_ipv6());
        assert_eq!(reading.country, None);
    }

    #[test]
    fn refuses_a_body_without_a_usable_address() {
        assert_eq!(
            parse(ProbeEndpoint::IfConfigCo, r#"{"country_iso":"CH"}"#),
            None
        );
        assert_eq!(parse(ProbeEndpoint::IfConfigCo, "not json at all"), None);
        assert_eq!(
            parse(ProbeEndpoint::IfConfigCo, r#"{"ip":"not-an-ip"}"#),
            None
        );
        assert_eq!(
            parse(ProbeEndpoint::IfConfigMe, r#"{"user_agent":"curl"}"#),
            None
        );
    }

    #[test]
    fn the_fallback_chain_stops_at_the_first_endpoint_that_answers() {
        let mut tried = Vec::new();
        let probe = Probe::choose_with(|endpoint| {
            tried.push(endpoint);
            if endpoint == ProbeEndpoint::IfConfigCo {
                Err(ProbeError::CurlFailed {
                    code: Some(28),
                    stderr: "timeout".into(),
                })
            } else {
                Ok(EgressReading {
                    endpoint,
                    ip: "1.2.3.4".parse().unwrap(),
                    country: None,
                    asn_org: None,
                })
            }
        })
        .unwrap();

        assert_eq!(
            tried,
            vec![ProbeEndpoint::IfConfigCo, ProbeEndpoint::IpInfoIo]
        );
        assert_eq!(probe.endpoint(), ProbeEndpoint::IpInfoIo);
    }

    #[test]
    fn no_endpoint_answering_is_reported_as_no_probe_rather_than_a_wrong_address() {
        let probe = Probe::choose_with(|_| Err(ProbeError::CurlMissing("no curl".into())));
        assert!(probe.is_none());
    }

    #[test]
    fn a_failure_to_parse_is_not_a_reading() {
        let error = ProbeError::Unparseable("{}".into());
        assert!(error.describe(&english()).contains("could not be parsed"));
        // A `curl` that failed says which code it failed with, and quotes what it said.
        let bare = ProbeError::CurlFailed {
            code: Some(28),
            stderr: String::new(),
        };
        assert_eq!(bare.describe(&english()), "curl finished with exit code 28");
        let talkative = ProbeError::CurlFailed {
            code: None,
            stderr: "connection timed out\n".into(),
        };
        assert_eq!(
            talkative.describe(&english()),
            "curl finished with exit code -1: connection timed out"
        );
        // And the parse itself never invents an address out of an empty object.
        assert_eq!(parse(ProbeEndpoint::IfConfigCo, "{}"), None);
    }

    #[test]
    fn the_probe_runs_only_curl_and_only_with_a_url() {
        // A guard on the shape of the exception: `curl -sS --max-time 8 --ipv4 <url>`, nothing else.
        let url = ProbeEndpoint::IfConfigCo.url();
        assert!(url.starts_with("https://"));
        assert!(!url.contains(' '));
        assert_eq!(Family::V4.flag(), "--ipv4");
        assert_eq!(Family::V6.flag(), "--ipv6");
    }
}
