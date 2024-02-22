#[macro_use]
extern crate log;

use std::str::FromStr;
use std::thread;
use std::time::Duration;
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::task::block_on;
use esp_idf_hal::timer::{TimerConfig, TimerDriver};
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::handle::RawHandle;
use esp_idf_svc::http::{Method, server};
use esp_idf_svc::http::server::EspHttpServer;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::timer::EspTaskTimerService;
use esp_idf_svc::wifi;
use esp_idf_svc::wifi::{AccessPointConfiguration, AsyncWifi, AuthMethod, EspWifi, Protocol};
use esp_idf_svc::io::Write;

fn main() {
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_sys::link_patches();

    esp_idf_svc::log::EspLogger::initialize_default();

    thread::Builder::new().stack_size(8192).spawn(move || {
        block_on(async move {
            let event_loop = EspSystemEventLoop::take().expect("No system event loop!");
            let timer_service = EspTaskTimerService::new().expect("Couldn't create timer service");

            info!("Hello, world!");

            let p = Peripherals::take().expect("No Peripherals!");
            let modem = p.modem;

            let mut timer = TimerDriver::new(p.timer00, &TimerConfig::new()).expect("Error creating timer");

            info!("Creating Wifi...");

            let nvs = EspDefaultNvsPartition::take().expect("Error getting NVS partition");
            let mut esp_wifi = EspWifi::new(modem, event_loop.clone(), Some(nvs)).expect("Can't create wifi");
            let mut wifi = AsyncWifi::wrap(&mut esp_wifi, event_loop.clone(), timer_service.clone()).expect("Error building async wifi");

            info!("Configuring Wifi...");

            wifi.set_configuration(&wifi::Configuration::AccessPoint(AccessPointConfiguration {
                ssid: heapless::String::from_str("WiFiCurse").expect("SSID too long"),
                ssid_hidden: false,
                channel: 9,
                secondary_channel: None,
                protocols: enumset::enum_set!(Protocol::P802D11BGNLR),
                auth_method: AuthMethod::None,
                password: heapless::String::new(),
                max_connections: 5,
            })).expect("Couldn't set config");

            info!("Starting WiFi...");

            wifi.start().await.expect("Error starting wifi");

            info!("WiFi started.");

            let mut http = EspHttpServer::new(&server::Configuration {
                http_port: 80,
                https_port: 443,
                max_sessions: 5,
                session_timeout: Duration::from_secs(10),
                stack_size: 4096,
                max_open_sockets: 2,
                max_uri_handlers: 2,
                max_resp_headers: 2,
                lru_purge_enable: false,
                uri_match_wildcard: false,
            }).expect("Error creating http server");

            http.fn_handler("/", Method::Get, |request| {
                info!("Received connection to '/'");

                let mut response = request.into_ok_response()?;

                write!(&mut response, "<h1>It Works!</h1>")?;

                Ok::<_, anyhow::Error>(())
            }).expect("Error setting up '/' handler");

            loop {
                timer.delay(timer.tick_hz()).await.expect("Error waiting");
            }
        });
    }).expect("Error starting thread");
}
