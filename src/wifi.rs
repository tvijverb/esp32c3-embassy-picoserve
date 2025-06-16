use embassy_executor::Spawner;
use embassy_net::{DhcpConfig, Runner, Stack, StackResources};
use embassy_time::{Duration, Timer};
use esp_hal::rng::Rng;
use rtt_target::rprintln;
use esp_wifi::wifi::{self, WifiController, WifiDevice, WifiEvent, WifiState};
use esp_wifi::EspWifiController;

use crate::mk_static;

const SSID: &str = "myneighboursaresohot";
const PASSWORD: &str = "p@nnenkoek";

pub async fn start_wifi(
    esp_wifi_ctrl: &'static EspWifiController<'static>,
    wifi: esp_hal::peripherals::WIFI<'static>,
    mut rng: Rng,
    spawner: &Spawner,
) -> Stack<'static> {
    let (controller, interfaces) = esp_wifi::wifi::new(&esp_wifi_ctrl, wifi).unwrap();
    let wifi_interface = interfaces.sta;
    let net_seed = rng.random() as u64 | ((rng.random() as u64) << 32);

    let dhcp_config = DhcpConfig::default();
    let net_config = embassy_net::Config::dhcpv4(dhcp_config);

    // Init network stack
    let (stack, runner) = embassy_net::new(
        wifi_interface,
        net_config,
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        net_seed,
    );

    spawner.spawn(connection_task(controller)).ok();
    spawner.spawn(net_task(runner)).ok();

    wait_for_connection(stack).await;

    stack
}


async fn wait_for_connection(stack: Stack<'_>) {
    rprintln!("Waiting for link to be up");
    loop {
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    rprintln!("Waiting to get IP address...");
    loop {
        if let Some(config) = stack.config_v4() {
            rprintln!("Got IP: {}", config.address);
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }
}

#[embassy_executor::task]
async fn connection_task(mut controller: WifiController<'static>) {
    rprintln!("start connection task");
    rprintln!("Device capabilities: {:?}", controller.capabilities());
    loop {
        match esp_wifi::wifi::wifi_state() {
            WifiState::StaConnected => {
                // wait until we're no longer connected
                controller.wait_for_event(WifiEvent::StaDisconnected).await;
                Timer::after(Duration::from_millis(5000)).await
            }
            _ => {}
        }
        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = wifi::Configuration::Client(wifi::ClientConfiguration {
                ssid: SSID.try_into().unwrap(),
                password: PASSWORD.try_into().unwrap(),
                ..Default::default()
            });
            controller.set_configuration(&client_config).unwrap();
            rprintln!("Starting wifi");
            controller.start_async().await.unwrap();
            rprintln!("Wifi started!");
        }
        rprintln!("About to connect...");

        match controller.connect_async().await {
            Ok(_) => rprintln!("Wifi connected!"),
            Err(e) => {
                rprintln!("Failed to connect to wifi: {:?}", e);
                Timer::after(Duration::from_millis(5000)).await
            }
        }
    }
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    runner.run().await
}