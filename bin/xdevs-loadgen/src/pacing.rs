//! Send pacing: a token bucket refilled at the target rate, plus a small duration parser.
//!
//! The bucket's burst capacity absorbs coarse timer wake-ups (Windows timers tick at
//! ~1-16 ms): after a late wake-up the loop sends the accumulated tokens back to back,
//! so the long-run rate stays on target without ever bursting more than `capacity`.

use std::time::{Duration, Instant};

/// Token bucket: `rate` tokens per second, at most `capacity` stored.
#[derive(Debug, Clone)]
pub struct TokenBucket {
    rate: f64,
    capacity: f64,
    tokens: f64,
    last: Instant,
}

impl TokenBucket {
    /// A bucket producing `rate` tokens/s (must be > 0) holding at most `capacity`
    /// (clamped to >= 1). Starts with one token so the first send is immediate.
    pub fn new(rate: f64, capacity: f64, now: Instant) -> Self {
        assert!(rate > 0.0, "rate must be positive");
        let capacity = capacity.max(1.0);
        Self {
            rate,
            capacity,
            tokens: 1.0,
            last: now,
        }
    }

    /// Default burst capacity for `rate`: 50 ms worth of tokens, at least 2 (with a
    /// capacity of 1, every late wake-up would drop the token accrued while oversleeping
    /// and low rates would undershoot by the timer slack).
    pub fn default_capacity(rate: f64) -> f64 {
        (rate / 20.0).max(2.0)
    }

    /// Add the tokens accrued since the last refill.
    pub fn refill(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.rate).min(self.capacity);
        self.last = now;
    }

    /// Whether a whole token is available (after refilling to `now`).
    pub fn ready(&mut self, now: Instant) -> bool {
        self.refill(now);
        self.tokens >= 1.0
    }

    /// Take one token if available.
    pub fn try_take(&mut self, now: Instant) -> bool {
        if self.ready(now) {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Time until the next whole token is available (zero if one already is).
    pub fn wait_time(&self) -> Duration {
        if self.tokens >= 1.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64((1.0 - self.tokens) / self.rate)
        }
    }

    /// Tokens currently stored.
    pub fn tokens(&self) -> f64 {
        self.tokens
    }
}

/// Parse `"60s"`, `"500ms"`, `"2m"`, `"1h"` or a bare number of seconds (`"30"`, `"1.5"`).
pub fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    let split = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let value: f64 = num
        .parse()
        .map_err(|_| format!("invalid duration {s:?} (examples: 60s, 500ms, 2m)"))?;
    let secs = match unit.trim() {
        "" | "s" | "sec" | "secs" => value,
        "ms" => value / 1000.0,
        "m" | "min" => value * 60.0,
        "h" => value * 3600.0,
        other => return Err(format!("unknown duration unit {other:?} in {s:?}")),
    };
    if !secs.is_finite() || secs < 0.0 {
        return Err(format!("invalid duration {s:?}"));
    }
    Ok(Duration::from_secs_f64(secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn bucket_paces_to_rate() {
        let t0 = Instant::now();
        let mut b = TokenBucket::new(100.0, 5.0, t0);
        // One initial token, then empty.
        assert!(b.try_take(t0));
        assert!(!b.try_take(t0));
        assert_eq!(b.wait_time(), ms(10));
        // 10 ms later exactly one more token.
        assert!(b.try_take(t0 + ms(10)));
        assert!(!b.try_take(t0 + ms(10)));
        // Simulate a loop ticking every 1 ms for 1 s: ~100 sends.
        let mut sent = 0;
        for i in 11..=1010 {
            while b.try_take(t0 + ms(i)) {
                sent += 1;
            }
        }
        assert!((99..=101).contains(&sent), "sent {sent}");
    }

    #[test]
    fn bucket_caps_bursts() {
        let t0 = Instant::now();
        let mut b = TokenBucket::new(1000.0, 5.0, t0);
        // A 1 s stall accrues 1000 tokens but only 5 are kept.
        let later = t0 + Duration::from_secs(1);
        let mut burst = 0;
        while b.try_take(later) {
            burst += 1;
        }
        assert_eq!(burst, 5);
        assert!(b.tokens() < 1.0);
    }

    #[test]
    fn coarse_timer_still_hits_rate() {
        // 200 tps with a 16 ms timer: the default capacity (10) covers the 3.2 tokens per tick.
        let t0 = Instant::now();
        let mut b = TokenBucket::new(200.0, TokenBucket::default_capacity(200.0), t0);
        let mut sent = 0;
        let mut t = 0;
        while t <= 2000 {
            while b.try_take(t0 + ms(t)) {
                sent += 1;
            }
            t += 16;
        }
        assert!((395..=402).contains(&sent), "sent {sent}");
        assert_eq!(TokenBucket::default_capacity(5.0), 2.0);
    }

    #[test]
    fn late_wakeups_do_not_lose_tokens() {
        // 10 tps, but every sleep overshoots by 15 ms: the long-run rate stays at 10.
        let t0 = Instant::now();
        let mut b = TokenBucket::new(10.0, TokenBucket::default_capacity(10.0), t0);
        let mut now = t0;
        let mut sent = 0;
        while now < t0 + Duration::from_secs(10) {
            while b.try_take(now) {
                sent += 1;
            }
            now += b.wait_time() + ms(15);
        }
        assert!((99..=101).contains(&sent), "sent {sent}");
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("60s").unwrap(), Duration::from_secs(60));
        assert_eq!(parse_duration("30").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("500ms").unwrap(), ms(500));
        assert_eq!(parse_duration("2m").unwrap(), Duration::from_secs(120));
        assert_eq!(parse_duration("1.5s").unwrap(), ms(1500));
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("10x").is_err());
        assert!(parse_duration("").is_err());
    }
}
