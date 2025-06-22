use embassy_net::Stack;
use embassy_time::{Duration, Instant};
use esp_alloc as _;
use picoserve::{io::Read, request::Path, response::ResponseWriter, routing, AppRouter, Router, AppWithStateBuilder};
use rtt_target::rprintln;
use core::fmt::Write;
use heapless::String;
use time;

use crate::clock::Clock;

pub const WEB_TASK_POOL_SIZE: usize = 1;

/// The state used by the web app, containing the clock
pub struct AppState {
    pub clock: Clock,
}

/// An extractor for getting the clock from the app state
pub struct ClockExtractor(pub Clock);

impl<'r> picoserve::extract::FromRequestParts<'r, AppState> for ClockExtractor {
    type Rejection = core::convert::Infallible;

    async fn from_request_parts(
        state: &'r AppState,
        _request_parts: &picoserve::request::RequestParts<'r>,
    ) -> Result<Self, Self::Rejection> {
        Ok(Self(state.clock.clone()))
    }
}

pub struct Application;

impl AppWithStateBuilder for Application {
    type State = AppState;
    type PathRouter = impl routing::PathRouter<AppState>;

    fn build_app(self) -> picoserve::Router<Self::PathRouter, AppState> {
        picoserve::Router::new()
            .route("/", routing::get(|| async move { "Hello World" }))
            .route("/version", routing::get(|| async move {
                let mut version_string = String::<64>::new();
                write!(version_string, "Version: {}", env!("CARGO_PKG_VERSION")).unwrap();
                version_string
            }))
            .route("/time", routing::get(|ClockExtractor(clock)| async move {
                match clock.now() {
                    Ok(time) => {
                        let mut time_string = String::<128>::new();
                        write!(time_string, "Current time: {}", time).unwrap();
                        time_string
                    }
                    Err(_) => {
                        let mut error_string = String::<128>::new();
                        write!(error_string, "Error getting current time").unwrap();
                        error_string
                    }
                }
            }))
            .layer(TimeLayer)
    }
}

pub struct WebApp {
    pub router: &'static Router<<Application as AppWithStateBuilder>::PathRouter, AppState>,
    pub config: &'static picoserve::Config<Duration>,
    pub state: &'static AppState,
}

impl Default for WebApp {
    fn default() -> Self {
        // Create a default clock for the default implementation
        let default_clock = Clock::new(0, time::UtcOffset::UTC);
        Self::new_with_clock(default_clock)
    }
}

impl WebApp {
    pub fn new_with_clock(clock: Clock) -> Self {
        let router = picoserve::make_static!(AppRouter<Application>, Application.build_app());

        let config = picoserve::make_static!(
            picoserve::Config<Duration>,
            picoserve::Config::new(picoserve::Timeouts {
                start_read_request: Some(Duration::from_secs(5)),
                persistent_start_read_request: Some(Duration::from_secs(1)),
                read_request: Some(Duration::from_secs(1)),
                write: Some(Duration::from_secs(1)),
            })
            .keep_connection_alive()
        );

        let state = picoserve::make_static!(
            AppState,
            AppState { clock }
        );

        Self { router, config, state }
    }
}


#[embassy_executor::task(pool_size = WEB_TASK_POOL_SIZE)]
pub async fn web_task(
    id: usize,
    stack: Stack<'static>,
    router: &'static AppRouter<Application>,
    config: &'static picoserve::Config<Duration>,
    state: &'static AppState,
) -> ! {
    let port = 80;
    let mut tcp_rx_buffer = [0; 1024];
    let mut tcp_tx_buffer = [0; 1024];
    let mut http_buffer = [0; 2048];

    picoserve::listen_and_serve_with_state(
        id,
        router,
        config,
        stack,
        port,
        &mut tcp_rx_buffer,
        &mut tcp_tx_buffer,
        &mut http_buffer,
        state,
    )
    .await
}

struct TimedResponseWriter<'r, W> {
    path: Path<'r>,
    start_time: Instant,
    response_writer: W,
}

impl<'r, W: ResponseWriter> ResponseWriter for TimedResponseWriter<'r, W> {
    type Error = W::Error;

    async fn write_response<
        R: Read<Error = Self::Error>,
        H: picoserve::response::HeadersIter,
        B: picoserve::response::Body,
    >(
        self,
        connection: picoserve::response::Connection<'_, R>,
        response: picoserve::response::Response<H, B>,
    ) -> Result<picoserve::ResponseSent, Self::Error> {
        let status_code = response.status_code();

        let result = self
            .response_writer
            .write_response(connection, response)
            .await;

        rprintln!(
            "Path: {}; Status Code: {}; Response Time: {}ms",
            self.path,
            status_code,
            self.start_time.elapsed().as_millis()
        );

        result
    }
}

struct TimeLayer;

impl<PathParameters> picoserve::routing::Layer<AppState, PathParameters> for TimeLayer {
    type NextState = AppState;
    type NextPathParameters = PathParameters;

    async fn call_layer<
        'a,
        R: Read + 'a,
        NextLayer: picoserve::routing::Next<'a, R, Self::NextState, Self::NextPathParameters>,
        W: ResponseWriter<Error = R::Error>,
    >(
        &self,
        next: NextLayer,
        state: &AppState,
        path_parameters: PathParameters,
        request_parts: picoserve::request::RequestParts<'_>,
        response_writer: W,
    ) -> Result<picoserve::ResponseSent, W::Error> {
        let path = request_parts.path();

        next.run(
            state,
            path_parameters,
            TimedResponseWriter {
                path,
                start_time: Instant::now(),
                response_writer,
            },
        )
        .await
    }
}