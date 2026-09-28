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
//!   duel                         seat 1 sets defenses, seat 0 attacks them (see `duel`)
//!   roads                        lay each road under seat 0 and follow it (see `roads`)

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
        self.obs_of(1)
    }

    /// Either seat's decoded observation (the host sees both cores).
    fn obs_of(&self, seat: usize) -> Option<Bn6Obs> {
        let b = self.driver.last_battle()?;
        b.detail[seat].as_ref()?.downcast_ref::<Bn6Obs>().cloned()
    }

    /// Both cores' raw RAM.
    fn snapshot_both(&self, name: &str) {
        for seat in 0..2 {
            if let Some(raw) = self.obs_of(seat).and_then(|o| o.raw) {
                std::fs::write(self.out.join(format!("{name}-c{seat}.bin")), raw).unwrap();
            }
        }
    }

    /// Open the chip screen (a full gauge and L), unless it is open.
    fn open_chip_select(&mut self) {
        let mut waited = 0;
        while self.obs_of(1).and_then(|o| o.chip_select).is_none() {
            if self
                .obs_of(1)
                .is_some_and(|o| o.custom_gauge >= tango_gamesupport_bn6::observe::CUSTOM_GAUGE_FULL)
            {
                self.press(&[1], keys::L, 3);
            } else {
                self.run(10);
            }
            waited += 1;
            assert!(waited < 400, "chip screen never opened");
        }
        self.run(20);
    }

    /// Pick up to five chips for `seat` whose ids satisfy `want`, in order
    /// of preference, closed-loop on the cursor. Returns the ids picked.
    fn pick(&mut self, seat: usize, want: &dyn Fn(u16) -> Option<u32>) -> Vec<u16> {
        use tango_gamesupport_bn6::observe::Cursor;
        let mut picked = vec![];
        let mut skip = std::collections::HashSet::new();
        for _ in 0..5 {
            let Some(cs) = self.obs_of(seat).and_then(|o| o.chip_select) else {
                break;
            };
            let Some(target) = (0..8)
                .filter(|&i| !cs.picked.contains(&(i as u8)) && !skip.contains(&i))
                .filter_map(|i| Some((want(cs.hand[i]?.id)?, i)))
                .min()
                .map(|(_, i)| i)
            else {
                break;
            };
            let before = cs.picked.len();
            let id = cs.hand[target].map(|c| c.id);
            for _ in 0..20 {
                let Some(cs) = self.obs_of(seat).and_then(|o| o.chip_select) else {
                    break;
                };
                let key = match cs.cursor {
                    Cursor::Hand(c) if c as usize == target => {
                        self.press(&[seat], keys::A, 3);
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
                self.press(&[seat], key, 3);
            }
            match self.obs_of(seat).and_then(|o| o.chip_select) {
                Some(cs) if cs.picked.len() > before => picked.push(id),
                _ => {
                    skip.insert(target);
                }
            }
        }
        picked.into_iter().flatten().collect()
    }

    /// Confirm both chip screens: START only while a window is open (it
    /// pauses the battle otherwise).
    fn confirm_both(&mut self) {
        for _ in 0..8 {
            for seat in 0..2 {
                if self.obs_of(seat).and_then(|o| o.chip_select).is_some() {
                    self.press(&[seat], keys::START, 3);
                }
                if self.obs_of(seat).and_then(|o| o.chip_select).is_some() {
                    self.press(&[seat], keys::A, 3);
                }
            }
            if (0..2).all(|s| self.obs_of(s).and_then(|o| o.chip_select).is_none()) {
                break;
            }
            self.run(30);
        }
        self.run(90);
    }

    /// Use `seat`'s next queued chip: A until its queue shrinks.
    fn use_chip(&mut self, seat: usize) -> bool {
        let len = |x: &Explorer| x.obs_of(seat).map_or(0, |o| o.queue.len());
        let before = len(self);
        if before == 0 {
            return false;
        }
        for _ in 0..30 {
            self.press(&[seat], keys::A, 3);
            if len(self) < before {
                return true;
            }
            self.run(10);
        }
        false
    }

    /// Move `seat` to row `row` (1-3).
    fn to_row(&mut self, seat: usize, row: u8) {
        for _ in 0..4 {
            let y = self.shared.lock().unwrap().tiles[seat].1;
            if y == row {
                return;
            }
            self.press(&[seat], if y > row { keys::UP } else { keys::DOWN }, 3);
            self.run(12);
        }
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

/// The `duel` command: seat 1 sets each defensive chip (barriers, auras,
/// Anti- traps, ElemTrap, Invisibl) and seat 0 then attacks it from the
/// same row: a chip that triggers traps (SpoutMan, Vulcan, Recov50 for
/// AntiRecv...) or a charged shot. Seat 1 never hits back, so it should
/// also be driven into Anger. Both cores' RAM and screenshots are saved
/// before, after setting, and after the hit.
fn duel(x: &mut Explorer, names: &dyn Fn(u16) -> String, folder: &[u16], log: &mut std::fs::File) {
    const DEF: &[&str] = &[
        "AntiDmg", "AntiNavi", "AntiRecv", "AntiSwrd", "ElemTrap", "Barrier", "Barr100", "Barr200", "BblWrap",
        "LifeAur", "Invisibl",
    ];
    const ATK: &[&str] = &["SpoutMan", "Recov50", "SuprVulc", "Vulcan1", "AirRaid3", "AirSpin3"];
    let rank = |list: &[&str], id: u16| list.iter().position(|n| *n == names(id)).map(|r| r as u32);
    let def: Vec<u16> = folder.iter().copied().filter(|&id| rank(DEF, id).is_some()).collect();
    let emotion = |x: &Explorer, seat: usize| {
        x.obs_of(seat).and_then(|o| o.raw).map(|r| {
            let at = |a: u32| r[(a - 0x0200_0000) as usize];
            (at(0x0203_52cc), at(0x0203_ce2c), at(0x0203_ce90))
        })
    };
    let mut tested = std::collections::HashSet::new();
    let mut n = 0;
    for round in 0..24 {
        if def.iter().all(|id| tested.contains(id)) {
            break;
        }
        x.open_chip_select();
        for seat in 0..2 {
            let cs = x.obs_of(seat).and_then(|o| o.chip_select);
            let hand: Vec<String> = cs.as_ref().map_or(vec![], |cs| {
                cs.hand.iter().map(|c| c.map_or("--".into(), |c| names(c.id))).collect()
            });
            writeln!(
                log,
                "{:>6} seat{seat} chip select {} cursor {:?} hand {hand:?}",
                x.tick,
                cs.is_some(),
                cs.map(|c| c.cursor)
            )
            .unwrap();
        }
        // Useful chips first, then anything, so the hand keeps cycling:
        // unpicked chips stay in hand for the next chip select.
        let d = x.pick(1, &|id| {
            Some(if tested.contains(&id) {
                100
            } else {
                rank(DEF, id).unwrap_or(100)
            })
        });
        let a = x.pick(0, &|id| Some(rank(ATK, id).unwrap_or(100)));
        writeln!(
            log,
            "{:>6} round {round}: seat1 {:?} seat0 {:?}",
            x.tick,
            d.iter().map(|&i| names(i)).collect::<Vec<_>>(),
            a.iter().map(|&i| names(i)).collect::<Vec<_>>()
        )
        .unwrap();
        x.confirm_both();
        let queue = x.obs_of(1).map(|o| o.queue).unwrap_or_default();
        // Only the untested defenses, which were picked first; the fillers
        // behind them are never used, so seat 1 never hits back.
        let useful: Vec<u16> = queue
            .into_iter()
            .take_while(|&id| rank(DEF, id).is_some() && !tested.contains(&id))
            .collect();
        for id in useful {
            n += 1;
            let tag = format!("{n:02}_{}", names(id));
            let row = x.shared.lock().unwrap().tiles[1].1;
            x.to_row(0, row);
            x.snapshot_both(&format!("{tag}_pre"));
            x.use_chip(1);
            x.run(60);
            x.snapshot_both(&format!("{tag}_set"));
            x.shot(&format!("{tag}_set"));
            x.run(90);
            x.snapshot_both(&format!("{tag}_up"));
            if let Some(o) = x.obs_of(1) {
                writeln!(log, "{:>6} {tag} up: {:?}", x.tick, o.units[1]).unwrap();
            }
            x.shot(&format!("{tag}_up"));
            let row = x.shared.lock().unwrap().tiles[1].1;
            x.to_row(0, row);
            let next0 = x.obs_of(0).and_then(|o| o.queue.first().copied());
            let with = if next0.is_some_and(|id| rank(ATK, id).is_some()) && x.use_chip(0) {
                "chip"
            } else {
                x.press(&[0], keys::B, 110);
                "charge"
            };
            x.run(90);
            x.snapshot_both(&format!("{tag}_hit"));
            x.shot(&format!("{tag}_hit"));
            let s = x.shared.lock().unwrap();
            writeln!(
                log,
                "{:>6} {tag}: hit with {with}; hp {:?}; emotion bytes (352cc, ce2c, ce90) c0 {:?} c1 {:?}",
                x.tick,
                s.hp,
                emotion(x, 0),
                emotion(x, 1)
            )
            .unwrap();
            drop(s);
            tested.insert(id);
        }
    }
    let missing: Vec<String> = def
        .iter()
        .filter(|id| !tested.contains(id))
        .map(|&id| names(id))
        .collect();
    writeln!(log, "duel done at tick {}; untested {missing:?}", x.tick).unwrap();
}

/// The `roads` command: seat 1 lays each road (GoingRd, ComingRd) in
/// seat 0's row while seat 0 stands still, and the log follows both
/// players' tiles, to see which way a road carries whoever stands on it.
fn roads(x: &mut Explorer, names: &dyn Fn(u16) -> String, log: &mut std::fs::File) {
    const ROADS: &[&str] = &["GoingRd", "ComingRd"];
    let rank = |id: u16| ROADS.iter().position(|n| *n == names(id)).map(|r| r as u32);
    let mut done = std::collections::HashSet::new();
    for round in 0..16 {
        if done.len() == ROADS.len() {
            break;
        }
        x.open_chip_select();
        let picked = x.pick(1, &|id| {
            Some(if done.contains(&id) {
                100
            } else {
                rank(id).unwrap_or(100)
            })
        });
        x.pick(0, &|_| Some(100));
        x.confirm_both();
        writeln!(
            log,
            "{:>6} round {round}: seat1 {:?}",
            x.tick,
            picked.iter().map(|&i| names(i)).collect::<Vec<_>>()
        )
        .unwrap();
        let queue = x.obs_of(1).map(|o| o.queue).unwrap_or_default();
        let Some(&id) = queue.first().filter(|&&id| rank(id).is_some() && !done.contains(&id)) else {
            continue;
        };
        let row = x.shared.lock().unwrap().tiles[0].1;
        x.to_row(1, row);
        x.use_chip(1);
        for step in 0..30 {
            x.run(10);
            let s = x.shared.lock().unwrap();
            let field = x.obs_of(1).map(|o| {
                use tango_match::telemetry::GameDetail;
                o.summary()
                    .lines()
                    .filter(|l| l.starts_with("field"))
                    .collect::<Vec<_>>()
                    .join(" / ")
            });
            writeln!(
                log,
                "{:>6} {} +{:3}: tiles p1 {:?} p2 {:?} {}",
                x.tick,
                names(id),
                step * 10,
                s.tiles[0],
                s.tiles[1],
                field.unwrap_or_default()
            )
            .unwrap();
        }
        x.shot(&format!("road_{}", names(id)));
        done.insert(id);
    }
}

/// The `beastover` command: seat 1 goes Beast Out, then keeps opening the
/// chip screen until Beast Over and beyond, logging its form, emotion,
/// mood, NaviCust readout and HP each turn, and HP every 20 ticks once
/// it's Exhausted.
fn beastover(x: &mut Explorer, log: &mut std::fs::File) {
    use tango_gamesupport_bn6::observe::Form;
    let report = |x: &Explorer, log: &mut std::fs::File, what: &str| {
        if let Some(o) = x.obs_of(1) {
            let u = o.units[1];
            let n = o.navicust;
            writeln!(
                log,
                "{:>6} {what}: form {:?} beast {} emotion {:?} mood {:?} hp {} · navicust atk {} armor {} undershirt {} air {} bugstop {}",
                x.tick, u.form, u.beast_turns_left, u.emotion, u.mood, x.shared.lock().unwrap().hp[1],
                n.attack, n.super_armor, n.undershirt, n.air_shoes, n.bug_stop
            )
            .unwrap();
        }
    };
    // Turn 1: Beast Out (cursor to the star under OK, A), then confirm.
    x.open_chip_select();
    report(x, log, "start");
    for _ in 0..5 {
        x.press(&[1], keys::RIGHT, 3);
    }
    x.press(&[1], keys::DOWN, 3);
    x.press(&[1], keys::A, 3);
    x.confirm_both();
    x.run(120);
    x.confirm_both();
    report(x, log, "after beast out");
    let mut over_turns = 0;
    for turn in 2..14 {
        x.open_chip_select();
        // Stay in Beast Out while it has turns: pick the star again.
        if x.obs_of(1).is_some() {
            for _ in 0..5 {
                x.press(&[1], keys::RIGHT, 3);
            }
            x.press(&[1], keys::DOWN, 3);
            x.press(&[1], keys::A, 3);
        }
        x.confirm_both();
        x.run(120);
        x.confirm_both();
        x.run(60);
        report(x, log, &format!("turn {turn}"));
        x.shot(&format!("turn{turn:02}"));
        if let Some(raw) = x.obs_of(1).and_then(|o| o.raw) {
            let ns = 0x0203_ce64 - 0x0200_0000;
            writeln!(
                log,
                "        navistats transformation {} beast {} | form table {}",
                raw[ns + 0x2c],
                raw[ns + 0x21],
                raw[0x0203_a990 - 0x0200_0000]
            )
            .unwrap();
        }
        if matches!(x.obs_of(1).map(|o| o.units[1].form), Some(Form::BeastOver)) {
            over_turns += 1;
        }
        if over_turns >= 2 {
            break;
        }
    }
    for _ in 0..10 {
        x.run(20);
        report(x, log, "drain");
    }
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
            "beastover" => {
                beastover(&mut x, &mut log);
                continue;
            }
            "roads" => {
                roads(&mut x, &names, &mut log);
                continue;
            }
            "duel" => {
                duel(&mut x, &names, &folder, &mut log);
                continue;
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
