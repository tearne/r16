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

const LED_BRIGHTNESS: u8 = 4;
const LOOP_MS: u32 = 10;

#[derive(Copy, Clone)]
struct Rgb {
    r: u8,
    g: u8,
    b: u8,
}

const C_OFF: Rgb = Rgb { r: 0, g: 0, b: 20 };
const C_RED: Rgb = Rgb { r: 255, g: 0, b: 0 };
const C_GOLD: Rgb = Rgb { r: 255, g: 160, b: 0 };
const C_PURPLE: Rgb = Rgb { r: 150, g: 0, b: 200 };
const C_BOSS: Rgb = Rgb { r: 0, g: 120, b: 255 };
const C_DEAD: Rgb = Rgb { r: 80, g: 0, b: 0 };

#[derive(Copy, Clone, PartialEq)]
enum Cell {
    Off,
    Red,
    Gold { spawn_ms: u32 },
    Purple { spawn_ms: u32, last_attack_ms: u32 },
    Boss { spawn_ms: u32 },
    Minion,
}

struct Rng {
    s: u32,
}

impl Rng {
    fn new(seed: u32) -> Self {
        Self {
            s: if seed == 0 { 0xDEAD_BEEF } else { seed },
        }
    }
    fn next(&mut self) -> u32 {
        let mut x = self.s;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.s = x;
        x
    }
    fn range(&mut self, n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            self.next() % n
        }
    }
}

struct Game {
    cells: [Cell; NUM_KEYS],
    elapsed_ms: u32,
    next_spawn_ms: u32,
    last_boss_ms: Option<u32>,
    rng: Rng,
    game_over: bool,
}

impl Game {
    fn new(seed: u32) -> Self {
        let mut g = Self {
            cells: [Cell::Off; NUM_KEYS],
            elapsed_ms: 0,
            next_spawn_ms: 3000,
            last_boss_ms: None,
            rng: Rng::new(seed),
            game_over: false,
        };
        // Start with one random red.
        if let Some(i) = g.random_off_cell() {
            g.cells[i] = Cell::Red;
        }
        g
    }

    fn lit_count(&self) -> u32 {
        self.cells.iter().filter(|c| **c != Cell::Off).count() as u32
    }

    fn has_boss(&self) -> bool {
        self.cells.iter().any(|c| matches!(c, Cell::Boss { .. }))
    }

    fn has_purple(&self) -> bool {
        self.cells.iter().any(|c| matches!(c, Cell::Purple { .. }))
    }

    fn random_off_cell(&mut self) -> Option<usize> {
        let mut offs = [0usize; NUM_KEYS];
        let mut n = 0;
        for (i, c) in self.cells.iter().enumerate() {
            if *c == Cell::Off {
                offs[n] = i;
                n += 1;
            }
        }
        if n == 0 {
            None
        } else {
            Some(offs[self.rng.range(n as u32) as usize])
        }
    }

    fn random_red_cell(&mut self) -> Option<usize> {
        let mut reds = [0usize; NUM_KEYS];
        let mut n = 0;
        for (i, c) in self.cells.iter().enumerate() {
            if *c == Cell::Red {
                reds[n] = i;
                n += 1;
            }
        }
        if n == 0 {
            None
        } else {
            Some(reds[self.rng.range(n as u32) as usize])
        }
    }

    /// Interval in ms using the spec formula, clamped to [500, +inf).
    fn spawn_interval_ms(&self) -> u32 {
        let secs = self.elapsed_ms / 1000;
        // 3000 - secs*50 + lit*100, in ms
        let base: i32 =
            3000i32 - (secs as i32) * 50 + (self.lit_count() as i32) * 100;
        if base < 500 {
            500
        } else {
            base as u32
        }
    }

    /// Raw (unclamped) interval used to decide when the boss unlocks — it
    /// unlocks once the formula wants to go at or below 500 ms, i.e. once
    /// seconds_alive has caught up to the difficulty cap.
    fn raw_interval_capped(&self) -> bool {
        let secs = self.elapsed_ms / 1000;
        let base: i32 = 3000i32 - (secs as i32) * 50;
        base <= 500
    }

    fn spawn_one(&mut self) {
        // Regular spawn: 90% red / 5% gold / 5% purple.
        let roll = self.rng.range(100);
        let kind = if roll < 90 {
            0
        } else if roll < 95 {
            1
        } else {
            2
        };
        let idx = match self.random_off_cell() {
            Some(i) => i,
            None => return,
        };
        self.cells[idx] = match kind {
            0 => Cell::Red,
            1 => Cell::Gold {
                spawn_ms: self.elapsed_ms,
            },
            _ => {
                if self.has_purple() {
                    Cell::Red
                } else {
                    Cell::Purple {
                        spawn_ms: self.elapsed_ms,
                        last_attack_ms: self.elapsed_ms,
                    }
                }
            }
        };
    }

    fn spawn_boss(&mut self) {
        let idx = match self.random_off_cell() {
            Some(i) => i,
            None => return,
        };
        self.cells[idx] = Cell::Boss {
            spawn_ms: self.elapsed_ms,
        };
        let row = idx / 4;
        let col = idx % 4;
        for k in 0..NUM_KEYS {
            if k == idx {
                continue;
            }
            if k / 4 == row || k % 4 == col {
                if self.cells[k] == Cell::Off {
                    self.cells[k] = Cell::Minion;
                }
            }
        }
    }

    fn unminion(&mut self) {
        for c in self.cells.iter_mut() {
            if *c == Cell::Minion {
                *c = Cell::Red;
            }
        }
    }

