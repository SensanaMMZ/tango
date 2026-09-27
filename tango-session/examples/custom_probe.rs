//! Headless probe: which inputs get a BN6 link battle through its custom
//! (chip-select) screen?
//!
//! Boots a real training battle from the Legacy Collection ROM and a
//! save, then for each candidate strategy drives BOTH seats with it and
//! reports the ticks at which each seat's custom screen opened and
//! closed. Then it puts the real [`Random`] opponent on one seat, with
//! the player seat confirming its chips and tapping L in battle, to
//! check the bot gets through chip select, fights, and reopens the
//! custom screen once its gauge fills. No window, no audio, no pacing:
//! it runs as fast as the emulator does.
//!
//! ```text
//! cargo run --release -p tango-session --example custom_probe -- "<path to BN6 Gregar.sav>"
//! ```

use std::sync::{Arc, Mutex};

use tango_session::keys;
use tango_session::opponent::{Opponent, Random, View};
use tango_session::Session;

use tango_match::telemetry::Event as tango_match_event;

/// A pad word for a seat, from how long its custom screen has been open
/// (`None`: not open).
type Strategy = fn(Option<u32>) -> u32;

/// Tap `bit` for 3 ticks out of every `period`.
fn tap(bit: u32, t: u32, period: u32) -> u32 {
    if t % period < 3 {
        bit
    } else {
        0
    }
}

const STRATEGIES: &[(&str, Strategy)] = &[
    ("nothing", |_| 0),
    ("A", |t| t.map_or(0, |t| tap(keys::A, t, 8))),
    ("START", |t| t.map_or(0, |t| tap(keys::START, t, 8))),
    ("R", |t| t.map_or(0, |t| tap(keys::R, t, 8))),
    ("L", |t| t.map_or(0, |t| tap(keys::L, t, 8))),
    ("SELECT", |t| t.map_or(0, |t| tap(keys::SELECT, t, 8))),
    // Walk the cursor right, past the chip rows, then press A.
    ("RIGHTx6,A", |t| {
        t.map_or(0, |t| {
            let step = (t / 8) % 7;
            tap(if step < 6 { keys::RIGHT } else { keys::A }, t, 8)
        })
    }),
    // Pick a chip, then START, then A.
    ("A,START,A", |t| {
        t.map_or(0, |t| {
            [keys::A, keys::START, keys::A][((t / 8) % 3) as usize] & tap(u32::MAX, t, 8)
        })
    }),
];

/// What the probe saw of one seat.
#[derive(Default, Debug)]
struct SeatLog {
    opened: Vec<u32>,
    closed: Vec<u32>,
}

/// Shared between the opponent seat (inside the driver) and the main
/// loop (which drives the player seat off the same readings).
#[derive(Default)]
struct Shared {
    tick: u32,
    custom: [bool; 2],
    open_since: [Option<u32>; 2],
    logs: [SeatLog; 2],
    hp: [u16; 2],
    saw_battle: bool,
    /// Chips each player used, per custom-screen episode (index = how
    /// many times that player's screen had closed before the use).
    chips: [Vec<u32>; 2],
}

impl Shared {
    fn observe(&mut self, view: &View<'_>) {
        self.tick += 1;
        for (_, e) in view.events {
            if let tango_match_event::ChipUsed { player, .. } = e {
                let episode = self.logs[*player].closed.len();
                let per = &mut self.chips[*player];
                if per.len() <= episode {
                    per.resize(episode + 1, 0);
                }
                per[episode] += 1;
            }
        }
        let Some(b) = view.battle else { return };
        self.saw_battle = true;
        self.hp = [b.units[0].hp, b.units[1].hp];
        for seat in 0..2 {
            if b.custom[seat] != self.custom[seat] {
                if b.custom[seat] {
                    self.logs[seat].opened.push(self.tick);
                    self.open_since[seat] = Some(self.tick);
                } else {
                    self.logs[seat].closed.push(self.tick);
                    self.open_since[seat] = None;
                }
            }
        }
        self.custom = b.custom;
    }

    fn input(&self, seat: usize, strategy: Strategy) -> u32 {
        strategy(self.open_since[seat].map(|since| self.tick - since))
    }
}

