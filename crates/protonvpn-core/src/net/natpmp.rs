//! NAT-PMP client for the port-forwarding lease — sanctioned exception #2.
//!
//! Why this exists at all: `protonvpn connect` exits, and with it the Local Agent, which was the
//! only thing renewing the forwarded port. The CLI says so itself — *"The port assignment requires
//! an external script to maintain the lease and retrieve the port number. Without the script, the
//! assigned port expires."* Showing a port we are not maintaining would be misinformation
//! (`docs/cli-surface.md` §4.7), so we maintain it.
//!
//! Why this is not "a competing library": NAT-PMP is IETF RFC 6886, the port is its IANA-assigned
//! one, and the gateway address is in Proton's own public manual-setup guide. This module is a
//! dozen datagrams, not a protocol stack.
//!
//! Shape (all fields big-endian, RFC 6886 §3):
//!
//! ```text
//! request  public address : version=0 | op=0                                  (2 bytes)
//! request  map            : version=0 | op(1=udp,2=tcp) | rsvd | internal | suggested | lifetime
//! response public address : version | op=128 | result | epoch | external ip
//! response map            : version | op=128+opcode | result | epoch | internal | external | lifetime
//! ```
//!
//! Risk containment, also from §10.1: an opcode-0 request is sent first. If the gateway does not
//! answer, the honest outcome is "port forwarding unavailable", not a port number we guessed.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use crate::i18n::I18n;

/// Documented gateway, from Proton's public manual-setup guide — **not** from the CLI.
pub const GATEWAY_IP: Ipv4Addr = Ipv4Addr::new(10, 2, 0, 1);
/// The IANA-assigned NAT-PMP port.
pub const GATEWAY_PORT: u16 = 5351;

/// The lease length Proton's own instructions use (`-a 1 0 udp 60`).
pub const LEASE: Duration = Duration::from_secs(60);
/// Renew well before expiry: two thirds of the lease leaves room for a lost datagram or two.
pub const RENEW_EVERY: Duration = Duration::from_secs(40);

const OPCODE_PUBLIC_ADDRESS: u8 = 0;
const OPCODE_MAP_UDP: u8 = 1;
const OPCODE_MAP_TCP: u8 = 2;
const RESPONSE_FLAG: u8 = 0x80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Udp,
    Tcp,
}

impl Protocol {
    pub fn opcode(self) -> u8 {
        match self {
            Self::Udp => OPCODE_MAP_UDP,
            Self::Tcp => OPCODE_MAP_TCP,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Udp => "UDP",
            Self::Tcp => "TCP",
        }
    }
}

/// A granted mapping. The gateway returns the same external port for TCP and UDP, equal to the
/// internal one (measured), but we do not rely on that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mapping {
    pub internal_port: u16,
    pub external_port: u16,
    pub lifetime: Duration,
}

