//! Rate-limited, expiring **share rooms** for read-only session sharing.
//!
//! A share room is created when a sharer clicks "share this session": the demo
//! service hands back a room id that viewers join read-only on the relay
//! (`?mode=view`, see `reticle-server`'s `JoinMode`). Unlike an agent session
//! (which the `/submit` path owns), a share room carries no agent loop and no
//! budget; it is purely a relay room id plus an expiry.
//!
//! Two abuse controls apply, mirroring the rest of the demo (ADR 0039):
//!
//! * **Creation is rate-limited per source IP** with the same sliding-window
//!   [`RateLimiter`] the submit path uses, so one client
//!   cannot mint an unbounded number of rooms.
//! * **Rooms expire.** Each room is stamped with a creation instant and a TTL;
//!   [`ShareRooms::create`] sweeps expired rooms first, and [`ShareRooms::is_live`]
//!   reports whether a room is still within its TTL. Expiry bounds how long a demo
//!   deployment retains any shared session, with no accounts and no manual cleanup.
//!
//! The registry is time-injectable ([`ShareRooms::create_at`] / [`is_live_at`])
//! so the rate-limit and TTL behaviour is tested without sleeping, exactly like
//! the [`RateLimiter`] itself.
//!
//! [`is_live_at`]: ShareRooms::is_live_at

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::rate::RateLimiter;

/// The abuse limits governing share-room creation.
///
/// Deliberately separate from [`LimitConfig`](crate::LimitConfig): share rooms are
/// a different resource from agent sessions (no token or command budget applies),
/// and keeping them apart avoids widening the frozen submit-path contract.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ShareLimits {
    /// Maximum share rooms a single source IP may create per minute.
    pub per_ip_create_per_min: u32,
    /// How long a share room stays live before it expires.
    pub room_ttl: Duration,
    /// Hard ceiling on the number of live share rooms tracked at once, so an
    /// unexpired backlog cannot grow without bound.
    pub max_live_rooms: usize,
}

impl Default for ShareLimits {
    fn default() -> Self {
        Self {
            // A share is a deliberate click, so a handful a minute is plenty and a
            // script cannot mint rooms in a tight loop.
            per_ip_create_per_min: 6,
            // Demo shares are ephemeral: half an hour is long enough to show a
            // colleague and short enough that nothing lingers.
            room_ttl: Duration::from_secs(30 * 60),
            // Bound total retained rooms regardless of TTL.
            max_live_rooms: 256,
        }
    }
}

/// The reason a share-room creation was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShareRejection {
    /// The source IP has created too many rooms in the current window.
    RateLimited,
    /// The server is already tracking [`ShareLimits::max_live_rooms`] live rooms.
    AtCapacity,
}

/// One tracked share room: its id and when it expires.
#[derive(Clone, Debug)]
struct ShareRoom {
    id: String,
    expires_at: Instant,
}

/// A registry of live share rooms, with per-IP creation rate-limiting and TTL
/// expiry.
///
/// Cloning is not provided; the demo server holds one behind an `Arc`. All state
/// is behind a `Mutex` (short, synchronous critical sections) plus the rate
/// limiter's own lock.
#[derive(Debug)]
pub struct ShareRooms {
    limits: ShareLimits,
    rate: RateLimiter,
    rooms: Mutex<Vec<ShareRoom>>,
    next_id: AtomicU64,
}

