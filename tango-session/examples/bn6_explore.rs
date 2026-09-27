//! Script-driven BN6 explorer, for finding and checking RAM addresses.
//! Boots a training battle (both seats on the same save), runs a command
//! script, and saves screenshots of both seats, raw snapshots of seat 1's
//! battle RAM (the `ram-probe` feature's raw window), and seat 1's decoded
//! [`Bn6Obs`](tango_gamesupport_bn6::observe::Bn6Obs).
//!
//! bn6_explore <save.sav> <script.txt> <out dir>
//!
//! Script, one command per line (`#` comments):
//!   tap <0|1|both> KEY [KEY..]   press for 3 ticks, then release for 5
//!   hold <0|1|both> KEY N        hold for N ticks, then release for 5
//!   wait N                       run N ticks
//!   set <0|1|both> KEY [KEY..]   hold keys until `clear`
//!   clear <0|1|both>             release everything
//!   until <0|1> open|closed      run until that seat's custom screen is so (max 3000)
//!   shot NAME                    NAME-p0.png / NAME-p1.png
//!   dump NAME                    NAME.bin: seat 1's raw RAM snapshot
//!   trace NAME N STEP            NAME.bin: a snapshot every STEP ticks for N ticks
//!   obs                          print seat 1's decoded observation to log.txt
//!   lab                          use every chip in the folder once (see `lab`)

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tango_gamesupport_bn6::observe::Bn6Obs;
use tango_session::keys;
use tango_session::opponent::{Opponent, View};
use tango_session::Session;

#[derive(Default)]
struct Shared {
    held: [u32; 2],
    custom: [bool; 2],
    hp: [u16; 2],
    tiles: [(u8, u8); 2],
    dump: Option<tango_match::telemetry::Detail>,
}

struct Seat1(Arc<Mutex<Shared>>);

impl Opponent for Seat1 {
    fn input(&mut self, view: &View<'_>) -> u32 {
        let mut s = self.0.lock().unwrap();
        if let Some(b) = view.battle {
            s.custom = b.custom;
            s.hp = [b.units[0].hp, b.units[1].hp];
            s.tiles = [b.units[0].tile, b.units[1].tile];
            s.dump = b.detail[1].clone();
        }
        s.held[1]
    }
}

fn key(name: &str) -> u32 {
    match name {
        "A" => keys::A,
        "B" => keys::B,
        "SELECT" => keys::SELECT,
        "START" => keys::START,
        "RIGHT" => keys::RIGHT,
        "LEFT" => keys::LEFT,
        "UP" => keys::UP,
        "DOWN" => keys::DOWN,
        "R" => keys::R,
        "L" => keys::L,
        _ => panic!("unknown key {name}"),
    }
}

struct Explorer {
    session: tango_session::training::TrainingSession,
    driver: tango_session::training::Driver,
    shared: Arc<Mutex<Shared>>,
    out: PathBuf,
    tick: u32,
}

impl Explorer {
    fn step(&mut self) {
        let p0 = self.shared.lock().unwrap().held[0];
        self.session.set_input(tango_session::HostInput::keys(p0));
        assert!(self.driver.tick(), "session ended at tick {}", self.tick);
        self.tick += 1;
    }

    fn run(&mut self, ticks: u32) {
        for _ in 0..ticks {
            self.step();
        }
    }

    fn press(&mut self, seats: &[usize], bits: u32, ticks: u32) {
        for &s in seats {
            self.shared.lock().unwrap().held[s] = bits;
        }
        self.run(ticks);
        for &s in seats {
            self.shared.lock().unwrap().held[s] = 0;
        }
        self.run(5);
    }

    fn snapshot(&self) -> Vec<u8> {
        let obs = self.obs().expect("no observation: is a battle live?");
        obs.raw.clone().expect("no raw RAM: is TANGO_BN6_RAWDUMP set?")
    }

    fn obs(&self) -> Option<Bn6Obs> {
        let s = self.shared.lock().unwrap();
        s.dump.as_ref()?.downcast_ref::<Bn6Obs>().cloned()
    }

    fn shot(&self, name: &str) {
        let (w, h) = self.session.frame_size();
        let frames = [Some(self.session.frame()), self.session.pip_frame()];
        for (seat, frame) in frames.iter().enumerate() {
            if let Some(f) = frame {
                let path = self.out.join(format!("{name}-p{seat}.png"));
                image::save_buffer(&path, f, w, h, image::ExtendedColorType::Rgba8).expect("save png");
            }
        }
    }
}

