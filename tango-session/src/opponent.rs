//! Who sits in training's second seat: whatever supplies the
//! non-player core's input, one tick at a time.
//!
//! An [`Opponent`] is asked for a pad word before every tick and handed
//! a [`View`] of the battle to decide it from. The view is built from
//! the pair's confirmed telemetry, which is **deliberately only what a
//! human in that seat could see** — both units' HP and tiles, whose
//! custom screens stand open, the chip-use and round events. Game RAM
//! holds far more (the other side's hand, its folder order); an opponent
//! that read it would be cheating, and it gets no way to.
//!
//! The view trails the input it informs by one tick: the readings come
//! out of the tick that just ran, and the input goes into the next.
//! That's the tightest loop a local lockstep pair allows, and faster
//! than any human reaction — slower opponents add their own delay.

use rand::{Rng, SeedableRng};
use tango_match::keys;
use tango_match::telemetry::{BattleObs, Event};

/// What an [`Opponent`] may look at before choosing a tick's input.
pub struct View<'a> {
    /// The seat (absolute player, 0 or 1) the opponent drives this tick.
    /// Follows the player's side swap, so an opponent must not cache it.
    pub seat: usize,
    /// The latest confirmed battle reading, or `None` while there's no
    /// live, undecided round to read (intros, result screens, menus).
    /// Its `detail` holds only `detail[seat]`, this seat's own core's
    /// game-specific readings; the other side's is removed before an
    /// opponent sees it, since it may carry that player's hand.
    pub battle: Option<&'a BattleObs>,
    /// Events confirmed by the tick that just ran, tick-stamped.
    pub events: &'a [(u32, Event)],
}

/// Supplies one seat's input, a tick at a time. Called on the session's
/// drive thread, once per tick, before the tick advances.
pub trait Opponent: Send {
    /// The pad word ([`keys`] bits) to hold for the next tick. Games act
    /// on press *edges*, so a held bit is one press: to press a button
    /// twice, release it in between.
    fn input(&mut self, view: &View<'_>) -> u32;
}

/// The opponents a host can pick between. A plain value so it can cross
/// threads as an atomic and the driver can build the opponent itself.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[repr(u8)]
pub enum Kind {
    /// Presses nothing: training's original stand-there dummy.
    #[default]
    Dummy = 0,
    /// Mashes plausible inputs at random. Proves the plumbing; not a
    /// fair fight.
    Random = 1,
}

impl Kind {
    pub(crate) fn from_u8(v: u8) -> Self {
        match v {
            1 => Kind::Random,
            _ => Kind::Dummy,
        }
    }

    /// Build a fresh opponent of this kind. `seed` makes a random
    /// opponent's choices reproducible.
    pub fn build(self, seed: [u8; 16]) -> Box<dyn Opponent> {
        match self {
            Kind::Dummy => Box::new(Dummy),
            Kind::Random => Box::new(Random::new(seed)),
        }
    }
}

/// Presses nothing, ever.
pub struct Dummy;

impl Opponent for Dummy {
    fn input(&mut self, _view: &View<'_>) -> u32 {
        0
    }
}

/// How long a tap is held. A game samples the pad once a frame, so a
/// single tick would do — a few give the press some margin without
/// reading as a hold.
const TAP_TICKS: u32 = 3;

/// A charge shot's hold, in ticks: from about one second, enough for
/// the buster to charge, to two.
const CHARGE_TICKS: std::ops::RangeInclusive<u32> = 60..=120;

/// Picks a gesture at random, holds it for its length, then lets go.
///
/// In battle: step a tile, tap the buster, charge and release it, use
/// the selected chip, tap L/R to open the custom screen (which only
/// opens once the gauge is full — the game ignores it before), or wait.
/// In the custom screen: pick a few chips (A, moving the cursor along
/// the chip row between picks), then START — which jumps the cursor to
/// OK — then A to confirm. The order matters: pressed at random, START
/// and A confirm an empty hand as often as not. A link battle waits at the custom screen until BOTH sides
/// confirm, so an opponent that never gets there stalls the match.
/// Every gesture ends in a release, so each one is a fresh press.
///
/// The custom-screen controls were found with the `custom_probe`
/// example on BN6: A alone, START alone, L, R or SELECT never close the
/// screen; A → START → A does, as does walking the cursor right onto
/// OK.
pub struct Random {
    rng: rand_pcg::Mcg128Xsl64,
    /// The pad word being held, and for how many more ticks.
    held: u32,
    ticks_left: u32,
    /// Whether this seat's custom screen was open last tick, to catch
    /// each opening.
    custom_was_open: bool,
    /// Chip picks (A presses) still to make before confirming. An A
    /// can fail — a chip whose code doesn't fit — so this counts
    /// attempts, not chips.
    picks_left: u32,
    /// START has parked the cursor on OK; the next A confirms.
    on_ok: bool,
}

impl Random {
    pub fn new(seed: [u8; 16]) -> Self {
        Self {
            rng: rand_pcg::Mcg128Xsl64::from_seed(seed),
            held: 0,
            ticks_left: 0,
            custom_was_open: false,
            picks_left: 0,
            on_ok: false,
        }
    }

    fn direction(&mut self) -> u32 {
        [keys::UP, keys::DOWN, keys::LEFT, keys::RIGHT][self.rng.gen_range(0..4)]
    }