    fn press(&mut self, i: usize) {
        let cell = self.cells[i];
        match cell {
            Cell::Off => {
                self.cells[i] = Cell::Red;
            }
            Cell::Red => {
                self.cells[i] = Cell::Off;
            }
            Cell::Gold { .. } => {
                self.cells[i] = Cell::Off;
                for _ in 0..2 {
                    if let Some(r) = self.random_red_cell() {
                        self.cells[r] = Cell::Off;
                    }
                }
            }
            Cell::Purple { .. } => {
                self.cells[i] = Cell::Off;
            }
            Cell::Boss { .. } => {
                self.cells[i] = Cell::Off;
                self.unminion();
            }
            Cell::Minion => {
                // Locked while the boss is blue — ignored.
            }
        }
    }

    fn tick(&mut self, dt_ms: u32, new_presses: u16) {
        if self.game_over {
            return;
        }
        // Mix press timing into the RNG for entropy.
        if new_presses != 0 {
            self.rng.s ^= self.elapsed_ms.wrapping_mul(2_654_435_761);
        }

        self.elapsed_ms = self.elapsed_ms.wrapping_add(dt_ms);

        for i in 0..NUM_KEYS {
            if (new_presses >> i) & 1 != 0 {
                self.press(i);
            }
        }

        // Gold timeout → Red.
        for i in 0..NUM_KEYS {
            if let Cell::Gold { spawn_ms } = self.cells[i] {
                if self.elapsed_ms.wrapping_sub(spawn_ms) >= 3000 {
                    self.cells[i] = Cell::Red;
                }
            }
        }

        // Purple attacks.
        for i in 0..NUM_KEYS {
            if let Cell::Purple {
                spawn_ms,
                last_attack_ms,
            } = self.cells[i]
            {
                let age = self.elapsed_ms.wrapping_sub(spawn_ms);
                if age >= 2000
                    && self.elapsed_ms.wrapping_sub(last_attack_ms) >= 5000
                {
                    if let Some(r) = self.random_off_cell() {
                        self.cells[r] = Cell::Red;
                    }
                    if let Cell::Purple {
                        ref mut last_attack_ms,
                        ..
                    } = self.cells[i]
                    {
                        *last_attack_ms = self.elapsed_ms;
                    }
                }
            }
        }

        // Boss timeout → Red, unlock minions.
        let mut boss_timed_out = false;
        for i in 0..NUM_KEYS {
            if let Cell::Boss { spawn_ms } = self.cells[i] {
                if self.elapsed_ms.wrapping_sub(spawn_ms) >= 3000 {
                    self.cells[i] = Cell::Red;
                    boss_timed_out = true;
                }
            }
        }
        if boss_timed_out {
            self.unminion();
        }

        // Regular spawn timer.
        while self.elapsed_ms >= self.next_spawn_ms {
            self.spawn_one();
            self.next_spawn_ms = self
                .next_spawn_ms
                .saturating_add(self.spawn_interval_ms());
        }

        // Boss spawn: once difficulty is capped, every 60s, no boss present.
        if self.raw_interval_capped() && !self.has_boss() {
            let due = match self.last_boss_ms {
                None => true,
                Some(t) => self.elapsed_ms.wrapping_sub(t) >= 60_000,
            };
            if due {
                self.spawn_boss();
                self.last_boss_ms = Some(self.elapsed_ms);
            }
        }

        // Game over check: no Off cells remain.
        if !self.cells.iter().any(|c| *c == Cell::Off) {
            self.game_over = true;
        }
    }

    fn render(&self) -> [Rgb; NUM_KEYS] {
        let mut frame = [C_OFF; NUM_KEYS];
        if self.game_over {
            for p in frame.iter_mut() {
                *p = C_DEAD;
            }
            return frame;
        }
        for (i, c) in self.cells.iter().enumerate() {
            frame[i] = match c {
                Cell::Off => C_OFF,
                Cell::Red => C_RED,
                Cell::Gold { .. } => C_GOLD,
                Cell::Purple { .. } => C_PURPLE,
                Cell::Boss { .. } => C_BOSS,
                Cell::Minion => C_RED,
            };
        }
        frame
    }
}

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

    let _ = i2c.write(TCA9555_ADDR, &[TCA9555_REG_CONFIG_0, 0xFF, 0xFF]);

    const HEARTBEAT_HALF_PERIOD: u32 = 500 / LOOP_MS;
    let mut ticks: u32 = 0;
    let mut prev_pressed: u16 = 0;

    let mut game = Game::new(0xDEAD_BEEF);

    loop {
        let pressed = read_buttons(&mut i2c).unwrap_or(0);
        let new_presses = pressed & !prev_pressed;
        prev_pressed = pressed;

        game.tick(LOOP_MS, new_presses);
        let frame = game.render();
        let _ = write_leds(&mut spi, &mut spi_cs, &frame);

        ticks = ticks.wrapping_add(1);
        if ticks % HEARTBEAT_HALF_PERIOD == 0 {
            onboard_led.toggle().unwrap();
        }

        delay.delay_ms(LOOP_MS);
    }
}

fn read_buttons<I: I2c>(i2c: &mut I) -> Option<u16> {
    i2c.write(TCA9555_ADDR, &[TCA9555_REG_INPUT_0]).ok()?;
    let mut buf = [0u8; 2];
    i2c.read(TCA9555_ADDR, &mut buf).ok()?;
    Some(!u16::from_le_bytes(buf))
}

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
