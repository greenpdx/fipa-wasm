//! A whole device in one binary: a sensor agent + the node shim. This is the
//! host-buildable proxy for the ESP32 firmware — the same `Agent` and `Shim` link
//! here as on-device; only `main` (Wi-Fi bring-up vs `TcpListener::bind`) differs.

use node_shim::{Agent, Ctx, Shim};

/// A trivial sensor: on `obj(read, temp)` it replies with a (fake) reading.
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

fn main() {
    // On ESP32 this is replaced by Wi-Fi bring-up + the assigned IP; the shim loop
    // below is identical.
    let listener = std::net::TcpListener::bind("0.0.0.0:9100").expect("bind");
    let mut shim = Shim::new("sensor-01", "0.0.0.0:9100");
    let mut agent = Sensor { reading: 20 };
    shim.serve(listener, &mut agent);
}
