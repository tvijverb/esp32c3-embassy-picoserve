use embassy_net::Stack;
use embassy_time::{Duration, Instant};
use esp_alloc as _;
use jiff::tz::{self, TimeZone};
use picoserve::{io::Read, make_static, request::Path, response::ResponseWriter, routing, AppBuilder, AppRouter, Router};
use rtt_target::rprintln;
use core::{fmt::Write, sync::atomic::AtomicUsize};
use heapless::String;

pub const WEB_TASK_POOL_SIZE: usize = 2;
static TZ: TimeZone = tz::get!("Europe/Amsterdam");

pub struct Application;

impl AppBuilder for Application {
    type PathRouter = impl routing::PathRouter;

    // fn build_app(self) -> picoserve::Router<Self::PathRouter> {
    //     picoserve::Router::new().route(
    //         "/",
    //         routing::get_service(File::html(include_str!("index.html"))),
    //     )
    // }

    fn build_app(self) -> picoserve::Router<Self::PathRouter> {
        picoserve::Router::new()
            .route("/", routing::get(|| async move { "Hello World" }))
            .route("/version", routing::get(|| async move {
                let mut version_string = String::<64>::new();
                write!(version_string, "Version: {}", env!("CARGO_PKG_VERSION")).unwrap();
                version_string
            }))
            .layer(TimeLayer)
    }
}

pub struct WebApp {
    pub router: &'static Router<<Application as AppBuilder>::PathRouter>,
    pub config: &'static picoserve::Config<Duration>,
}

impl Default for WebApp {
    fn default() -> Self {
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

        Self { router, config }
    }
}


#[embassy_executor::task(pool_size = WEB_TASK_POOL_SIZE)]
pub async fn web_task(
    id: usize,
    stack: Stack<'static>,
    router: &'static AppRouter<Application>,
    config: &'static picoserve::Config<Duration>,
) -> ! {
    let port = 80;
    let mut tcp_rx_buffer = [0; 1024];
    let mut tcp_tx_buffer = [0; 1024];
    let mut http_buffer = [0; 2048];

    picoserve::listen_and_serve(
        id,
        router,
        config,
        stack,
        port,
        &mut tcp_rx_buffer,
        &mut tcp_tx_buffer,
        &mut http_buffer,
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

impl<State, PathParameters> picoserve::routing::Layer<State, PathParameters> for TimeLayer {
    type NextState = State;
    type NextPathParameters = PathParameters;

    async fn call_layer<
        'a,
        R: Read + 'a,
        NextLayer: picoserve::routing::Next<'a, R, Self::NextState, Self::NextPathParameters>,
        W: ResponseWriter<Error = R::Error>,
    >(
        &self,
        next: NextLayer,
        state: &State,
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