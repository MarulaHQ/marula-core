//! Bounded admission for the memory-hard password hash (Wave 56 lane
//! `p56-auth-bounds`).
//!
//! Why this exists. `POST /api/auth/login` and `POST /api/auth/register` both
//! run argon2id over input nobody has authenticated yet. The audit recorded
//! two faces of the same hole
//! (`core-api/auth/login-argon2-holds-pool-connection-across-hash-no-concurrency-bound`
//! and `.../register-argon2-ungated-anonymous-no-concurrency-bound`): the
//! durable per-IP/per-actor gate is a SEQUENTIAL attempt counter — the lock
//! row is only written after a failed hash — so a burst of concurrent
//! requests all passes the pre-check while no lock exists, and each one
//! spends ~19 MiB and tens of milliseconds of CPU on work it never pays for.
//! Register had no gate at all. With no concurrency bound, an unauthenticated
//! caller decides how much hash work this process does, and (before the login
//! split landed with this module) held a pooled DB connection across it.
//!
//! The answer is a bound, not a bigger counter: one semaphore per process,
//! taken before the hash and released after it, with a short queue wait and a
//! typed 503 for a caller that cannot get a slot. A refused caller learns
//! "come back in a moment", never a 500 and never an unbounded wait.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use tokio::sync::{Semaphore, SemaphorePermit};

/// Hash slots per available core. Two, not one and not eight.
///
/// Argon2id is memory-bandwidth bound, not arithmetic bound: a single verify
/// leaves the core's load/store units idle while it walks its 19 MiB of
/// scratch memory, so one in-flight hash per core under-uses the machine. The
/// second slot gives the CPU something to run during those stalls and hides
/// scheduler and page-fault jitter. Beyond two the extra slots buy no
/// throughput (there is no idle bus to fill) and cost memory linearly —
/// every in-flight slot is another 19 MiB resident, and every additional slot
/// is another request the pool was sized for. Two per core is where the
/// throughput curve flattens; it is a constant so the choice is arguable in
/// one place instead of appearing as a magic number in the handler.
pub const DEFAULT_SLOTS_PER_CORE: usize = 2;

/// Floor for [`default_slots`], so a single-core box still hashes two at a
/// time instead of serialising every login behind one.
const MIN_DEFAULT_SLOTS: usize = 2;

/// Default queue wait for a slot, in milliseconds. Long enough to absorb a
/// normal burst (a handful of real users signing in together), short enough
/// that a client is not parked on a request it could retry: 250 ms is roughly
/// one argon2 verify on a small core, so a caller that has to wait longer than
/// one hash is already in the territory where refusing is the honest answer.
pub const DEFAULT_QUEUE_WAIT_MS: u64 = 250;

/// The `Retry-After` on a refusal, in seconds. One second is the shortest
/// value HTTP allows to be useful and matches the granularity of the durable
/// backoff (`app_auth_backoff_secs(1)` starts at 30 s, so a retry a second
/// later still meets a cheap gate check rather than a lock).
pub const RETRY_AFTER_SECS: u64 = 2;

/// The process default: [`DEFAULT_SLOTS_PER_CORE`] per available core.
///
/// `available_parallelism` reports the cores this process may actually run
/// on (cgroup quota included, unlike `num_cpus`' raw count), which is the
/// number the semaphore is supposed to track.
pub fn default_slots() -> usize {
    std::thread::available_parallelism()
        .map(|cores| cores.get())
        .unwrap_or(1)
        .saturating_mul(DEFAULT_SLOTS_PER_CORE)
        .max(MIN_DEFAULT_SLOTS)
}

/// A slot was not available in time. Carries what the caller needs to answer
/// `503` honestly: how long to wait, nothing about internal capacity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refused {
    /// Seconds for the `Retry-After` header.
    pub retry_after_secs: u64,
}

/// The process-wide hash admission bound.
///
/// `slots` is fixed at construction: resizing a bound while requests are
/// queued on it produces a burst the size of the resize, which is the thing
/// the bound exists to prevent. The size comes from config at first use and
/// does not move.
#[derive(Debug)]
pub struct HashGate {
    slots: usize,
    queue_wait: Duration,
    permits: Semaphore,
}

impl HashGate {
    /// A gate with `slots` concurrent hashes and a `queue_wait` deadline.
    ///
    /// A zero `slots` is coerced to one: a gate that admits nothing turns
    /// every login into a 503, which is a worse outage than the flood it
    /// guards against. Zero `queue_wait` is meaningful and means "refuse
    /// immediately rather than queue", which is what a saturation test wants.
    pub fn new(slots: usize, queue_wait: Duration) -> Self {
        let slots = slots.max(1);
        Self {
            slots,
            queue_wait,
            permits: Semaphore::new(slots),
        }
    }

