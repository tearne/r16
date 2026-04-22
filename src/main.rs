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

const C_OFF: Rgb = Rgb { r: 0, g: 0, b: 0 };
const C_RED: Rgb = Rgb { r: 255, g: 0, b: 0 };
const C_GOLD: Rgb = Rgb { r: 255, g: 160, b: 0 };
const C_PURPLE: Rgb = Rgb { r: 150, g: 0, b: 200 };
const C_BOSS: Rgb = Rgb { r: 0, g: 120, b: 255 };
const C_DEAD: Rgb = Rgb { r: 80, g: 0, b: 0 };
const C_ANCHOR: Rgb = Rgb { r: 0, g: 200, b: 30 };
const C_WHITE: Rgb = Rgb { r: 200, g: 200, b: 200 };
const C_WIN: Rgb = Rgb { r: 0, g: 200, b: 0 };

fn dim(c: Rgb) -> Rgb {
    Rgb { r: c.r / 2, g: c.g / 2, b: c.b / 2 }
}

/// HSV → RGB with saturation/value = 1. Hue in [0, 360). Used for the
/// rainbow-cycled Final Boss button.
fn hue_rgb(hue_deg: u32) -> Rgb {
    let h = hue_deg % 360;
    let region = h / 60;
    let f = (h % 60) as u16;
    let t = ((255 * f) / 60) as u8;
    let q = 255u8 - t;
    match region {
        0 => Rgb { r: 255, g: t, b: 0 },
        1 => Rgb { r: q, g: 255, b: 0 },
        2 => Rgb { r: 0, g: 255, b: t },
        3 => Rgb { r: 0, g: q, b: 255 },
        4 => Rgb { r: t, g: 0, b: 255 },
        _ => Rgb { r: 255, g: 0, b: q },
    }
}
const C_SELECT_ENDLESS: Rgb = Rgb { r: 0, g: 80, b: 255 };
const C_SELECT_STORY: Rgb = Rgb { r: 255, g: 0, b: 0 };

#[derive(Copy, Clone, PartialEq)]
enum Mode {
    Endless,
    Story,
}

#[derive(Copy, Clone, PartialEq)]
enum Cell {
    Off,
    Red { shielded: bool, quiet: bool },
    Gold { spawn_ms: u32 },
    Purple { last_attack_ms: u32 },
    Boss { spawn_ms: u32, timeout_ms: u32 },
    Minion,
    Phantom { spawn_ms: u32 },
    Anchor,
    /// Story-only: looks like a red but attacks like a purple.
    Decoy { shielded: bool, quiet: bool, last_attack_ms: u32 },
    /// Final Boss only: white, teleports, press to clear.
    Bodyguard,
    /// Final Boss: hidden among bodyguards until all 3 are pressed.
    FinalBoss { revealed: bool },
}

const PLAIN_RED: Cell = Cell::Red { shielded: false, quiet: false };

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
    /// Drives the spawn-rate ramp. Equals elapsed_ms in Endless; in Story it
    /// jumps forward by the head-start at each fight transition.
    spawn_clock_ms: u32,
    next_spawn_ms: u32,
    last_boss_ms: Option<u32>,
    rng: Rng,
    game_over: bool,
    mode: Mode,
    /// Story: 1..=5 during boss fights, 6 during break, 7 pending final boss.
    fight: u8,
    /// Story: elapsed_ms at which the post-fight-5 break ends.
    break_until_ms: u32,
    /// Story: true when current fight's boss hasn't spawned yet.
    pending_boss: bool,
    /// Final Boss: current round 1..=3; 0 = not yet, 4 = won.
    final_round: u8,
    final_next_teleport_ms: u32,
    final_next_red_ms: u32,
    won: bool,
    /// Time spent on the win/lose screen. Presses are ignored until this
    /// reaches the end-screen hold window.
    end_hold_ms: u32,
}

impl Game {
    fn new(seed: u32, mode: Mode) -> Self {
        let mut g = Self {
            cells: [Cell::Off; NUM_KEYS],
            elapsed_ms: 0,
            spawn_clock_ms: 0,
            next_spawn_ms: 3000,
            last_boss_ms: None,
            rng: Rng::new(seed),
            game_over: false,
            mode,
            fight: 1,
            break_until_ms: 0,
            pending_boss: true,
            final_round: 0,
            final_next_teleport_ms: 0,
            final_next_red_ms: 0,
            won: false,
            end_hold_ms: 0,
        };
        // Start with one random red.
        if let Some(i) = g.random_off_cell() {
            g.cells[i] = PLAIN_RED;
        }
        g
    }