impl ShareRooms {
    /// Builds a share-room registry enforcing `limits`.
    #[must_use]
    pub fn new(limits: ShareLimits) -> Self {
        let rate = RateLimiter::new(limits.per_ip_create_per_min, Duration::from_secs(60));
        Self {
            limits,
            rate,
            rooms: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// The limits in force.
    #[must_use]
    pub fn limits(&self) -> ShareLimits {
        self.limits
    }

    /// Creates a share room for `ip` at the real clock, or returns why it was
    /// refused. See [`ShareRooms::create_at`].
    ///
    /// # Errors
    ///
    /// Returns [`ShareRejection::RateLimited`] if the IP is over its creation rate,
    /// or [`ShareRejection::AtCapacity`] if the live-room ceiling is reached.
    pub fn create(&self, ip: &str) -> Result<String, ShareRejection> {
        self.create_at(ip, Instant::now())
    }

    /// Creates a share room for `ip` as of instant `now`, sweeping expired rooms
    /// first.
    ///
    /// The order matters: expiry is swept before the capacity check, so rooms that
    /// have aged out free capacity for a new one. The rate limit is checked first
    /// of all, so a flood is rejected before any room bookkeeping happens (and a
    /// rejected create does not itself count a room).
    ///
    /// # Errors
    ///
    /// Returns [`ShareRejection::RateLimited`] or [`ShareRejection::AtCapacity`] as
    /// [`ShareRooms::create`] describes.
    pub fn create_at(&self, ip: &str, now: Instant) -> Result<String, ShareRejection> {
        // 1. Per-IP creation rate. A rejected create records nothing.
        if !self.rate.check_at(ip, now) {
            return Err(ShareRejection::RateLimited);
        }

        let mut rooms = self.rooms.lock().expect("share rooms mutex poisoned");
        // 2. Drop expired rooms so they free capacity and never linger.
        rooms.retain(|room| room.expires_at > now);
        // 3. Capacity ceiling on live rooms.
        if rooms.len() >= self.limits.max_live_rooms {
            return Err(ShareRejection::AtCapacity);
        }

        // 4. Mint and record the room.
        let n = self.next_id.fetch_add(1, Ordering::Relaxed);
        let id = format!("share-{n:08x}");
        rooms.push(ShareRoom {
            id: id.clone(),
            expires_at: now + self.limits.room_ttl,
        });
        Ok(id)
    }

    /// Whether `room` is a tracked share room still within its TTL, at the real
    /// clock. See [`ShareRooms::is_live_at`].
    #[must_use]
    pub fn is_live(&self, room: &str) -> bool {
        self.is_live_at(room, Instant::now())
    }

    /// Whether `room` is tracked and unexpired as of instant `now`.
    ///
    /// Also sweeps expired rooms as a side effect, so a viewer probing a room keeps
    /// the registry tidy.
    #[must_use]
    pub fn is_live_at(&self, room: &str, now: Instant) -> bool {
        let mut rooms = self.rooms.lock().expect("share rooms mutex poisoned");
        rooms.retain(|r| r.expires_at > now);
        rooms.iter().any(|r| r.id == room)
    }

    /// The number of live (unexpired) share rooms as of instant `now`, sweeping
    /// expired rooms first. Exposed for assertions.
    #[must_use]
    pub fn live_count_at(&self, now: Instant) -> usize {
        let mut rooms = self.rooms.lock().expect("share rooms mutex poisoned");
        rooms.retain(|r| r.expires_at > now);
        rooms.len()
    }
}

#[cfg(test)]
mod tests {
    use super::{ShareLimits, ShareRejection, ShareRooms};
    use std::time::{Duration, Instant};

    fn small_limits() -> ShareLimits {
        ShareLimits {
            per_ip_create_per_min: 3,
            room_ttl: Duration::from_secs(60),
            max_live_rooms: 4,
        }
    }

    #[test]
    fn creates_unique_room_ids() {
        let share = ShareRooms::new(small_limits());
        let t0 = Instant::now();
        let a = share.create_at("ip", t0).expect("first room");
        let b = share.create_at("ip", t0).expect("second room");
        assert_ne!(a, b, "each share room gets a distinct id");
        assert!(a.starts_with("share-") && b.starts_with("share-"));
        assert!(share.is_live_at(&a, t0));
        assert!(share.is_live_at(&b, t0));
    }

    #[test]
    fn creation_is_rate_limited_per_ip() {
        let share = ShareRooms::new(small_limits());
        let t0 = Instant::now();
        // Three creations from one IP are allowed.
        for _ in 0..3 {
            assert!(share.create_at("flooder", t0).is_ok());
        }
        // The fourth in the window is refused as rate-limited.
        assert_eq!(
            share.create_at("flooder", t0),
            Err(ShareRejection::RateLimited)
        );
        // A different IP has its own budget.
        assert!(share.create_at("other", t0).is_ok());
    }

    #[test]
    fn rooms_expire_after_their_ttl() {
        let share = ShareRooms::new(small_limits());
        let t0 = Instant::now();
        let room = share.create_at("ip", t0).expect("room");
        // Live right up to the TTL boundary.
        assert!(share.is_live_at(&room, t0 + Duration::from_secs(59)));
        // Past the TTL it is gone.
        assert!(!share.is_live_at(&room, t0 + Duration::from_secs(61)));
        assert_eq!(share.live_count_at(t0 + Duration::from_secs(61)), 0);
    }

    #[test]
    fn expiry_frees_capacity() {
        // A registry that only holds one live room at a time.
        let limits = ShareLimits {
            per_ip_create_per_min: 100,
            room_ttl: Duration::from_secs(60),
            max_live_rooms: 1,
        };
        let share = ShareRooms::new(limits);
        let t0 = Instant::now();
        let _first = share.create_at("ip", t0).expect("first room fits");
        // A second room while the first is live exceeds the ceiling.
        assert_eq!(share.create_at("ip", t0), Err(ShareRejection::AtCapacity));
        // Once the first expires, capacity frees up and a new room fits.
        let later = t0 + Duration::from_secs(61);
        assert!(
            share.create_at("ip", later).is_ok(),
            "an expired room should free capacity"
        );
    }

    #[test]
    fn unknown_room_is_not_live() {
        let share = ShareRooms::new(small_limits());
        assert!(!share.is_live_at("share-deadbeef", Instant::now()));
    }
}