    /// How many hashes may run at once.
    pub fn slots(&self) -> usize {
        self.slots
    }

    /// How long a caller waits for a free slot before being refused.
    pub fn queue_wait(&self) -> Duration {
        self.queue_wait
    }

    /// Slots currently free. Test-facing and cheap.
    pub fn available(&self) -> usize {
        self.permits.available_permits()
    }

    /// Takes a slot, waiting at most [`Self::queue_wait`].
    ///
    /// The fast path is a non-blocking acquire, so an unloaded process pays
    /// no timer. Under load a caller waits one short deadline and is then
    /// refused: the queue is bounded by the wait, never by the burst.
    ///
    /// The permit is the proof that hashing may proceed. Dropping it (or
    /// letting it fall out of scope) returns the slot, which is why the
    /// handlers drop it explicitly the moment the hash returns rather than
    /// carrying it through the rest of the request.
    pub async fn admit(&self) -> Result<SemaphorePermit<'_>, Refused> {
        if let Ok(permit) = self.permits.try_acquire() {
            return Ok(permit);
        }
        match tokio::time::timeout(self.queue_wait, self.permits.acquire()).await {
            Ok(Ok(permit)) => Ok(permit),
            // The semaphore is never closed — nothing in this process shuts
            // it down — but if it ever were, refusing is the safe direction.
            Ok(Err(_closed)) => Err(self.refused()),
            Err(_elapsed) => Err(self.refused()),
        }
    }

    fn refused(&self) -> Refused {
        REFUSALS.fetch_add(1, Ordering::Relaxed);
        warn_refusal(self.slots);
        Refused {
            retry_after_secs: RETRY_AFTER_SECS,
        }
    }
}

/// The one gate this process has.
///
/// `OnceLock` rather than a field on `AppState` for two reasons: the ruling
/// is one bound per PROCESS (two states in one process — two tests, an
/// all-in-one worker — must not each get their own pool of slots), and the
/// bound has to be visible to handlers without threading a new type through
/// every state constructor.
///
/// Sizing is first-caller-wins. In a running API the first request to reach a
/// password path installs it, and every later caller passes the same config
/// values, so the lock's size is the boot config either way. Tests that want
/// a specific size call this BEFORE driving a request.
pub fn global(slots: usize, queue_wait: Duration) -> &'static HashGate {
    GATE.get_or_init(|| HashGate::new(slots, queue_wait))
}

/// The installed gate, if a password path has been reached yet.
pub fn installed() -> Option<&'static HashGate> {
    GATE.get()
}

static GATE: OnceLock<HashGate> = OnceLock::new();

/// Refusals since process start. The signal an operator graphs: a climbing
/// counter with flat CPU means the bound is doing its job under a flood.
static REFUSALS: AtomicU64 = AtomicU64::new(0);

/// Refusals since process start.
pub fn refusal_count() -> u64 {
    REFUSALS.load(Ordering::Relaxed)
}

/// Process start, so the warn gate reads a monotonic clock without a mutex.
static START: OnceLock<Instant> = OnceLock::new();

/// When the last refusal warning was emitted, in milliseconds since
/// [`START`]. Zero means "never warned".
static LAST_WARN_MS: AtomicU64 = AtomicU64::new(0);

/// Minimum spacing between refusal warnings.
///
/// A refusal is per request, and a flood is exactly when refusals happen, so
/// logging each one would turn the defence into a log-amplification vector:
/// the caller would choose how many lines this process writes per second. One
/// line every couple of seconds keeps the condition visible (with the
/// lifetime counter for the magnitude) without letting the flood write the
/// log.
const WARN_INTERVAL: Duration = Duration::from_secs(2);

