# esp32-node — a FIPA node + agent in one ESP32 firmware

The agent is native Rust compiled into the image; [`node-shim`](../../crates/node-shim)
wraps it with the FIPA protocols (signed envelope + Noise transport + the run loop).
No wasm engine, no libp2p/tonic/sled — just the agent and the wire.

## Why one binary

A leaf device doesn't host a wasm tenant — it *is* the agent. So node and agent
collapse into a single firmware: `Shim::serve(listener, &mut agent)` is the whole
node. The same `Agent` impl runs unchanged on a hosted node; only `main` differs
(Wi-Fi bring-up here vs `TcpListener::bind` on the host demo).

## Build (requires the esp toolchain)

```sh
# one-time: install the ESP Rust toolchain + flashing tools
cargo install espup ldproxy espflash
espup install            # installs the esp/riscv toolchain + IDF prerequisites
. ~/export-esp.sh        # sets ESP_IDF + PATH (Xtensa only; riscv uses stock nightly)

# build the size-optimized image (ESP32-C3 / RISC-V is the default target)
cd firmware/esp32-node
WIFI_SSID="myssid" WIFI_PASS="mypass" cargo build --release

# flash + monitor
WIFI_SSID="myssid" WIFI_PASS="mypass" cargo run --release
```

For classic ESP32 / ESP32-S3 (Xtensa): set `target` in `.cargo/config.toml` to
`xtensa-esp32-espidf` (or `…s3…`), set `MCU=esp32`/`esp32s3`, and switch
`rust-toolchain.toml` to `channel = "esp"`.

## Measure the footprint

```sh
# final flash image size (must fit the 3 MB app slot in partitions.csv)
espflash save-image --chip esp32c3 --merge target/riscv32imc-esp-espidf/release/esp32-node image.bin
ls -l image.bin

# section breakdown (.text/.rodata = flash; .data/.bss = RAM)
cargo size --release -- -A

# runtime RAM headroom is reported at boot by esp-idf; watch the monitor for
# "Free heap" after Wi-Fi connects — that is the real 400 KB budget check.
```

## RAM budget note

The app + shim use only a few KB of static RAM (the host proxy measures ~550 bytes
`.bss`). The 400 KB SRAM target is dominated by the esp-idf baseline — Wi-Fi buffers
+ lwIP + FreeRTOS. The knobs that move that number live in `sdkconfig.defaults`
(Wi-Fi RX/TX buffer counts, lwIP window/socket sizes), not in our code.

## Known size optimization

`snow` 0.9's `default-resolver` bundles every cipher (AES-GCM, ChaChaPoly, SHA-2,
BLAKE2) even though our suite is only `Noise_XX_25519_ChaChaPoly_BLAKE2s`. The
AES-GCM code is dead weight in flash. Trimming it needs a leaner Noise resolver
(e.g. `ring-resolver`, or a patched/minimal Noise) — tracked, not yet done.
