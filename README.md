# r16

A Rust project for the [Pimoroni Pico RGB Keypad Base](https://shop.pimoroni.com/products/pico-rgb-keypad-base)
(Raspberry Pi Pico / RP2040 + 4×4 button grid + APA102 LEDs), intended as
a platform for a simple game.

The starter example lights every key dim blue, and switches a key to
bright orange while it is held down. The Pico's onboard LED blinks once
a second as a heartbeat — useful liveness signal when debugging.

## Hardware

The keypad base wires the RP2040 as follows:

| Peripheral | Pin | Function |
|---|---|---|
| SPI0 SCK | GP18 | APA102 clock |
| SPI0 TX  | GP19 | APA102 data |
| GPIO     | GP17 | APA102 chip-select (board-level gate) |
| I2C0 SDA | GP4  | TCA9555 data |
| I2C0 SCL | GP5  | TCA9555 clock |

The 16 buttons hang off a TCA9555 I²C I/O expander at address `0x20`,
and the 16 LEDs are a single APA102 chain in row-major order with key 0
at the end of the board nearest the Pico's USB port (top-left with USB
up), reading left-to-right, top-to-bottom, ending at key 15.

`src/main.rs` also reserves **GP16** as a dummy MISO so that
`rp2040-hal`'s SPI pin tuple is satisfied — nothing is wired to GP16 on
the keypad base, but don't re-use it for another peripheral without
rethinking the SPI setup.

## Toolchain setup (Linux)

Install the stable Rust toolchain (e.g. via [rustup](https://rustup.rs))
and add the Cortex-M0+ target:

```bash
rustup target add thumbv6m-none-eabi
```

## Flashing with `picotool` (no debug probe required)

This project's `cargo run` is wired to invoke `picotool load` on the
built ELF. `picotool` talks to the RP2040's built-in USB BOOTSEL
bootloader — you just need the Pico in BOOTSEL mode (hold the `BOOTSEL`
button on the Pico while plugging in USB).

### 1. Install `picotool` from the official prebuilt binary

Pre-built binaries live in the Raspberry Pi
[`pico-sdk-tools` releases page](https://github.com/raspberrypi/pico-sdk-tools/releases)
(not in the `picotool` repo itself, which only publishes source).

Open the latest release and download the asset matching your CPU. The
filename follows the pattern `picotool-<version>-<arch>-lin.tar.gz`, e.g.:

- `picotool-2.2.0-a4-x86_64-lin.tar.gz` — Intel/AMD 64-bit
- `picotool-2.2.0-a4-aarch64-lin.tar.gz` — ARM 64-bit (e.g. Raspberry Pi 5)

Then extract it and drop the binary on your `PATH`:

```bash
mkdir -p ~/.local/bin ~/.local/share/picotool
tar -xzf ~/Downloads/picotool-*-lin.tar.gz -C ~/.local/share/picotool
ln -sf ~/.local/share/picotool/picotool/picotool ~/.local/bin/picotool

# Ensure ~/.local/bin is on your PATH:
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc
source ~/.bashrc

picotool version     # sanity check
```

### 2. Install the udev rule so you don't need `sudo`

Grab the rule file from the `picotool` source tree and install it:

```bash
sudo curl -o /etc/udev/rules.d/99-picotool.rules \
  https://raw.githubusercontent.com/raspberrypi/picotool/master/udev/99-picotool.rules
sudo udevadm control --reload-rules
sudo udevadm trigger
```

Log out and back in once so your user picks up the `plugdev` group (or
whichever group the rule file targets — the file is commented; read it
before accepting).

### 3. Put the Pico in BOOTSEL mode and flash

1. Unplug the USB cable.
2. Hold down the `BOOTSEL` button on the Pico.
3. Plug the USB cable back in, then release `BOOTSEL`.
4. The Pico appears as a mass-storage device (`RPI-RP2`).

Then from this project's directory:

```bash
cargo run --release
```

Cargo compiles for `thumbv6m-none-eabi`, and the runner configured in
`.cargo/config.toml` executes:

```
picotool load -u -v -x -t elf target/thumbv6m-none-eabi/release/r16
```

- `-u` wipes the target flash region first
- `-v` verifies the write
- `-x` executes immediately after loading
- `-t elf` declares the input as an ELF

Press keys; they should light orange while held.

### What you should see

- **Onboard LED** (green, next to the USB socket): blinks on/off once
  per second. **Blinking = firmware running.**
- **Keypad**: all 16 keys glow dim blue at idle. Pressing a key makes
  it glow bright orange until released.

If the onboard LED never blinks, the firmware isn't running — recheck
the BOOTSEL cycle and the `picotool load` output. If the LED blinks but
the keypad is dark or unresponsive, the base likely isn't fully seated
on the Pico's header pins.

### Reflashing after the first boot

Our firmware does not wire up the USB reset interface, so `picotool`
cannot kick the board into BOOTSEL on its own. Every reflash repeats
step 3 above: unplug, hold `BOOTSEL`, plug in, `cargo run --release`.

### Flashing manually (equivalent of the runner)

```bash
cargo build --release
picotool load -u -v -x -t elf target/thumbv6m-none-eabi/release/r16
```

### Alternative: `elf2uf2-rs`

If you'd rather not install `picotool`, `elf2uf2-rs` is a pure-Rust
alternative that writes a UF2 to the mounted BOOTSEL volume:

```bash
cargo install elf2uf2-rs
```

Then swap the runner in `.cargo/config.toml` to:

```toml
runner = "elf2uf2-rs -d"
```

## Project layout

```
Cargo.toml          # deps and build profile
memory.x            # RP2040 linker regions (BOOT2 + FLASH + RAM)
build.rs            # copies memory.x into OUT_DIR for the linker
.cargo/config.toml  # target + runner + linker flags
src/main.rs         # entry point: poll buttons, update LEDs
```

The `no_std` entry point lives in `src/main.rs`. SPI and I²C are driven
directly through `embedded-hal 1.0` traits; no APA102 or TCA9555
driver crates are pulled in, since both protocols are a handful of
bytes.