fn warn_refusal(slots: usize) {
    let start = START.get_or_init(Instant::now);
    let now_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    let last = LAST_WARN_MS.load(Ordering::Relaxed);
    if last != 0 && now_ms.saturating_sub(last) < WARN_INTERVAL.as_millis() as u64 {
        return;
    }
    // Stored at least 1 so the "never warned" sentinel is not re-used. A
    // benign race: two threads may both pass the check in the same instant
    // and write two lines. Two lines per interval is not a flood.
    LAST_WARN_MS.store(now_ms.max(1), Ordering::Relaxed);
    tracing::warn!(
        slots,
        refusals = refusal_count(),
        "password hash slots exhausted; answering 503 with Retry-After"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate(slots: usize, wait_ms: u64) -> HashGate {
        HashGate::new(slots, Duration::from_millis(wait_ms))
    }

    /// The bound holds: with N slots, N+1 simultaneous acquisitions with a
    /// zero wait leave the last one refused instead of queueing forever.
    #[tokio::test]
    async fn n_plus_one_acquisitions_refuse_at_the_bound() {
        const SLOTS: usize = 3;
        let gate = gate(SLOTS, 0);
        let mut held = Vec::new();
        for slot in 0..SLOTS {
            held.push(
                gate.admit()
                    .await
                    .unwrap_or_else(|_| panic!("slot {slot} of {SLOTS} must be admitted")),
            );
        }
        assert_eq!(gate.available(), 0);
        let refused = gate.admit().await.expect_err("N+1 must be refused");
        assert_eq!(refused.retry_after_secs, RETRY_AFTER_SECS);
        assert_eq!(gate.available(), 0, "a refusal must not consume a slot");
    }

    /// Dropping a permit returns exactly one slot, and the next caller in is
    /// admitted — the release-after-the-hash contract, at the primitive.
    #[tokio::test]
    async fn releasing_one_slot_admits_the_next_caller() {
        let gate = gate(1, 0);
        let permit = gate.admit().await.expect("first caller admitted");
        assert!(gate.admit().await.is_err(), "second caller refused");
        drop(permit);
        assert_eq!(gate.available(), 1);
        let _permit = gate
            .admit()
            .await
            .expect("slot released for the next caller");
    }

    /// A caller that waits gets the slot as soon as one frees, without being
    /// refused. This is the path a normal burst takes.
    #[tokio::test]
    async fn a_waiter_admitted_within_the_deadline_is_not_refused() {
        let gate = HashGate::new(1, Duration::from_millis(200));
        let permit = gate.admit().await.expect("first caller");
        // The slot is released while the second caller is already waiting on
        // the deadline; both futures run on one task so the waiter cannot be
        // starved by the release.
        let release = async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            drop(permit);
        };
        let (admitted, ()) = tokio::join!(gate.admit(), release);
        assert!(
            admitted.is_ok(),
            "a waiter inside the deadline must be admitted, not refused"
        );
    }

    /// A waiter that cannot get a slot inside the deadline is refused, and
    /// the refusal arrives at about the deadline — not at the caller's
    /// patience.
    #[tokio::test]
    async fn a_waiter_past_the_deadline_is_refused() {
        let gate = gate(1, 30);
        let _permit = gate.admit().await.expect("first caller");
        let started = Instant::now();
        let refused = gate.admit().await.expect_err("no slot inside the deadline");
        let waited = started.elapsed();
        assert_eq!(refused.retry_after_secs, RETRY_AFTER_SECS);
        assert!(
            waited >= Duration::from_millis(25),
            "refusal must wait the deadline, waited {waited:?}"
        );
        assert!(
            waited < Duration::from_secs(2),
            "refusal must not wait like an unbounded queue, waited {waited:?}"
        );
    }

    /// A zero-slot gate still admits one: a bound of zero would be an outage
    /// dressed as a defence.
    #[tokio::test]
    async fn zero_slots_is_coerced_to_one() {
        let gate = gate(0, 0);
        assert_eq!(gate.slots(), 1);
        let permit = gate.admit().await.expect("one slot survives a zero config");
        drop(permit);
    }

    /// Sizing is cores × the per-core constant, never below the floor.
    #[test]
    fn default_slots_track_the_host_and_keep_a_floor() {
        let expected = std::thread::available_parallelism()
            .map(|cores| cores.get() * DEFAULT_SLOTS_PER_CORE)
            .unwrap_or(2)
            .max(MIN_DEFAULT_SLOTS);
        assert_eq!(default_slots(), expected);
        assert!(default_slots() >= MIN_DEFAULT_SLOTS);
    }

    /// The global gate is ONE object: a second call with different arguments
    /// must not install a second bound, or a process would end up with two
    /// independent slot pools and twice the hash work.
    #[test]
    fn the_global_gate_is_installed_once() {
        let first = global(5, Duration::from_millis(11));
        let second = global(99, Duration::from_millis(999));
        assert!(std::ptr::eq(first, second));
        assert_eq!(first.slots(), 5);
        assert_eq!(first.queue_wait(), Duration::from_millis(11));
    }

    // --- measurement (ignored) -------------------------------------------

    /// What the bound costs and what it buys, against the REAL argon2, with
    /// no database in the way.
    ///
    ///     cargo test -p core-api --lib auth::hash_gate -- --ignored --nocapture
    ///
    /// Each arm runs TWO bursts — one the size of the bound, one three times
    /// it — because the question is not "is a burst slow" but "does the
    /// latency of a burst grow with the burst". Both arms are the same real
    /// `password::hash_async`; only the gate differs:
    ///
    /// * `unbounded` — the pre-change shape (a million slots, no wait): every
    ///   caller starts hashing at once, so the tail is a function of how many
    ///   callers there are.
    /// * `bounded` — [`default_slots`] slots and the shipped queue wait:
    ///   admitted callers are capped at the bound, and the excess is answered
    ///   at the queue deadline (250 ms by default) instead of joining the
    ///   pile and finishing whenever the attacker's burst ends.
    ///
    /// `BURST` overrides the small burst. The numbers this prints are the
    /// lane's evidence; the debug-profile hash cost is what makes each row
    /// seconds rather than milliseconds, which is why the shapes are compared
    /// inside one run and never to a shipped-cost table.
    #[ignore = "measurement (Wave 56 lane auth-bounds): prints the bounded/unbounded hash-latency table; see internal/design/qa-p56-auth-bounds/GATES.md"]
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn hash_slot_bound_before_and_after() {
        let bound = default_slots();
        let small: usize = std::env::var("BURST")
            .ok()
            .and_then(|raw| raw.parse().ok())
            .unwrap_or(bound);
        let bursts = [small, small * 3];

        println!(
            "cores {}  holes-per-hash ~19 MiB  default bound {bound} slots ({} per core)",
            std::thread::available_parallelism()
                .map(|c| c.get())
                .unwrap_or(1),
            DEFAULT_SLOTS_PER_CORE
        );
        println!(
            "{:<28} {:>6} {:>7} {:>7} {:>9} {:>9} {:>9} {:>9} {:>9}",
            "arm",
            "burst",
            "admitted",
            "refused",
            "wall_s",
            "p50_ms",
            "p95_ms",
            "max_ms",
            "refuse_us"
        );

        for (label, slots, wait_ms) in [
            // The pre-change shape: more slots than any burst this test can
            // produce, so no caller is ever refused and every hash starts at
            // once. (Not `usize::MAX`: tokio caps a semaphore's permits.)
            ("unbounded (pre-change)", 1_000_000usize, 0u64),
            ("bounded", bound, DEFAULT_QUEUE_WAIT_MS),
        ] {
            for burst in bursts {
                let row = run_hash_burst(slots, wait_ms, burst).await;
                println!(
                    "{label:<28} {:>6} {:>7} {:>7} {:>9.2} {:>9.1} {:>9.1} {:>9.1} {:>9.0}",
                    burst,
                    row.admitted.len(),
                    row.refused,
                    row.wall.as_secs_f64(),
                    row.percentile(0.50) as f64 / 1000.0,
                    row.percentile(0.95) as f64 / 1000.0,
                    row.max_ms(),
                    row.refused_p50_us as f64,
                );
            }
        }
    }

    struct BurstRow {
        admitted: Vec<Duration>,
        refused: usize,
        refused_p50_us: u128,
        wall: Duration,
    }

    impl BurstRow {
        fn percentile(&self, p: f64) -> u128 {
            if self.admitted.is_empty() {
                return 0;
            }
            let index = (((self.admitted.len() as f64 - 1.0) * p).round() as usize)
                .min(self.admitted.len() - 1);
            self.admitted[index].as_micros()
        }

        fn max_ms(&self) -> f64 {
            self.admitted
                .last()
                .map(|d| d.as_secs_f64() * 1000.0)
                .unwrap_or(0.0)
        }
    }

    /// One burst of `burst` real argon2id hashes through one gate.
    async fn run_hash_burst(slots: usize, wait_ms: u64, burst: usize) -> BurstRow {
        use crate::auth::password;

        let gate = std::sync::Arc::new(HashGate::new(slots, Duration::from_millis(wait_ms)));
        let started = Instant::now();
        let mut tasks = Vec::with_capacity(burst);
        for _ in 0..burst {
            let gate = gate.clone();
            tasks.push(tokio::spawn(async move {
                let request_started = Instant::now();
                match gate.admit().await {
                    Ok(permit) => {
                        let _ =
                            password::hash_async("correct horse battery staple".to_owned()).await;
                        drop(permit);
                        Ok(request_started.elapsed())
                    }
                    Err(refused) => Err((refused, request_started.elapsed())),
                }
            }));
        }
        let mut admitted: Vec<Duration> = Vec::new();
        let mut refused_latency: Vec<Duration> = Vec::new();
        for task in tasks {
            match task.await.expect("hash task") {
                Ok(total) => admitted.push(total),
                Err((_refused, total)) => refused_latency.push(total),
            }
        }
        let wall = started.elapsed();
        admitted.sort();
        refused_latency.sort();
        BurstRow {
            admitted,
            refused: refused_latency.len(),
            refused_p50_us: refused_latency
                .get(refused_latency.len() / 2)
                .map(|d| d.as_micros())
                .unwrap_or(0),
            wall,
        }
    }
}
