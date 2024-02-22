#[macro_use]
extern crate log;

use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use anyhow::Context;
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::rmt::{PinState, Pulse, TxRmtDriver, VariableLengthSignal};
use esp_idf_hal::rmt::config::TransmitConfig;
use esp_idf_hal::task::block_on;
use esp_idf_hal::timer::{TimerConfig, TimerDriver};
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::http::{Method, server};
use esp_idf_svc::http::server::EspHttpServer;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::timer::EspTaskTimerService;
use esp_idf_svc::wifi;
use esp_idf_svc::wifi::{AccessPointConfiguration, AsyncWifi, AuthMethod, EspWifi, Protocol};
use esp_idf_svc::io::{Read, Write};

const HTTP_RES: &'static str = r#"
<!DOCTYPE html>
<html>
<body>
<h1>It Works!</h1>

<label for="red">Red:</label>
<input id="red" type="text"><br>
<label for="green">Green:</label>
<input id="green" type="text"><br>
<label for="blue">Blue:</label>
<input id="blue" type="text"><br>
<input id="submit" type="button" value="Submit">

<script>
document.getElementById("submit").addEventListener("click", function() {
    const req = new XMLHttpRequest();
    req.open("POST", "/led", true);

    const body = new Uint8Array(3);
    const redElem = document.getElementById("red");
    const greenElem = document.getElementById("green");
    const blueElem = document.getElementById("blue");

    body[0] = parseInt(redElem.value);
    body[1] = parseInt(greenElem.value);
    body[2] = parseInt(blueElem.value);

    const blob = new Blob([body], {type: "application/octet-stream"});
    req.send(blob);
    console.log("Sent: " + body[0] + ", " + body[1] + ", " + body[2]);
    console.log("Blob: " + blob);
});
</script>
</body>
</html>
"#;

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

            info!("Loading timers...");

            let pin = p.pins.gpio8;
            let channel = p.rmt.channel0;
            let config = TransmitConfig::new().clock_divider(1);
            let tx = Arc::new(Mutex::new(TxRmtDriver::new(channel, pin, &config).expect("Error getting remote util")));

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

                write!(&mut response, "{}", HTTP_RES)?;

                Ok::<_, anyhow::Error>(())
            }).expect("Error setting up '/' handler");

            let tx_clone = tx.clone();
            http.fn_handler("/led", Method::Post, move |mut request| {
                let mut buf = [0u8; 3];
                request.read_exact(&mut buf).context("Reading HTTP buffer")?;

                let rgb = RGB {
                    r: buf[0],
                    g: buf[1],
                    b: buf[2],
                };

                info!("Received /led: {:?}", &rgb);

                let mut tx = tx_clone.lock().expect("Error locking NeoPixel");
                neopixel(&[rgb], &mut tx).context("Outputting to NeoPixel")?;

                let mut response = request.into_ok_response()?;
                response.write(&buf)?;

                Ok::<_, anyhow::Error>(())
            }).expect("Erroring setting up '/led' handler");

            loop {
                timer.delay(timer.tick_hz()).await.expect("Error waiting");
            }
        });
    }).expect("Error starting thread");
}

// copied from neopixel example

#[derive(Debug)]
struct RGB {
    r: u8,
    g: u8,
    b: u8,
}

fn ns(nanos: u64) -> Duration {
    Duration::from_nanos(nanos)
}

fn neopixel(rgb_list: &[RGB], tx: &mut TxRmtDriver) -> anyhow::Result<()> {
    let ticks_hz = tx.counter_clock()?;
    let t0h = Pulse::new_with_duration(ticks_hz, PinState::High, &ns(350))?;
    let t0l = Pulse::new_with_duration(ticks_hz, PinState::Low, &ns(800))?;
    let t1h = Pulse::new_with_duration(ticks_hz, PinState::High, &ns(700))?;
    let t1l = Pulse::new_with_duration(ticks_hz, PinState::Low, &ns(600))?;
    // 2 pulses per bit, 8 bits per byte, 3 bytes per color
    let mut signal = VariableLengthSignal::with_capacity(48 * rgb_list.len());
    for rgb in rgb_list.iter() {
        // e.g. rgb: (1,2,4)
        // G        R        B
        // 7      0 7      0 7      0
        // 00000010 00000001 00000100
        let color: u32 = ((rgb.g as u32) << 16) | ((rgb.r as u32) << 8) | rgb.b as u32;
        for i in (0..24).rev() {
            let p = 2_u32.pow(i);
            let bit = p & color != 0;
            let (high_pulse, low_pulse) = if bit { (t1h, t1l) } else { (t0h, t0l) };
            signal.push(&[high_pulse, low_pulse])?;
        }
    }
    tx.start_blocking(&signal)?;

    Ok(())
}
