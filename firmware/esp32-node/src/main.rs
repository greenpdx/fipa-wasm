//! ESP32 firmware: a sensor agent + the node shim, in one image. Wi-Fi comes up,
//! then the *exact same* `Shim::serve` loop the host demo runs handles the FIPA
//! protocol. The agent is native Rust compiled into the firmware — no wasm engine.

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{BlockingWifi, ClientConfiguration, Configuration, EspWifi};

use node_shim::{Agent, Ctx, Shim};

// Set these at build time, e.g. `WIFI_SSID=... WIFI_PASS=... cargo build --release`.
const SSID: &str = env!("WIFI_SSID");
const PASS: &str = env!("WIFI_PASS");
const PORT: u16 = 9100;

/// A trivial sensor agent: on `obj(read, temp)` it replies with a reading.
struct Sensor {
    reading: i32,
}

impl Agent for Sensor {
    fn on_message(&mut self, unl: &str, _body: &[u8], ctx: &mut Ctx) {
        if unl.contains("read") {
            self.reading += 1;
            let from = ctx.from().to_string();
            ctx.send(from, "obj(reading, temp)", self.reading.to_string().into_bytes());
        }
    }
}

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let sysloop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    let mut wifi =
        BlockingWifi::wrap(EspWifi::new(peripherals.modem, sysloop.clone(), Some(nvs))?, sysloop)?;
    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: SSID.try_into().expect("ssid too long"),
        password: PASS.try_into().expect("pass too long"),
        ..Default::default()
    }))?;
    wifi.start()?;
    wifi.connect()?;
    wifi.wait_netif_up()?;

    let ip = wifi.wifi().sta_netif().get_ip_info()?.ip;
    let addr = format!("{ip}:{PORT}");
    log::info!("node-shim up at {addr}");

    // The same shim loop as the host demo — the device IS the node.
    let listener = std::net::TcpListener::bind(("0.0.0.0", PORT))?;
    let mut shim = Shim::new("sensor-01", &addr);
    let mut agent = Sensor { reading: 20 };
    shim.serve(listener, &mut agent);
    Ok(())
}
