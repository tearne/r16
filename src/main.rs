#![no_std]
#![no_main]

use panic_halt as _;

use rp_pico::entry;
use rp_pico::hal::{self, pac, Clock};

use embedded_hal::digital::{OutputPin, StatefulOutputPin};
use embedded_hal::i2c::I2c;
use embedded_hal::spi::SpiBus;

use fugit::RateExtU32;

// --- Pimoroni Pico RGB Keypad Base wiring ---
// APA102 LED chain: SCK = GP18, DATA = GP19, CS = GP17 (gated on the board)
// TCA9555 I/O expander (buttons): SDA = GP4, SCL = GP5, I2C addr 0x20

const TCA9555_ADDR: u8 = 0x20;
const TCA9555_REG_INPUT_0: u8 = 0x00;
const TCA9555_REG_CONFIG_0: u8 = 0x06;

const NUM_KEYS: usize = 16;

// APA102 per-LED global brightness, 0..=31. Keep this low: 16 LEDs at full
// tilt will sag the Pico's 3V3 rail.
const LED_BRIGHTNESS: u8 = 4;

#[derive(Copy, Clone)]
struct Rgb {
    r: u8,
    g: u8,
    b: u8,
}

const COLOR_IDLE: Rgb = Rgb { r: 8, g: 8, b: 40 };
const COLOR_PRESSED: Rgb = Rgb { r: 255, g: 80, b: 0 };

#[entry]
fn main() -> ! {
    let mut pac = pac::Peripherals::take().unwrap();
    let core = pac::CorePeripherals::take().unwrap();

    let mut watchdog = hal::Watchdog::new(pac.WATCHDOG);
    let clocks = hal::clocks::init_clocks_and_plls(
        rp_pico::XOSC_CRYSTAL_FREQ,
        pac.XOSC,
        pac.CLOCKS,
        pac.PLL_SYS,
        pac.PLL_USB,
        &mut pac.RESETS,
        &mut watchdog,
    )
    .ok()
    .unwrap();

    let mut delay = cortex_m::delay::Delay::new(core.SYST, clocks.system_clock.freq().to_Hz());

    let sio = hal::Sio::new(pac.SIO);
    let pins = rp_pico::Pins::new(
        pac.IO_BANK0,
        pac.PADS_BANK0,
        sio.gpio_bank0,
        &mut pac.RESETS,
    );

    let mut onboard_led = pins.led.into_push_pull_output();

    // SPI0 for the APA102 chain. GP16 is a dummy MISO: rp2040-hal's SPI
    // pin tuple needs all three pins, and the keypad base leaves GP16
    // unconnected.
    let spi_sclk = pins.gpio18.into_function::<hal::gpio::FunctionSpi>();
    let spi_mosi = pins.gpio19.into_function::<hal::gpio::FunctionSpi>();
    let spi_miso = pins.gpio16.into_function::<hal::gpio::FunctionSpi>();
    let mut spi_cs = pins.gpio17.into_push_pull_output();
    spi_cs.set_high().unwrap();

    let mut spi = hal::Spi::<_, _, _, 8>::new(pac.SPI0, (spi_mosi, spi_miso, spi_sclk)).init(
        &mut pac.RESETS,
        clocks.peripheral_clock.freq(),
        4.MHz(),
        embedded_hal::spi::MODE_0,
    );

    // I2C0 for the TCA9555.
    let sda: hal::gpio::Pin<_, hal::gpio::FunctionI2C, hal::gpio::PullUp> =
        pins.gpio4.reconfigure();
    let scl: hal::gpio::Pin<_, hal::gpio::FunctionI2C, hal::gpio::PullUp> =
        pins.gpio5.reconfigure();

    let mut i2c = hal::I2C::i2c0(
        pac.I2C0,
        sda,
        scl,
        400.kHz(),
        &mut pac.RESETS,
        &clocks.system_clock,
    );

    // Configure both TCA9555 banks as inputs. Power-on default is already
    // all-input; set it explicitly in case of a warm reset.
    let _ = i2c.write(TCA9555_ADDR, &[TCA9555_REG_CONFIG_0, 0xFF, 0xFF]);

    const LOOP_MS: u32 = 10;
    const HEARTBEAT_HALF_PERIOD: u32 = 500 / LOOP_MS; // toggle every 500 ms
    let mut ticks: u32 = 0;

    let mut frame = [COLOR_IDLE; NUM_KEYS];

    loop {
        let pressed = read_buttons(&mut i2c).unwrap_or(0);
        for i in 0..NUM_KEYS {
            frame[i] = if (pressed >> i) & 1 != 0 {
                COLOR_PRESSED
            } else {
                COLOR_IDLE
            };
        }
        let _ = write_leds(&mut spi, &mut spi_cs, &frame);

        ticks = ticks.wrapping_add(1);
        if ticks % HEARTBEAT_HALF_PERIOD == 0 {
            onboard_led.toggle().unwrap();
        }

        delay.delay_ms(LOOP_MS);
    }
}

/// Reads both TCA9555 input registers. Returns a 16-bit mask where bit `i`
/// is set iff key `i` is currently pressed. Keys pull the expander pin to
/// GND, so the raw register reads active-low; we invert here.
fn read_buttons<I: I2c>(i2c: &mut I) -> Option<u16> {
    i2c.write(TCA9555_ADDR, &[TCA9555_REG_INPUT_0]).ok()?;
    let mut buf = [0u8; 2];
    i2c.read(TCA9555_ADDR, &mut buf).ok()?;
    Some(!u16::from_le_bytes(buf))
}

/// Writes one APA102 frame to the 16-LED chain.
///
/// Frame: 32 zero bits, N × [0b111bbbbb, B, G, R] per LED, then at least
/// N/2 one bits to clock the last pixel through the chain.
fn write_leds<S, CS>(spi: &mut S, cs: &mut CS, leds: &[Rgb; NUM_KEYS]) -> Result<(), ()>
where
    S: SpiBus<u8>,
    CS: OutputPin,
{
    cs.set_low().map_err(|_| ())?;
    spi.write(&[0x00; 4]).map_err(|_| ())?;
    let header = 0xE0 | (LED_BRIGHTNESS & 0x1F);
    for led in leds.iter() {
        spi.write(&[header, led.b, led.g, led.r]).map_err(|_| ())?;
    }
    spi.write(&[0xFF; 4]).map_err(|_| ())?;
    cs.set_high().map_err(|_| ())?;
    Ok(())
}
