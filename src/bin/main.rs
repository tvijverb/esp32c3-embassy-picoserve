#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

use embassy_executor::Spawner;
use embassy_net::Stack;
use embassy_time::{Duration, Timer};
use esp32c3_embassy_picoserve::clock::Clock;
use esp32c3_embassy_picoserve::http::Client;
use esp32c3_embassy_picoserve::random::RngWrapper;
use esp_hal::clock::CpuClock;
use esp_hal::rng::Rng;
use esp_hal::rtc_cntl::Rtc;
use esp_hal::timer::systimer::SystemTimer;
use esp_hal::timer::timg::TimerGroup;
use rtt_target::rprintln;

use esp_wifi::EspWifiController;

use esp32c3_embassy_picoserve as lib;
use time::UtcOffset;


#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

extern crate alloc;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[esp_hal_embassy::main]
async fn main(spawner: Spawner) {
    // generator version: 0.4.0

    rtt_target::rtt_init_print!();
    rprintln!("Starting esp32c3_embassy_picoserve...");

    // Load environment variables from .env file.
    // Fails if .env file not found, not readable or invalid.

    // let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::_80MHz);
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timer0 = SystemTimer::new(peripherals.SYSTIMER);
    esp_hal_embassy::init(timer0.alarm0);

    rprintln!("Embassy initialized!");

    let rng = esp_hal::rng::Rng::new(peripherals.RNG);
    let timer1 = TimerGroup::new(peripherals.TIMG0);
    let _rtc = Rtc::new(peripherals.LPWR);

    // let wifi_init = esp_wifi::init(timer1.timer0, rng, peripherals.RADIO_CLK)
    //     .expect("Failed to initialize WIFI/BLE controller");
    // let (mut _wifi_controller, _interfaces) = esp_wifi::wifi::new(&wifi_init, peripherals.WIFI)
    //     .expect("Failed to initialize WIFI controller");

    // TODO: Spawn some tasks
    let _ = spawner;

    // Spawn a task to print "Hello world!" every second
    spawner.spawn(print_hello_world()).unwrap();

    // NEW code
    let esp_wifi_ctrl = &*lib::mk_static!(
        EspWifiController<'static>,
        esp_wifi::init(timer1.timer0, rng.clone(), peripherals.RADIO_CLK).unwrap()
    );

    let stack = lib::wifi::start_wifi(esp_wifi_ctrl, peripherals.WIFI, rng, &spawner).await;

    rprintln!("Starting RTC...");

    let clock = load_clock(
        spawner,
        stack,
        rng,
    )
    .await;

    rprintln!("Now is {}", clock.now().unwrap());

    // let web_app = lib::web::WebApp::default(clock.clone());
    let web_app = lib::web::WebApp::new_with_clock(clock.clone());

    for id in 0..lib::web::WEB_TASK_POOL_SIZE {
        spawner.must_spawn(lib::web::web_task(
            id,
            stack,
            web_app.router,
            web_app.config,
            web_app.state,
        ));
    }
    rprintln!("Web server started...");

    // loop {
    //     rprintln!("Hello world!");
    //     Timer::after(Duration::from_secs(1)).await;
    // }

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.0.0-beta.1/examples/src/bin
}

#[embassy_executor::task]
async fn print_hello_world() {
    loop {
        rprintln!("Hello world from embassy using esp-hal-async!");
        Timer::after(Duration::from_millis(30_000)).await;
    }
}

// #[embassy_executor::task]
// async fn rtc_set_current_date(mut lpwr: LPWR, current_time_us: u64) {
//     let mut rtc = Rtc::new(&mut lpwr);
//     rtc.set_current_time_us(current_time_us);
// }

/// Load clock from RTC memory of from server
async fn load_clock(
    _spawner: Spawner,
    stack: Stack<'static>,
    rng: Rng,
) -> Clock {
    let clock = if let Some(clock) = Clock::from_rtc_memory() {
        rprintln!("Clock loaded from RTC memory");
        clock
    } else {
        rprintln!("Synchronize clock from server");
        let mut http_client = Client::new(stack, RngWrapper::from(rng));
        let clock = Clock::from_server(&mut http_client).await;

        if let Err(e) = clock {
            rprintln!("Failed to synchronize clock: {:?}", e);
            // Fallback to a default clock
            return Clock::new(0, UtcOffset::UTC);
        } else {
            rprintln!("Clock synchronized from server");
            return clock.unwrap();
        }
    };

    clock
}