    /// The next gesture as `(pad word, ticks to hold it)`.
    fn next_gesture(&mut self, custom_open: bool) -> (u32, u32) {
        if self.held != 0 {
            // Let go after every press, so the next one is an edge.
            return (0, self.rng.gen_range(2..=6));
        }
        if custom_open {
            let bit = if self.picks_left > 0 {
                // Pick the chip under the cursor, or move along the row
                // to another. Only left and right: up and down leave the
                // chip row for the cross/beast picks.
                match self.rng.gen_range(0..100) {
                    0..=59 => {
                        self.picks_left -= 1;
                        keys::A
                    }
                    60..=79 => keys::LEFT,
                    _ => keys::RIGHT,
                }
            } else if !self.on_ok {
                self.on_ok = true;
                keys::START
            } else {
                // Confirm. Should the screen still be open afterwards,
                // this cycles back through START, so it can't get stuck.
                self.on_ok = false;
                keys::A
            };
            return (bit, TAP_TICKS);
        }
        match self.rng.gen_range(0..100) {
            0..=34 => (self.direction(), TAP_TICKS),
            35..=59 => (keys::B, TAP_TICKS),
            60..=74 => (keys::B, self.rng.gen_range(CHARGE_TICKS)),
            75..=84 => (keys::A, TAP_TICKS),
            85..=89 => ([keys::L, keys::R][self.rng.gen_range(0..2)], TAP_TICKS),
            _ => (0, self.rng.gen_range(10..=40)),
        }
    }
}

impl Opponent for Random {
    fn input(&mut self, view: &View<'_>) -> u32 {
        let Some(battle) = view.battle else {
            // Nothing to fight: let go, and start fresh when the battle
            // comes back.
            self.held = 0;
            self.ticks_left = 0;
            return 0;
        };
        let custom_open = battle.custom[view.seat];
        if custom_open && !self.custom_was_open {
            // A fresh chip select: plan this hand's picks.
            self.picks_left = self.rng.gen_range(1..=5);
            self.on_ok = false;
        }
        self.custom_was_open = custom_open;
        if self.ticks_left == 0 {
            (self.held, self.ticks_left) = self.next_gesture(custom_open);
        }
        self.ticks_left -= 1;
        self.held
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tango_match::telemetry::UnitObs;

    fn battle(custom: bool) -> BattleObs {
        let unit = UnitObs { hp: 500, tile: (2, 2) };
        BattleObs {
            units: [unit, UnitObs { tile: (5, 2), ..unit }],
            custom: [false, custom],
            detail: [None, None],
        }
    }

    /// Run `opponent` from seat 1 for `ticks` ticks over a fixed reading.
    fn run(opponent: &mut dyn Opponent, obs: Option<&BattleObs>, ticks: usize) -> Vec<u32> {
        (0..ticks)
            .map(|_| {
                opponent.input(&View {
                    seat: 1,
                    battle: obs,
                    events: &[],
                })
            })
            .collect()
    }

    #[test]
    fn the_dummy_presses_nothing() {
        let obs = battle(false);
        assert!(run(&mut Dummy, Some(&obs), 1000).iter().all(|&k| k == 0));
    }

    #[test]
    fn random_is_idle_without_a_battle() {
        let mut r = Random::new([7; 16]);
        let obs = battle(false);
        // Mid-gesture when the battle goes away: it must let go at once.
        run(&mut r, Some(&obs), 50);
        assert!(run(&mut r, None, 100).iter().all(|&k| k == 0));
    }

    #[test]
    fn random_is_reproducible_from_its_seed() {
        let obs = battle(false);
        let a = run(&mut Random::new([3; 16]), Some(&obs), 2000);
        let b = run(&mut Random::new([3; 16]), Some(&obs), 2000);
        assert_eq!(a, b);
    }

    #[test]
    fn random_fights_with_edges() {
        let obs = battle(false);
        let inputs = run(&mut Random::new([1; 16]), Some(&obs), 5000);
        for bit in [
            keys::A,
            keys::B,
            keys::UP,
            keys::DOWN,
            keys::LEFT,
            keys::RIGHT,
            keys::L,
            keys::R,
        ] {
            assert!(inputs.iter().any(|&k| k & bit != 0), "never pressed {bit:#x}");
        }
        // Nothing is held longer than a full charge, and every press is
        // released before the next — one gesture's bits at a time.
        let mut run_len = 0;
        for w in inputs.windows(2) {
            run_len = if w[1] != 0 && w[1] == w[0] { run_len + 1 } else { 0 };
            assert!(run_len < *CHARGE_TICKS.end());
            assert!(
                w[0] == 0 || w[1] == 0 || w[0] == w[1],
                "no release between {:#x} and {:#x}",
                w[0],
                w[1]
            );
        }
    }

    #[test]
    fn random_picks_chips_before_confirming() {
        // Many openings, each from a fresh screen: START (the move to
        // OK) must always follow at least one pick.
        for seed in 0..50u8 {
            let mut r = Random::new([seed; 16]);
            // A tick in battle first, so the next is a fresh opening.
            run(&mut r, Some(&battle(false)), 1);
            let inputs = run(&mut r, Some(&battle(true)), 600);
            let start = inputs.iter().position(|&k| k == keys::START).expect("reaches START");
            assert!(inputs[..start].contains(&keys::A), "seed {seed}: START before any pick");
        }
    }

    #[test]
    fn random_only_navigates_the_custom_screen() {
        let obs = battle(true);
        let inputs = run(&mut Random::new([9; 16]), Some(&obs), 5000);
        // What closes the screen: a pick, START onto OK, and A.
        assert!(inputs.iter().any(|&k| k == keys::A));
        assert!(inputs.iter().any(|&k| k == keys::START));
        let allowed = keys::A | keys::START | keys::LEFT | keys::RIGHT;
        assert!(inputs.iter().all(|&k| k & !allowed == 0));
    }
}