/// Records what its seat sees, then plays either a scripted strategy or
/// a real opponent.
struct Probe {
    shared: Arc<Mutex<Shared>>,
    strategy: Strategy,
    real: Option<Box<dyn Opponent>>,
}

impl Opponent for Probe {
    fn input(&mut self, view: &View<'_>) -> u32 {
        let mut s = self.shared.lock().unwrap();
        s.observe(view);
        match &mut self.real {
            Some(real) => real.input(view),
            None => s.input(view.seat, self.strategy),
        }
    }
}

/// The player seat's script for the bot run: confirm chips, and tap L
/// every second in battle to reopen the custom screen when it can.
fn player(open_for: Option<u32>, tick: u32) -> u32 {
    match open_for {
        Some(t) => [keys::A, keys::START, keys::A][((t / 8) % 3) as usize] & tap(u32::MAX, t, 8),
        None => tap(keys::L, tick, 60),
    }
}

/// The player seat confirming its chips but never opening the custom
/// screen: any reopen is the bot's own L/R.
fn passive_player(open_for: Option<u32>, _tick: u32) -> u32 {
    open_for.map_or(0, |t| {
        [keys::A, keys::START, keys::A][((t / 8) % 3) as usize] & tap(u32::MAX, t, 8)
    })
}

/// Boot a battle with `opponent` on seat 1, drive seat 0 with
/// `seat0(open_for, tick)` for `ticks` ticks, and print what happened.
fn run(
    name: &str,
    game: tango_library::rom::GameRef,
    rom: &Arc<Vec<u8>>,
    sram: &[u8],
    strategy: Strategy,
    real: Option<Box<dyn Opponent>>,
    seat0: impl Fn(Option<u32>, u32) -> u32,
    ticks: u32,
) {
    let (session, mut driver, _audio) = tango_session::training::TrainingSession::new(
        game,
        rom.clone(),
        sram.to_vec(),
        std::time::SystemTime::UNIX_EPOCH,
        [0; 16],
        48000,
    )
    .expect("boot training");
    let shared = Arc::new(Mutex::new(Shared::default()));
    driver.set_opponent(Box::new(Probe {
        shared: shared.clone(),
        strategy,
        real,
    }));
    let mut ended = false;
    for _ in 0..ticks {
        // Seat 0 plays off the readings seat 1's probe just took.
        let keys = {
            let s = shared.lock().unwrap();
            seat0(s.open_since[0].map(|since| s.tick - since), s.tick)
        };
        session.set_input(tango_session::HostInput::keys(keys));
        if !driver.tick() {
            ended = true;
            break;
        }
    }
    let s = shared.lock().unwrap();
    println!(
        "{name:>10}: battle seen {}, hp {:?}, ended {ended}, chips used per chip-select p0 {:?} p1 {:?}\n            p0 opened {:?} closed {:?}\n            p1 opened {:?} closed {:?}",
        s.saw_battle, s.hp, s.chips[0], s.chips[1], s.logs[0].opened, s.logs[0].closed, s.logs[1].opened, s.logs[1].closed,
    );
}

fn main() {
    let save_path = std::env::args().nth(1).expect("usage: custom_probe <BN6 Gregar .sav>");
    let sram = std::fs::read(&save_path).expect("read save");
    let roms = tango_library::bnlc::scan_steam_roms();
    let (game, rom) = roms
        .into_iter()
        .find(|(g, _)| g.family_and_variant() == ("bn6", 0))
        .expect("BN6 Gregar (US) in the Legacy Collection");
    let rom = Arc::new(rom);

    // 30 seconds each: which inputs close the first custom screen?
    for &(name, strategy) in STRATEGIES {
        run(name, game, &rom, &sram, strategy, None, |t, _| strategy(t), 1800);
    }
    // 90 seconds of the real random bot against a scripted player.
    run(
        "Random",
        game,
        &rom,
        &sram,
        STRATEGIES[0].1,
        Some(Box::new(Random::new([5; 16]))),
        player,
        5400,
    );
    // The same, but only the bot ever presses L/R.
    run(
        "Random/solo",
        game,
        &rom,
        &sram,
        STRATEGIES[0].1,
        Some(Box::new(Random::new([5; 16]))),
        passive_player,
        5400,
    );
}