    fn lit_count(&self) -> u32 {
        self.cells.iter().filter(|c| **c != Cell::Off).count() as u32
    }

    fn has_boss(&self) -> bool {
        self.cells.iter().any(|c| matches!(c, Cell::Boss { .. }))
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
            if matches!(c, Cell::Red { .. } | Cell::Decoy { .. }) {
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

    /// Interval in ms using the spec formula `max(0.05, 3.0 - secs*0.01 + lit*0.1)`.
    fn spawn_interval_ms(&self) -> u32 {
        let secs = self.spawn_clock_ms / 1000;
        let base: i32 =
            3000i32 - (secs as i32) * 10 + (self.lit_count() as i32) * 100;
        if base < 50 {
            50
        } else {
            base as u32
        }
    }

    /// Head start in ms for a given Story fight number (1-indexed).
    fn story_head_start_ms(fight: u8) -> u32 {
        match fight {
            1 => 0,
            2 => 30_000,
            3 => 60_000,
            4 => 90_000,
            _ => 120_000,
        }
    }

    /// Boss timeout in ms for the current state.
    fn boss_timeout_ms(&self) -> u32 {
        match self.mode {
            Mode::Endless => 3_000,
            Mode::Story => match self.fight {
                1 => 5_000,
                2 => 7_000,
                3 => 9_000,
                4 => 11_000,
                _ => 13_000,
            },
        }
    }

    /// Boss unlocks on a fixed wall-clock timer, independent of the spawn
    /// ramp — tuning the ramp shouldn't delay the boss.
    fn boss_unlocked(&self) -> bool {
        self.elapsed_ms >= 115_000
    }

    fn purple_count(&self) -> u32 {
        self.cells
            .iter()
            .filter(|c| matches!(c, Cell::Purple { .. }))
            .count() as u32
    }

    fn spawn_one(&mut self) {
        // 85% red / 5% gold / 10% purple.
        let roll = self.rng.range(100);
        let kind = if roll < 85 {
            0
        } else if roll < 90 {
            1
        } else {
            2
        };
        let idx = match self.random_off_cell() {
            Some(i) => i,
            None => return,
        };
        let story = self.mode == Mode::Story;
        self.cells[idx] = match kind {
            0 => {
                // Within a would-be red: 10% phantom, 5% anchor, else red.
                let sub = self.rng.range(100);
                if sub < 10 {
                    Cell::Phantom { spawn_ms: self.elapsed_ms }
                } else if sub < 15 {
                    Cell::Anchor
                } else {
                    self.make_story_red(story)
                }
            }
            1 => Cell::Gold {
                spawn_ms: self.elapsed_ms,
            },
            _ => {
                if self.purple_count() >= 2 {
                    self.make_story_red(story)
                } else {
                    Cell::Purple {
                        last_attack_ms: self.elapsed_ms,
                    }
                }
            }
        };
    }

    /// Build a red-ish spawn; in Story this may become a shielded/quiet/decoy
    /// variant. In Endless it's always a plain red.
    fn make_story_red(&mut self, story: bool) -> Cell {
        if !story {
            return PLAIN_RED;
        }
        let shielded = self.rng.range(100) < 25;
        let quiet = self.rng.range(100) < 20;
        let decoy = self.rng.range(100) < 10;
        if decoy {
            Cell::Decoy {
                shielded,
                quiet,
                last_attack_ms: self.elapsed_ms,
            }
        } else {
            Cell::Red { shielded, quiet }
        }
    }

    fn spawn_boss(&mut self) {
        let idx = match self.random_off_cell() {
            Some(i) => i,
            None => return,
        };
        self.cells[idx] = Cell::Boss {
            spawn_ms: self.elapsed_ms,
            timeout_ms: self.boss_timeout_ms(),
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

    /// Called when a boss is cleared (pressed or timed out). In Story mode this
    /// advances the fight state and retunes the spawn ramp.
    fn on_boss_cleared(&mut self) {
        if self.mode != Mode::Story {
            return;
        }
        if self.fight <= 5 {
            self.fight += 1;
            if self.fight <= 5 {
                let hs = Self::story_head_start_ms(self.fight);
                if self.spawn_clock_ms < hs {
                    self.spawn_clock_ms = hs;
                }
                self.pending_boss = true;
            } else {
                // Cleared fight 5 → 10s break, then final boss.
                self.break_until_ms = self.elapsed_ms.wrapping_add(10_000);
                self.pending_boss = false;
            }
        }
    }

    fn in_break(&self) -> bool {
        self.mode == Mode::Story && self.fight == 6 && self.elapsed_ms < self.break_until_ms
    }

    fn unminion(&mut self) {
        for c in self.cells.iter_mut() {
            if *c == Cell::Minion {
                *c = PLAIN_RED;
            }
        }
    }

    fn phantom_lit(&self, spawn_ms: u32) -> bool {
        // 1s on / 1s off, starts lit at spawn.
        ((self.elapsed_ms.wrapping_sub(spawn_ms)) / 1000) % 2 == 0
    }

    fn lit_neighbour_count(&self, i: usize) -> u32 {
        let row = i / 4;
        let col = i % 4;
        let mut n = 0;
        let offsets: [(i32, i32); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];
        for (dr, dc) in offsets {
            let nr = row as i32 + dr;
            let nc = col as i32 + dc;
            if nr < 0 || nr >= 4 || nc < 0 || nc >= 4 {
                continue;
            }
            let k = (nr as usize) * 4 + nc as usize;
            if self.cells[k] != Cell::Off {
                n += 1;
            }
        }
        n
    }

    fn press(&mut self, i: usize) {
        let cell = self.cells[i];
        match cell {
            Cell::Off => {
                self.cells[i] = PLAIN_RED;
            }
            Cell::Red { shielded: true, quiet } => {
                if self.fight == 7 {
                    // Shields inactive during Final Boss.
                    self.cells[i] = Cell::Off;
                } else {
                    self.cells[i] = Cell::Red { shielded: false, quiet };
                }
            }
            Cell::Red { shielded: false, .. } => {
                self.cells[i] = Cell::Off;
            }
            Cell::Decoy { shielded: true, quiet, last_attack_ms } => {
                if self.fight == 7 {
                    // Shields (and decoy behaviour) inactive during Final Boss.
                    self.cells[i] = Cell::Off;
                } else {
                    self.cells[i] = Cell::Decoy { shielded: false, quiet, last_attack_ms };
                }
            }
            Cell::Decoy { shielded: false, .. } => {
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
                // Boss is fully locked while blue. It only becomes pressable
                // once it times out into a red button.
            }
            Cell::Minion => {
                // Locked while the boss is blue — ignored.
            }
            Cell::Phantom { spawn_ms } => {
                if self.phantom_lit(spawn_ms) {
                    self.cells[i] = Cell::Off;
                }
            }
            Cell::Anchor => {
                if self.lit_neighbour_count(i) == 0 {
                    self.cells[i] = Cell::Off;
                }
            }
            Cell::Bodyguard => {
                self.cells[i] = Cell::Off;
                self.reveal_final_boss_if_clear();
            }
            Cell::FinalBoss { revealed: false } => {
                // Must clear bodyguards first.
            }
            Cell::FinalBoss { revealed: true } => {
                if self.final_round >= 3 {
                    self.final_round = 4;
                    self.won = true;
                    self.cells[i] = Cell::Off;
                } else {
                    self.final_round += 1;
                    self.cells[i] = Cell::FinalBoss { revealed: false };
                    self.setup_final_boss_round(false);
                }
            }
        }
    }

    fn reveal_final_boss_if_clear(&mut self) {
        if self.cells.iter().any(|c| matches!(c, Cell::Bodyguard)) {
            return;
        }
        for c in self.cells.iter_mut() {
            if let Cell::FinalBoss { revealed } = c {
                *revealed = true;
            }
        }
    }

    /// Place 3 bodyguards and (if `first`) the hidden FinalBoss into random
    /// off cells, fire the round's burst of reds, and reset timers.
    fn setup_final_boss_round(&mut self, first: bool) {
        if first {
            if let Some(i) = self.random_off_cell() {
                self.cells[i] = Cell::FinalBoss { revealed: false };
            }
        }
        for _ in 0..3 {
            if let Some(i) = self.random_off_cell() {
                self.cells[i] = Cell::Bodyguard;
            }
        }
        let burst = match self.final_round {
            1 => 3,
            2 => 6,
            _ => 9,
        };
        for _ in 0..burst {
            if let Some(i) = self.random_off_cell() {
                self.cells[i] = PLAIN_RED;
            }
        }
        let teleport = match self.final_round {
            1 => 2_000,
            2 => 1_500,
            _ => 1_000,
        };
        self.final_next_teleport_ms = self.elapsed_ms.wrapping_add(teleport);
        self.final_next_red_ms = self.elapsed_ms.wrapping_add(1_000);
    }

    fn teleport_bodyguards(&mut self) {
        // Collect current bodyguard positions, clear them, then place back in
        // fresh random off cells. Boss does not teleport.
        for i in 0..NUM_KEYS {
            if matches!(self.cells[i], Cell::Bodyguard) {
                self.cells[i] = Cell::Off;
                if let Some(j) = self.random_off_cell() {
                    self.cells[j] = Cell::Bodyguard;
                } else {
                    // No empty cell — put it back where it was.
                    self.cells[i] = Cell::Bodyguard;
                }
            }
        }
    }

    fn tick(&mut self, dt_ms: u32, new_presses: u16) {
        if self.game_over || self.won {
            self.end_hold_ms = self.end_hold_ms.saturating_add(dt_ms);
            return;
        }
        // Mix press timing into the RNG for entropy.
        if new_presses != 0 {
            self.rng.s ^= self.elapsed_ms.wrapping_mul(2_654_435_761);
        }

        self.elapsed_ms = self.elapsed_ms.wrapping_add(dt_ms);
        self.spawn_clock_ms = self.spawn_clock_ms.wrapping_add(dt_ms);

        for i in 0..NUM_KEYS {
            if (new_presses >> i) & 1 != 0 {
                self.press(i);
            }
        }

        // Gold timeout → Red.
        for i in 0..NUM_KEYS {
            if let Cell::Gold { spawn_ms } = self.cells[i] {
                if self.elapsed_ms.wrapping_sub(spawn_ms) >= 3000 {
                    self.cells[i] = PLAIN_RED;
                }
            }
        }

        // Purple / Decoy attacks — spawn one random button every second.
        // Suppressed during the post-fight-5 break ("no buttons spawn") and
        // also during the Final Boss fight (decoys are inactive then).
        let attacks_suppressed = self.in_break() || self.fight == 7;
        for i in 0..NUM_KEYS {
            let last = match self.cells[i] {
                Cell::Purple { last_attack_ms } => Some(last_attack_ms),
                Cell::Decoy { last_attack_ms, .. } if !attacks_suppressed => {
                    Some(last_attack_ms)
                }
                Cell::Decoy { .. } => None,
                _ => None,
            };
            let is_purple = matches!(self.cells[i], Cell::Purple { .. });
            if is_purple && attacks_suppressed {
                continue;
            }
            if let Some(last_attack_ms) = last {
                if self.elapsed_ms.wrapping_sub(last_attack_ms) >= 1000 {
                    self.spawn_one();
                    match &mut self.cells[i] {
                        Cell::Purple { last_attack_ms } => {
                            *last_attack_ms = self.elapsed_ms;
                        }
                        Cell::Decoy { last_attack_ms, .. } => {
                            *last_attack_ms = self.elapsed_ms;
                        }
                        _ => {}
                    }
                }
            }
        }

        // Boss timeout → Red, unlock minions.
        let mut boss_timed_out = false;
        for i in 0..NUM_KEYS {
            if let Cell::Boss { spawn_ms, timeout_ms } = self.cells[i] {
                if self.elapsed_ms.wrapping_sub(spawn_ms) >= timeout_ms {
                    self.cells[i] = PLAIN_RED;
                    boss_timed_out = true;
                }
            }
        }
        if boss_timed_out {
            self.unminion();
            self.on_boss_cleared();
        }

        // Check for break → Final Boss transition.
        if self.mode == Mode::Story && self.fight == 6 && self.elapsed_ms >= self.break_until_ms {
            self.fight = 7;
            self.final_round = 1;
            self.setup_final_boss_round(true);
        }

        // Regular spawn timer — suppressed during the break and replaced
        // entirely by 1 red/sec during the Final Boss fight.
        let in_final = self.fight == 7 && !self.won;
        let spawns_suppressed = self.in_break() || in_final;
        if spawns_suppressed {
            self.next_spawn_ms = self.elapsed_ms.wrapping_add(self.spawn_interval_ms());
        } else {
            while self.elapsed_ms >= self.next_spawn_ms {
                self.spawn_one();
                self.next_spawn_ms = self
                    .next_spawn_ms
                    .saturating_add(self.spawn_interval_ms());
            }
        }

        // Final Boss: 1 red per second, plus bodyguard teleport timer.
        if in_final {
            while self.elapsed_ms >= self.final_next_red_ms {
                if let Some(i) = self.random_off_cell() {
                    self.cells[i] = PLAIN_RED;
                }
                self.final_next_red_ms = self.final_next_red_ms.saturating_add(1_000);
            }
            if self.elapsed_ms >= self.final_next_teleport_ms {
                self.teleport_bodyguards();
                let interval = match self.final_round {
                    1 => 2_000,
                    2 => 1_500,
                    _ => 1_000,
                };
                self.final_next_teleport_ms =
                    self.final_next_teleport_ms.saturating_add(interval);
            }
        }

        // Boss spawn.
        if !self.has_boss() {
            let want_boss = match self.mode {
                Mode::Endless => {
                    self.boss_unlocked()
                        && match self.last_boss_ms {
                            None => true,
                            Some(t) => self.elapsed_ms.wrapping_sub(t) >= 30_000,
                        }
                }
                Mode::Story => {
                    // One boss per fight (1..=5). Fight 1 waits for the 115s
                    // unlock; fights 2–5 wait 60s after the previous boss
                    // spawn (spec: "every 60 seconds in story mode").
                    self.pending_boss
                        && self.fight >= 1
                        && self.fight <= 5
                        && match self.last_boss_ms {
                            None => self.boss_unlocked(),
                            Some(t) => self.elapsed_ms.wrapping_sub(t) >= 60_000,
                        }
                }
            };
            if want_boss {
                self.spawn_boss();
                self.last_boss_ms = Some(self.elapsed_ms);
                self.pending_boss = false;
            }
        }

        // Game over check: no Off cells remain.
        if !self.cells.iter().any(|c| *c == Cell::Off) {
            self.game_over = true;
        }
    }

    fn render(&self) -> [Rgb; NUM_KEYS] {
        let mut frame = [C_OFF; NUM_KEYS];
        if self.won {
            for p in frame.iter_mut() {
                *p = C_WIN;
            }
            return frame;
        }
        if self.game_over {
            for p in frame.iter_mut() {
                *p = C_DEAD;
            }
            return frame;
        }
        for (i, c) in self.cells.iter().enumerate() {
            frame[i] = match c {
                Cell::Off => C_OFF,
                Cell::Red { quiet, .. } => if *quiet { dim(C_RED) } else { C_RED },
                Cell::Decoy { quiet, .. } => if *quiet { dim(C_RED) } else { C_RED },
                Cell::Gold { .. } => C_GOLD,
                Cell::Purple { .. } => C_PURPLE,
                Cell::Boss { .. } => C_BOSS,
                Cell::Minion => C_RED,
                Cell::Phantom { spawn_ms } => {
                    if self.phantom_lit(*spawn_ms) { C_RED } else { C_OFF }
                }
                Cell::Anchor => C_ANCHOR,
                Cell::Bodyguard => C_WHITE,
                Cell::FinalBoss { revealed: false } => C_WHITE,
                Cell::FinalBoss { revealed: true } => hue_rgb(self.elapsed_ms / 10),
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

    enum AppState {
        Select,
        Playing(Game),
    }

    let mut state = AppState::Select;
    let mut seed: u32 = 0xDEAD_BEEF;

    loop {
        let pressed = read_buttons(&mut i2c).unwrap_or(0);
        let new_presses = pressed & !prev_pressed;
        prev_pressed = pressed;

        // Stir the RNG seed with button timing for entropy across sessions.
        if new_presses != 0 {
            seed = seed.wrapping_mul(2_654_435_761).wrapping_add(ticks);
        }

        let frame = match &mut state {
            AppState::Select => {
                let mut picked: Option<Mode> = None;
                for i in 0..NUM_KEYS {
                    if (new_presses >> i) & 1 != 0 {
                        picked = Some(if i % 4 < 2 { Mode::Endless } else { Mode::Story });
                        break;
                    }
                }
                if let Some(mode) = picked {
                    state = AppState::Playing(Game::new(seed, mode));
                    [C_OFF; NUM_KEYS]
                } else {
                    render_select()
                }
            }
            AppState::Playing(game) => {
                game.tick(LOOP_MS, new_presses);
                if (game.game_over || game.won)
                    && new_presses != 0
                    && game.end_hold_ms >= 3_000
                {
                    state = AppState::Select;
                    [C_OFF; NUM_KEYS]
                } else {
                    game.render()
                }
            }
        };
        let _ = write_leds(&mut spi, &mut spi_cs, &frame);

        ticks = ticks.wrapping_add(1);
        if ticks % HEARTBEAT_HALF_PERIOD == 0 {
            onboard_led.toggle().unwrap();
        }

        delay.delay_ms(LOOP_MS);
    }
}

fn render_select() -> [Rgb; NUM_KEYS] {
    let mut frame = [C_OFF; NUM_KEYS];
    for i in 0..NUM_KEYS {
        frame[i] = if i % 4 < 2 { C_SELECT_ENDLESS } else { C_SELECT_STORY };
    }
    frame
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
