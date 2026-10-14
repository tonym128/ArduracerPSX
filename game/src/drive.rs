//! Which subsystem currently owns the CD drive.
//!
//! One drive, one head. Two things need it -- the XA music stream, which wants
//! it continuously, and the texture fetcher, which wants it in bursts -- and
//! neither can simply ask for it, because the other's stream would desynchronise
//! underneath it.
//!
//! # The rule
//!
//! The side that wants the drive *continuously* holds the stream, and the side
//! that wants it in *bursts* interrupts. That is not a preference; it follows
//! from where the cost sits. An interruption costs whoever is holding the drive
//! one ~150 ms seek, so the cost of a design is (interruptions) x (seek):
//!
//! - XA as a run queue pays a seek per run, and a run is a few sectors, so it
//!   pays ~3x the drive for the audio it carries. Measured, and asserted in
//!   `arduracer_core::xa`.
//! - XA as a continuous stream pays a seek per *interruption*, and textures
//!   interrupt far less often than XA would seek. The seek amortises over every
//!   sector in between, which is the whole difference.
//!
//! So textures preempt XA rather than the reverse. Preemption is not free -- XA
//! must re-seek to the sector it had reached -- so `suspend`/`resume` carry that
//! cursor, and losing it would turn every texture fetch into an audible restart
//! of the music from the top of the track.

/// Which subsystem holds the drive.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Owner {
    /// Nobody. The drive is idle and may be claimed.
    Idle,
    /// The XA music stream, continuously.
    Xa,
    /// A texture fetch run, in bursts.
    Texture,
}

impl Owner {
    pub const fn label(self) -> &'static str {
        match self {
            Owner::Idle => "idle",
            Owner::Xa => "xa",
            Owner::Texture => "texture",
        }
    }
}

static mut OWNER: Owner = Owner::Idle;

/// Preemption hook, installed by the XA player.
///
/// Called when a texture run needs the drive while XA holds it, before the
/// ownership changes. It must stop the drive stream and remember the sector it
/// had reached, so [`release`](Owner) can hand it back in the right place.
static mut PREEMPT: Option<fn()> = None;

/// Installs the hook called when XA is preempted. Called once during init.
pub fn install_preempt(f: fn()) {
    unsafe { PREEMPT = Some(f) };
}

/// Who holds the drive.
pub fn owner() -> Owner {
    unsafe { core::ptr::read_volatile(core::ptr::addr_of!(OWNER)) }
}

/// Takes the drive for `want`, preempting XA if that is who holds it.
///
/// Returns false only when another *burst* owner already holds it, which cannot
/// happen with two callers but is checked rather than assumed -- a lost claim
/// would mean two streams writing sectors we both believe we own.
pub fn claim(want: Owner) -> bool {
    let held = owner();
    if held == want {
        return true;
    }
    match held {
        Owner::Idle => {
            unsafe { OWNER = want };
            true
        }
        Owner::Xa => {
            // Music yields to data. The music is the one that can be
            // reconstructed from a remembered cursor; a half-written texture
            // run cannot.
            if let Some(f) = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(PREEMPT)) } {
                f();
            }
            unsafe { OWNER = want };
            true
        }
        Owner::Texture => false,
    }
}

/// Gives the drive back. The XA player reopens its stream on its next frame.
pub fn release(who: Owner) {
    if owner() == who {
        unsafe { OWNER = Owner::Idle };
    }
}

/// Test and diagnostic reset. Not used by the game.
pub fn reset() {
    unsafe {
        OWNER = Owner::Idle;
        PREEMPT = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claiming_an_idle_drive_succeeds() {
        reset();
        assert_eq!(owner(), Owner::Idle);
        assert!(claim(Owner::Texture));
        assert_eq!(owner(), Owner::Texture);
    }

    #[test]
    fn a_holder_keeps_the_drive() {
        reset();
        assert!(claim(Owner::Xa));
        assert!(claim(Owner::Xa), "re-claiming must be idempotent");
        assert_eq!(owner(), Owner::Xa);
    }

    #[test]
    fn textures_preempt_xa_and_run_the_hook() {
        use core::sync::atomic::{AtomicUsize, Ordering};
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        fn hook() {
            CALLS.fetch_add(1, Ordering::Relaxed);
        }

        reset();
        CALLS.store(0, Ordering::Relaxed);
        install_preempt(hook);
        assert!(claim(Owner::Xa));
        assert!(claim(Owner::Texture), "textures must preempt music");
        assert_eq!(
            CALLS.load(Ordering::Relaxed),
            1,
            "preempt hook must run once"
        );
        assert_eq!(owner(), Owner::Texture);
    }

    #[test]
    fn release_hands_the_drive_back() {
        reset();
        assert!(claim(Owner::Texture));
        release(Owner::Texture);
        assert_eq!(owner(), Owner::Idle);
        // Releasing when you do not hold it must not steal it from someone who
        // does -- the XA player releasing late must not blank a texture run.
        assert!(claim(Owner::Texture));
        release(Owner::Xa);
        assert_eq!(owner(), Owner::Texture, "a non-holder released the drive");
    }
}