/// What about the gateway's answer was wrong.
///
/// Kept as data rather than as a sentence: the answer is decoded on whichever thread asked, and
/// the sentence is built by [`NatPmpError::describe`], where the locale is known. Every number here
/// is a protocol byte count or opcode and is data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Malformed {
    /// Fewer bytes than the RFC's fixed-size answer.
    Length { got: usize, expected: usize },
    /// A protocol version this client does not speak.
    Version(u8),
    /// A response flag or opcode that does not match the request. `expected` is absent where the
    /// answer stands alone, as the public-address response does.
    Opcode { got: u8, expected: Option<u8> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NatPmpError {
    Io(String),
    /// The gateway never answered within the retry budget.
    Timeout,
    /// The gateway answered with a non-zero result code.
    Refused {
        code: u16,
    },
    Malformed(Malformed),
}

impl NatPmpError {
    /// One line for the console note that records the lease attempt, and the reason the Overview
    /// page's port-forwarding row gives.
    ///
    /// A catalogue rather than `Display`, for the reason [`crate::socks5::Closed::describe`]
    /// gives: every one of these is shown to a person. The numbers inside them — an opcode, a
    /// result code, a byte count — are protocol data and are never translated.
    pub fn describe(&self, i18n: &I18n) -> String {
        match self {
            Self::Io(detail) => i18n.natpmp_io(detail),
            Self::Timeout => i18n.natpmp_timeout(GATEWAY_IP.to_string()),
            Self::Refused { code } => {
                i18n.natpmp_refused(*code as i64, result_message(*code, i18n))
            }
            Self::Malformed(why) => i18n.natpmp_malformed(describe_malformed(why, i18n)),
        }
    }
}

/// The sentence for one malformed answer, in the caller's locale.
fn describe_malformed(why: &Malformed, i18n: &I18n) -> String {
    match why {
        Malformed::Length { got, expected } => {
            i18n.natpmp_malformed_length(*got as i64, *expected as i64)
        }
        Malformed::Version(version) => i18n.natpmp_malformed_version(*version as i64),
        Malformed::Opcode {
            got,
            expected: Some(expected),
        } => i18n.natpmp_malformed_opcode_wanted(*got as i64, *expected as i64),
        Malformed::Opcode {
            got,
            expected: None,
        } => i18n.natpmp_malformed_opcode(*got as i64),
    }
}

/// What the RFC's result codes mean, so the console says something a human can act on. The number
/// is printed beside the meaning by `natpmp-refused`, and is never part of it.
fn result_message(code: u16, i18n: &I18n) -> String {
    match code {
        0 => i18n.natpmp_result_ok(),
        1 => i18n.natpmp_result_version(),
        2 => i18n.natpmp_result_not_authorized(),
        3 => i18n.natpmp_result_network(),
        4 => i18n.natpmp_result_resources(),
        5 => i18n.natpmp_result_unsupported(),
        _ => i18n.natpmp_result_unknown(),
    }
}

/// Encodes an opcode-0 request: version 0, opcode 0.
pub fn encode_public_address_request() -> [u8; 2] {
    [0, OPCODE_PUBLIC_ADDRESS]
}

/// Encodes a mapping request. `internal` and `suggested` are 0 to let the gateway choose, which is
/// what Proton's own instructions do (`-a 1 0 udp 60`).
pub fn encode_map_request(
    protocol: Protocol,
    internal_port: u16,
    suggested_external: u16,
    lifetime: Duration,
) -> [u8; 12] {
    let mut request = [0u8; 12];
    request[1] = protocol.opcode();
    request[4..6].copy_from_slice(&internal_port.to_be_bytes());
    request[6..8].copy_from_slice(&suggested_external.to_be_bytes());
    request[8..12].copy_from_slice(&(lifetime.as_secs() as u32).to_be_bytes());
    request
}

/// Decodes an opcode-0 response into the external address the gateway sees.
pub fn decode_public_address_response(response: &[u8]) -> Result<IpAddr, NatPmpError> {
    if response.len() < 12 {
        return Err(NatPmpError::Malformed(Malformed::Length {
            got: response.len(),
            expected: 12,
        }));
    }
    if response[0] != 0 {
        return Err(NatPmpError::Malformed(Malformed::Version(response[0])));
    }
    if response[1] != RESPONSE_FLAG | OPCODE_PUBLIC_ADDRESS {
        return Err(NatPmpError::Malformed(Malformed::Opcode {
            got: response[1],
            expected: None,
        }));
    }
    let code = u16::from_be_bytes([response[2], response[3]]);
    if code != 0 {
        return Err(NatPmpError::Refused { code });
    }
    let octets = [response[8], response[9], response[10], response[11]];
    Ok(IpAddr::V4(Ipv4Addr::from(octets)))
}

/// Decodes a mapping response.
pub fn decode_map_response(response: &[u8], expected: Protocol) -> Result<Mapping, NatPmpError> {
    if response.len() < 16 {
        return Err(NatPmpError::Malformed(Malformed::Length {
            got: response.len(),
            expected: 16,
        }));
    }
    if response[1] != RESPONSE_FLAG | expected.opcode() {
        return Err(NatPmpError::Malformed(Malformed::Opcode {
            got: response[1],
            expected: Some(RESPONSE_FLAG | expected.opcode()),
        }));
    }
    let code = u16::from_be_bytes([response[2], response[3]]);
    if code != 0 {
        return Err(NatPmpError::Refused { code });
    }
    Ok(Mapping {
        internal_port: u16::from_be_bytes([response[8], response[9]]),
        external_port: u16::from_be_bytes([response[10], response[11]]),
        lifetime: Duration::from_secs(u32::from_be_bytes([
            response[12],
            response[13],
            response[14],
            response[15],
        ]) as u64),
    })
}

/// The client. Cheap to build; each request opens its own socket, so there is no state to corrupt
/// and nothing to leak when the connection goes away.
#[derive(Debug, Clone)]
pub struct NatPmp {
    gateway: SocketAddr,
    attempts: u32,
    first_timeout: Duration,
}

impl Default for NatPmp {
    fn default() -> Self {
        Self::with_gateway(SocketAddr::new(IpAddr::V4(GATEWAY_IP), GATEWAY_PORT))
    }
}

impl NatPmp {
    pub fn with_gateway(gateway: SocketAddr) -> Self {
        Self {
            gateway,
            // RFC 6886 §3.1 suggests a doubling retry ladder; three tries is ~1.75 s in total,
            // which is short enough to keep the UI responsive and long enough to survive a loss.
            attempts: 3,
            first_timeout: Duration::from_millis(250),
        }
    }

    pub fn gateway(&self) -> SocketAddr {
        self.gateway
    }

    /// Opcode 0: is the gateway answering at all, and what address does it see us on?
    pub fn public_address(&self) -> Result<IpAddr, NatPmpError> {
        let response = self.exchange(&encode_public_address_request(), 12)?;
        decode_public_address_response(&response)
    }

    pub fn map(
        &self,
        protocol: Protocol,
        internal_port: u16,
        suggested_external: u16,
        lifetime: Duration,
    ) -> Result<Mapping, NatPmpError> {
        let request = encode_map_request(protocol, internal_port, suggested_external, lifetime);
        let response = self.exchange(&request, 16)?;
        decode_map_response(&response, protocol)
    }

    /// Releases a mapping by asking for a zero lifetime. Best-effort: a failure here must not
    /// stop a disconnect.
    pub fn release(&self, protocol: Protocol, internal_port: u16) -> Result<(), NatPmpError> {
        self.map(protocol, internal_port, 0, Duration::ZERO)
            .map(|_| ())
    }

    /// Sends `request` and waits for a response, retrying on the RFC's doubling ladder.
    fn exchange(&self, request: &[u8], expected: usize) -> Result<Vec<u8>, NatPmpError> {
        let socket = UdpSocket::bind(("0.0.0.0", 0)).map_err(|e| NatPmpError::Io(e.to_string()))?;
        socket
            .connect(self.gateway)
            .map_err(|e| NatPmpError::Io(e.to_string()))?;

        let started = Instant::now();
        let mut timeout = self.first_timeout;
        let mut buffer = [0u8; 64];

        for _ in 0..self.attempts {
            socket
                .send(request)
                .map_err(|e| NatPmpError::Io(e.to_string()))?;

            // Never wait past the ladder's own budget, so a hung gateway costs one bounded delay.
            let remaining = timeout.saturating_sub(started.elapsed());
            let _ = socket.set_read_timeout(Some(remaining.max(Duration::from_millis(50))));
            match socket.recv(&mut buffer) {
                Ok(n) if n >= expected => return Ok(buffer[..n].to_vec()),
                Ok(n) => {
                    return Err(NatPmpError::Malformed(Malformed::Length {
                        got: n,
                        expected,
                    }));
                }
                Err(_) => timeout *= 2,
            }
        }
        Err(NatPmpError::Timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Locale;
    use std::net::UdpSocket;
    use std::thread;

    fn english() -> I18n {
        I18n::new(Locale::SOURCE)
    }

    #[test]
    fn encodes_requests_exactly_as_the_rfc_says() {
        assert_eq!(encode_public_address_request(), [0, 0]);

        let request = encode_map_request(Protocol::Udp, 0, 0, Duration::from_secs(60));
        assert_eq!(
            request,
            [0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 60],
            "`natpmpc -a 1 0 udp 60` in 12 bytes"
        );

        let request = encode_map_request(Protocol::Tcp, 39949, 0, Duration::from_secs(60));
        assert_eq!(&request[..2], &[0, 2]);
        assert_eq!(u16::from_be_bytes([request[4], request[5]]), 39949);
    }

    #[test]
    fn decodes_the_measured_public_address_response() {
        // result=0, epoch, external 46.29.25.99 — the shape the real gateway returned.
        let mut response = vec![0u8, 0x80, 0, 0];
        response.extend_from_slice(&0x0000_0001u32.to_be_bytes());
        response.extend_from_slice(&[46, 29, 25, 99]);
        assert_eq!(
            decode_public_address_response(&response).unwrap(),
            "46.29.25.99".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn decodes_the_measured_mapping_response() {
        let mut response = vec![0u8, 0x80 | OPCODE_MAP_UDP, 0, 0];
        response.extend_from_slice(&0x0000_0001u32.to_be_bytes());
        response.extend_from_slice(&39949u16.to_be_bytes());
        response.extend_from_slice(&39949u16.to_be_bytes());
        response.extend_from_slice(&60u32.to_be_bytes());
        let mapping = decode_map_response(&response, Protocol::Udp).unwrap();
        assert_eq!(
            mapping,
            Mapping {
                internal_port: 39949,
                external_port: 39949,
                lifetime: Duration::from_secs(60)
            }
        );
    }

    #[test]
    fn a_refusal_is_not_a_mapping() {
        let mut response = vec![0u8, 0x80 | OPCODE_MAP_UDP];
        response.extend_from_slice(&2u16.to_be_bytes());
        response.extend_from_slice(&[0u8; 12]);
        match decode_map_response(&response, Protocol::Udp) {
            Err(error @ NatPmpError::Refused { code }) => {
                assert_eq!(code, 2);
                let described = error.describe(&english());
                assert!(described.contains("P2P"), "{described}");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_refusal_is_worded_in_the_locale_it_is_described_in() {
        // The point of `describe` rather than `Display`: the same error, one catalogue later, is a
        // different sentence — and the code inside it is data in both.
        let error = NatPmpError::Refused { code: 2 };
        let english = error.describe(&english());
        let russian = error.describe(&I18n::new(Locale::from_id("ru").unwrap()));
        assert_ne!(english, russian);
        assert!(english.contains("2"), "{english}");
        assert!(russian.contains("2"), "{russian}");
        assert!(
            english.contains("P2P") && russian.contains("P2P"),
            "{russian}"
        );
    }

    /// A byte count is a count, and Russian declines the word for "byte" three ways. A translation
    /// that reuses the many-form for four is the mistake this pins.
    #[test]
    fn a_byte_count_is_declined_and_not_merely_substituted() {
        let russian = I18n::new(Locale::from_id("ru").unwrap());
        let describe = |got| {
            NatPmpError::Malformed(Malformed::Length { got, expected: 12 }).describe(&russian)
        };
        let one = describe(1);
        let few = describe(4);
        let many = describe(16);
        assert_ne!(one, few);
        assert_ne!(few, many);
        for (got, line) in [(1, &one), (4, &few), (16, &many)] {
            assert!(line.contains(&got.to_string()), "{line}");
            assert!(line.contains("12"), "{line}");
        }
        // English has two forms, and the source says so.
        assert!(
            NatPmpError::Malformed(Malformed::Length {
                got: 1,
                expected: 12
            })
            .describe(&english())
            .contains("1 byte long")
        );
    }

    #[test]
    fn short_or_wrong_answers_are_malformed_not_guessed() {
        assert!(matches!(
            decode_map_response(&[0, 0x81], Protocol::Udp),
            Err(NatPmpError::Malformed(_))
        ));
        let mut wrong_opcode = vec![0u8, 0x80 | OPCODE_MAP_TCP];
        wrong_opcode.extend_from_slice(&[0u8; 14]);
        assert!(matches!(
            decode_map_response(&wrong_opcode, Protocol::Udp),
            Err(NatPmpError::Malformed(_))
        ));
    }

    /// A stand-in gateway: real UDP, real RFC bytes, no VPN required.
    fn fake_gateway(
        port: u16,
        response: Vec<u8>,
        expected_requests: usize,
    ) -> thread::JoinHandle<Vec<Vec<u8>>> {
        let socket = UdpSocket::bind(("127.0.0.1", port)).unwrap();
        thread::spawn(move || {
            let mut received = Vec::new();
            let mut buffer = [0u8; 64];
            while received.len() < expected_requests {
                let Ok((n, peer)) = socket.recv_from(&mut buffer) else {
                    break;
                };
                received.push(buffer[..n].to_vec());
                socket.send_to(&response, peer).unwrap();
                if response.len() >= 16 && response[12..16] == [0, 0, 0, 0] {
                    break;
                }
            }
            received
        })
    }

    fn mapped_response(internal: u16, external: u16, lifetime: u32) -> Vec<u8> {
        let mut response = vec![0u8, 0x80 | OPCODE_MAP_UDP, 0, 0];
        response.extend_from_slice(&1u32.to_be_bytes());
        response.extend_from_slice(&internal.to_be_bytes());
        response.extend_from_slice(&external.to_be_bytes());
        response.extend_from_slice(&lifetime.to_be_bytes());
        response
    }

    #[test]
    fn asks_the_gateway_and_returns_the_lease() {
        // Bind to an ephemeral port properly, then hand the address to the client.
        let socket = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        let address = socket.local_addr().unwrap();
        drop(socket);

        let gateway = fake_gateway(address.port(), mapped_response(0, 39949, 60), 1);
        let client = NatPmp::with_gateway(address);
        let mapping = client
            .map(Protocol::Udp, 0, 0, Duration::from_secs(60))
            .unwrap();
        assert_eq!(mapping.external_port, 39949);
        assert_eq!(mapping.lifetime, Duration::from_secs(60));

        let requests = gateway.join().unwrap();
        assert_eq!(
            requests[0],
            encode_map_request(Protocol::Udp, 0, 0, Duration::from_secs(60)).to_vec()
        );
    }

    #[test]
    fn releasing_asks_for_a_zero_lifetime() {
        let socket = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        let address = socket.local_addr().unwrap();
        drop(socket);

        let gateway = fake_gateway(address.port(), mapped_response(39949, 39949, 0), 1);
        let client = NatPmp::with_gateway(address);
        client.release(Protocol::Udp, 39949).unwrap();

        let requests = gateway.join().unwrap();
        assert_eq!(requests[0][8..12], [0, 0, 0, 0], "lifetime must be zero");
        assert_eq!(u16::from_be_bytes([requests[0][4], requests[0][5]]), 39949);
    }

    #[test]
    fn a_silent_gateway_degrades_to_unavailable_instead_of_a_port() {
        // Nothing is listening on this port, so the retry ladder runs out.
        let socket = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        let address = socket.local_addr().unwrap();
        drop(socket);

        let mut client = NatPmp::with_gateway(address);
        client.attempts = 2;
        client.first_timeout = Duration::from_millis(30);
        let error = client.public_address().unwrap_err();
        assert_eq!(error, NatPmpError::Timeout);
        assert!(error.describe(&english()).contains("did not answer"));
    }
}
