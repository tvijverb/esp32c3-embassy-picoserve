// Copyright Claudio Mattera 2024-2025.
//
// Distributed under the MIT License or the Apache 2.0 License at your option.
// See the accompanying files LICENSE-MIT.txt and LICENSE-APACHE-2.0.txt, or
// online at
// https://opensource.org/licenses/MIT
// https://opensource.org/licenses/Apache-2.0

//! HTTP client

use core::str::from_utf8;
use core::num::ParseIntError;

use embassy_net::dns::DnsSocket;
use embassy_net::dns::Error as DnsError;
use embassy_net::tcp::client::TcpClient;
use embassy_net::tcp::client::TcpClientState;
use embassy_net::tcp::ConnectError as TcpConnectError;
use embassy_net::tcp::Error as TcpError;
use embassy_net::Stack;

use reqwless::client::HttpClient;
use reqwless::client::TlsConfig;
use reqwless::client::TlsVerify;
use reqwless::request::Method;
use reqwless::Error as ReqlessError;

use heapless::Vec;

use rand_core::RngCore as _;
use rtt_target::rprintln;
use time::error::Parse;
use time::OffsetDateTime;

use crate::random::RngWrapper;

/// Response size
const RESPONSE_SIZE: usize = 4096;

/// HTTP client
///
/// This trait exists to be extended with requests to specific sites, like in
/// [`WorldTimeApiClient`][crate::worldtimeapi::WorldTimeApiClient].
pub trait ClientTrait {
    /// Send an HTTP request
    async fn send_request(&mut self, url: &str) -> Result<Vec<u8, RESPONSE_SIZE>, Error>;
}

/// HTTP client
pub struct Client {
    /// Wifi stack
    stack: Stack<'static>,

    /// Random numbers generator
    rng: RngWrapper,

    /// TCP client state
    tcp_client_state: TcpClientState<1, 4096, 4096>,

    /// Buffer for received TLS data
    read_record_buffer: [u8; 16640],

    /// Buffer for transmitted TLS data
    write_record_buffer: [u8; 16640],
}

impl Client {
    /// Create a new client
    pub fn new(stack: Stack<'static>, rng: RngWrapper) -> Self {
        rprintln!("Create TCP client state");
        let tcp_client_state = TcpClientState::<1, 4096, 4096>::new();

        Self {
            stack,
            rng,

            tcp_client_state,

            read_record_buffer: [0_u8; 16640],
            write_record_buffer: [0_u8; 16640],
        }
    }

    pub async fn fetch_current_time(&mut self) -> Result<OffsetDateTime, Error> {
        let url = "https://io.adafruit.com/api/v2/time/seconds";

        let response = self.send_request(url).await?;

        let text_result = from_utf8(&response);
        let text = match text_result {
            Ok(text) => text,
            Err(e) => return Err(Error::Utf8Error(e)),
        };
        let timestamp_result = text.parse::<i64>(); 
        let timestamp = match timestamp_result {
            Ok(ts) => ts,
            Err(e) => return Err(Error::ParseIntError(e)),
        };
        let utc_result = OffsetDateTime::from_unix_timestamp(timestamp);
        let utc = utc_result.unwrap(); // We assume the timestamp is valid
        rprintln!("Current UTC time: {}", utc);
        Ok(utc)
    }
}

impl ClientTrait for Client {
    async fn send_request(&mut self, url: &str) -> Result<Vec<u8, RESPONSE_SIZE>, Error> {
        rprintln!("Send HTTPs request to {}", url);

        rprintln!("Create DNS socket");
        let dns_socket = DnsSocket::new(self.stack);

        let seed = self.rng.next_u64();
        let tls_config = TlsConfig::new(
            seed,
            &mut self.read_record_buffer,
            &mut self.write_record_buffer,
            TlsVerify::None,
        );

        rprintln!("Create TCP client");
        let tcp_client = TcpClient::new(self.stack, &self.tcp_client_state);

        rprintln!("Create HTTP client");
        let mut client = HttpClient::new_with_tls(&tcp_client, &dns_socket, tls_config);

        rprintln!("Create HTTP request");
        let mut buffer = [0_u8; 4096];
        let mut request = client.request(Method::GET, url).await?;

        rprintln!("Send HTTP request");
        let response = request.send(&mut buffer).await?;

        rprintln!("Response status: {:?}", response.status);

        let buffer = response.body().read_to_end().await?;

        rprintln!("Read {} bytes", buffer.len());

        let output =
            Vec::<u8, RESPONSE_SIZE>::from_slice(buffer).map_err(|()| Error::ResponseTooLarge)?;

        Ok(output)
    }
}

/// An error within an HTTP request
#[derive(Debug)]
pub enum Error {
    /// Response was too large
    ResponseTooLarge,

    /// Error within TCP streams
    Tcp(TcpError),

    /// Error within TCP connection
    TcpConnect(#[expect(unused, reason = "Never read directly")] TcpConnectError),

    /// Error within DNS system
    Dns(#[expect(unused, reason = "Never read directly")] DnsError),

    /// Error in HTTP client
    Reqless(#[expect(unused, reason = "Never read directly")] ReqlessError),

    /// Error parsing UTF-8
    Utf8Error(#[expect(unused, reason = "Never read directly")] core::str::Utf8Error),

    /// Error parsing a timestamp
    ParseIntError(#[expect(unused, reason = "Never read directly")] ParseIntError),
}

impl From<TcpError> for Error {
    fn from(error: TcpError) -> Self {
        Self::Tcp(error)
    }
}

impl From<TcpConnectError> for Error {
    fn from(error: TcpConnectError) -> Self {
        Self::TcpConnect(error)
    }
}

impl From<DnsError> for Error {
    fn from(error: DnsError) -> Self {
        Self::Dns(error)
    }
}

impl From<ReqlessError> for Error {
    fn from(error: ReqlessError) -> Self {
        Self::Reqless(error)
    }
}