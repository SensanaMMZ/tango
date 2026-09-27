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
    dump: Option<tango_match::telemetry::Detail>,
}

struct Seat1(Arc<Mutex<Shared>>);

impl Opponent for Seat1 {
    fn input(&mut self, view: &View<'_>) -> u32 {
        let mut s = self.0.lock().unwrap();
        if let Some(b) = view.battle {
            s.custom = b.custom;
            s.hp = [b.units[0].hp, b.units[1].hp];
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

    let (session, mut driver, _audio) = tango_session::training::TrainingSession::new(
        game,
        Arc::new(rom),
        sram,
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
