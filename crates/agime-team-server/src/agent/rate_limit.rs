//! Rate limiting middleware for agent API
//!
//! Provides simple in-memory rate limiting to prevent abuse.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Rate limiter configuration
pub struct RateLimiter {
    /// Maximum requests per window
    max_requests: u32,
    /// Time window duration
    window: Duration,
    /// Request counts per user
    counts: RwLock<HashMap<String, (u32, Instant)>>,
}

impl RateLimiter {
    /// Create a new rate limiter
    pub fn new(max_requests: u32, window_secs: u64) -> Self {
        Self {
            max_requests,
            window: Duration::from_secs(window_secs),
            counts: RwLock::new(HashMap::new()),
        }
    }

    /// Check if request is allowed for user.
    /// Also performs inline cleanup of expired entries when the map grows large.
    pub async fn check(&self, user_id: &str) -> bool {
        let now = Instant::now();
        let mut counts = self.counts.write().await;

        // Inline cleanup: when map exceeds 1000 entries, remove expired ones
        if counts.len() > 1000 {
            counts.retain(|_, (_, start)| now.duration_since(*start) <= self.window);
        }

        if let Some((count, window_start)) = counts.get_mut(user_id) {
            if now.duration_since(*window_start) > self.window {
                // Window expired, reset
                *count = 1;
                *window_start = now;
                true
            } else if *count >= self.max_requests {
                // Rate limit exceeded
                false
            } else {
                *count += 1;
                true
            }
        } else {
            // First request from this user
            counts.insert(user_id.to_string(), (1, now));
            true
        }
    }
}

/// Default rate limiter: 60 requests per minute
#[allow(dead_code)]
pub fn default_rate_limiter() -> Arc<RateLimiter> {
    Arc::new(RateLimiter::new(60, 60))
}

/// Stricter rate limiter for task submission: 10 per minute
#[allow(dead_code)]
pub fn task_rate_limiter() -> Arc<RateLimiter> {
    Arc::new(RateLimiter::new(10, 60))
}