/// The `lab` command: use every chip in seat 1's folder once, capturing
/// RAM and screenshots around each use. Seat 1 steers its own chip
/// screen from its decoded observation; seat 0 only confirms. Both stay
/// at their back column in the middle row (where a battle starts), so
/// the tile in front of the user is free for objects to spawn.
fn lab(x: &mut Explorer, names: &dyn Fn(u16) -> String, want: &[u16], log: &mut std::fs::File) {
    use tango_gamesupport_bn6::observe::{Cursor, CUSTOM_GAUGE_FULL};
    let mut tested = std::collections::HashSet::new();
    let mut n = 0;
    for round in 0..16 {
        if want.iter().all(|id| tested.contains(id)) {
            break;
        }
        // Open the chip screen: it opens by itself at battle start; later
        // it needs a full gauge and L.
        let mut waited = 0;
        while x.obs().and_then(|o| o.chip_select).is_none() {
            if x.obs().is_some_and(|o| o.custom_gauge >= CUSTOM_GAUGE_FULL) {
                x.press(&[1], keys::L, 3);
            } else {
                x.run(10);
            }
            waited += 1;
            assert!(waited < 400, "round {round}: chip screen never opened");
        }
        x.run(20);
        // Pick up to five untested chips, one at a time, closed-loop on
        // the cursor. A pick that doesn't land (code mismatch) is skipped.
        let mut skip = std::collections::HashSet::new();
        for _ in 0..5 {
            let Some(cs) = x.obs().and_then(|o| o.chip_select) else {
                break;
            };
            let Some(target) = (0..8).find(|&i| {
                cs.hand[i]
                    .is_some_and(|c| want.contains(&c.id) && !tested.contains(&c.id) && !skip.contains(&(i as u8)))
                    && !cs.picked.contains(&(i as u8))
            }) else {
                break;
            };
            let before = cs.picked.len();
            for _ in 0..20 {
                let Some(cs) = x.obs().and_then(|o| o.chip_select) else {
                    break;
                };
                let key = match cs.cursor {
                    Cursor::Hand(c) if c as usize == target => {
                        x.press(&[1], keys::A, 3);
                        break;
                    }
                    Cursor::Hand(c) if (c < 5) != (target < 5) => {
                        if target < 5 {
                            keys::UP
                        } else {
                            keys::DOWN
                        }
                    }
                    Cursor::Hand(c) if (c as usize) < target => keys::RIGHT,
                    Cursor::Hand(_) => keys::LEFT,
                    Cursor::CrossBar | Cursor::CrossList => keys::B,
                    _ => keys::LEFT,
                };
                x.press(&[1], key, 3);
            }
            let after = x.obs().and_then(|o| o.chip_select).map_or(before, |cs| cs.picked.len());
            if after == before {
                skip.insert(target as u8);
            }
        }
        // Confirm both. START only while the window is open (it pauses the
        // battle otherwise); seat 0's openness comes from its custom flag.
        for _ in 0..6 {
            if x.obs().and_then(|o| o.chip_select).is_some() {
                x.press(&[1], keys::START, 3);
            }
            if x.obs().and_then(|o| o.chip_select).is_some() {
                x.press(&[1], keys::A, 3);
            }
            if x.shared.lock().unwrap().custom[0] {
                x.press(&[0], keys::START, 3);
            }
            if x.shared.lock().unwrap().custom[0] {
                x.press(&[0], keys::A, 3);
            }
            let s = x.shared.lock().unwrap();
            if !s.custom[0] && !s.custom[1] {
                break;
            }
        }
        x.run(90);
        // Use the queue, one chip at a time.
        let queue = x.obs().map(|o| o.queue).unwrap_or_default();
        writeln!(
            log,
            "{:>6} round {round}: queue {:?}",
            x.tick,
            queue.iter().map(|&id| names(id)).collect::<Vec<_>>()
        )
        .unwrap();
        for id in queue {
            n += 1;
            let tag = format!("{n:02}_{}", names(id).replace(|c: char| !c.is_ascii_alphanumeric(), ""));
            // A fresh row each chip, so an object's spawn tile (the one in
            // front) isn't still held by an earlier object.
            let row = 1 + (n % 3) as u8;
            for _ in 0..4 {
                let y = x.shared.lock().unwrap().tiles[1].1;
                if y == row {
                    break;
                }
                x.press(&[1], if y > row { keys::UP } else { keys::DOWN }, 3);
                x.run(12);
            }
            std::fs::write(x.out.join(format!("{tag}_pre.bin")), x.snapshot()).unwrap();
            x.shot(&format!("{tag}_pre"));
            // A is swallowed while the previous chip is still animating:
            // press until the queue actually shrinks, and time from there.
            let len = |x: &Explorer| x.obs().map_or(0, |o| o.queue.len());
            let before = len(x);
            for _ in 0..30 {
                x.press(&[1], keys::A, 3);
                if len(x) < before {
                    break;
                }
                x.run(10);
            }
            writeln!(
                log,
                "{:>6} {tag} fired from {:?}",
                x.tick,
                x.shared.lock().unwrap().tiles[1]
            )
            .unwrap();
            x.run(52);
            std::fs::write(x.out.join(format!("{tag}_60.bin")), x.snapshot()).unwrap();
            x.shot(&format!("{tag}_60"));
            x.run(120);
            std::fs::write(x.out.join(format!("{tag}_180.bin")), x.snapshot()).unwrap();
            x.shot(&format!("{tag}_180"));
            let summary = x.obs().map(|o| {
                use tango_match::telemetry::GameDetail;
                o.summary()
            });
            let s = x.shared.lock().unwrap();
            writeln!(
                log,
                "{:>6} used {tag}: hp {:?}\n{}",
                x.tick,
                s.hp,
                summary.unwrap_or_default()
            )
            .unwrap();
            drop(s);
            tested.insert(id);
        }
    }
    let missing: Vec<String> = want
        .iter()
        .filter(|id| !tested.contains(id))
        .map(|&id| names(id))
        .collect();
    writeln!(log, "lab done at tick {}; untested {missing:?}", x.tick).unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, save, script, out] = &args[..] else {
        panic!("usage: bn6_explore <save.sav> <script.txt> <out dir>");
    };
    std::env::set_var("TANGO_BN6_RAWDUMP", "1");
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let sram = std::fs::read(save).expect("read save");
    let script = std::fs::read_to_string(script).expect("read script");
    let (game, rom) = tango_library::bnlc::scan_steam_roms()
        .into_iter()
        .find(|(g, _)| g.family_and_variant() == ("bn6", 0))
        .expect("BN6 Gregar (US) in the Legacy Collection");

    // Chip names and the folder, for `lab`.
    let parsed = game.parse_save(&sram).expect("parse save");
    let save_view = tango_gamesupport_common_dataview::save_ref(parsed.as_ref());
    let wram = save_view.as_raw_wram().into_owned();
    let assets = game.load_rom_assets(&rom, &wram, None).expect("rom assets");
    let assets = tango_gamesupport_common_dataview::assets_ref(assets.as_ref());
    let names = |id: u16| {
        assets
            .chip(id as usize)
            .and_then(|c| c.name())
            .unwrap_or_else(|| format!("chip{id}"))
    };
    let folder: Vec<u16> = {
        let chips = save_view.view_chips().expect("chips view");
        let f = chips.equipped_folder_index();
        let mut ids: Vec<u16> = (0..chips.folder_size())
            .filter_map(|i| chips.chip(f, i))
            .map(|c| c.id as u16)
            .collect();
        ids.sort();
        ids.dedup();
        ids
    };

    let (session, mut driver, _audio) = tango_session::training::TrainingSession::new(
        game,
        Arc::new(rom.clone()),
        sram.clone(),
        std::time::SystemTime::UNIX_EPOCH,
        [0; 16],
        48000,
    )
    .expect("boot training");
    session.set_opponent_visible(true);
    let shared = Arc::new(Mutex::new(Shared::default()));
    driver.set_opponent(Box::new(Seat1(shared.clone())));
    let mut x = Explorer {
        session,
        driver,
        shared,
        out,
        tick: 0,
    };

    let mut log = std::fs::File::create(x.out.join("log.txt")).unwrap();
    for line in script
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let w: Vec<&str> = line.split_whitespace().collect();
        let seats = |s: &str| -> Vec<usize> {
            match s {
                "both" => vec![0, 1],
                s => vec![s.parse().unwrap()],
            }
        };
        match w[0] {
            "tap" => {
                let bits = w[2..].iter().map(|k| key(k)).fold(0, |a, b| a | b);
                x.press(&seats(w[1]), bits, 3);
            }
            "hold" => x.press(&seats(w[1]), key(w[2]), w[3].parse().unwrap()),
            "set" => {
                let bits = w[2..].iter().map(|k| key(k)).fold(0, |a, b| a | b);
                for s in seats(w[1]) {
                    x.shared.lock().unwrap().held[s] = bits;
                }
            }
            "clear" => {
                for s in seats(w[1]) {
                    x.shared.lock().unwrap().held[s] = 0;
                }
            }
            "wait" => x.run(w[1].parse().unwrap()),
            "until" => {
                let seat: usize = w[1].parse().unwrap();
                let want = w[2] == "open";
                let mut n = 0;
                while x.shared.lock().unwrap().custom[seat] != want {
                    x.step();
                    n += 1;
                    assert!(n < 3000, "`{line}` timed out");
                }
            }
            "shot" => x.shot(w[1]),
            "dump" => std::fs::write(x.out.join(format!("{}.bin", w[1])), x.snapshot()).unwrap(),
            "trace" => {
                let (n, step): (u32, u32) = (w[2].parse().unwrap(), w[3].parse().unwrap());
                let mut f = std::fs::File::create(x.out.join(format!("{}.bin", w[1]))).unwrap();
                for _ in 0..n / step {
                    f.write_all(&x.snapshot()).unwrap();
                    x.run(step);
                }
            }
            "lab" => {
                lab(&mut x, &names, &folder, &mut log);
                continue;
            }
            "obs" => {
                let mut o = x.obs();
                if let Some(o) = o.as_mut() {
                    o.raw = None;
                }
                writeln!(log, "{:>6} {o:#?}", x.tick).unwrap();
                continue;
            }
            other => panic!("unknown command {other}"),
        }
        let s = x.shared.lock().unwrap();
        writeln!(log, "{:>6} {line:<32} custom {:?} hp {:?}", x.tick, s.custom, s.hp).unwrap();
    }
    println!("done at tick {}; see {}", x.tick, x.out.display());
}
