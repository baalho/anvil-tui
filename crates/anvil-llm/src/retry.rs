//! Retry logic for LLM API requests.
//!
//! Distinguishes retryable errors (429, 503, timeouts) from permanent ones
//! (400, 404). Exponential backoff with jitter. The `tool_choice` fallback
//! for MLX (retry without `tool_choice` on 400/422) is handled here.

use std::future::Future;
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_retries: usize,
    pub initial_delay_ms: u64,
    pub backoff_multiplier: f64,
    pub max_delay_ms: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay_ms: 1000,
            backoff_multiplier: 2.0,
            max_delay_ms: 30_000,
        }
    }
}

impl RetryConfig {
    pub fn delay_for_attempt(&self, attempt: usize) -> Duration {
        let base_ms = self.initial_delay_ms as f64 * self.backoff_multiplier.powi(attempt as i32);
        let capped_ms = (base_ms as u64).min(self.max_delay_ms);

        // Jitter ±20% using timestamp-based pseudo-randomness
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos();
        let jitter_pct = 0.8 + (nanos % 400) as f64 / 1000.0; // 0.8 to 1.2
        let jittered_ms = (capped_ms as f64 * jitter_pct) as u64;

        Duration::from_millis(jittered_ms)
    }
}

/// Whether an HTTP status code is retryable.
pub fn is_retryable_status(status: u16) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504)
}

/// Whether an error message suggests a retryable condition.
pub fn is_retryable_error(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.contains("connection reset")
        || lower.contains("connection refused")
        || lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("broken pipe")
        || lower.contains("eof")
}

/// Error wrapper that distinguishes retryable from permanent errors.
#[derive(Debug)]
pub enum RetryError<E> {
    Retryable(E),
    Permanent(E),
}

impl<E: std::fmt::Display> std::fmt::Display for RetryError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RetryError::Retryable(e) | RetryError::Permanent(e) => e.fmt(f),
        }
    }
}

impl<E> RetryError<E> {
    pub fn into_inner(self) -> E {
        match self {
            RetryError::Retryable(e) | RetryError::Permanent(e) => e,
        }
    }
}

/// Retry an async operation with exponential backoff.
/// The operation should return `RetryError::Retryable` for transient errors
/// and `RetryError::Permanent` for errors that should not be retried.
pub async fn retry_async<F, Fut, T, E>(
    config: &RetryConfig,
    mut on_retry: impl FnMut(usize, usize, Duration),
    operation: F,
) -> Result<T, E>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T, RetryError<E>>>,
    E: std::fmt::Display,
{
    let mut last_error: Option<E> = None;

    for attempt in 0..=config.max_retries {
        if attempt > 0 {
            let delay = config.delay_for_attempt(attempt - 1);
            on_retry(attempt, config.max_retries, delay);
            tokio::time::sleep(delay).await;
        }

        match operation().await {
            Ok(result) => return Ok(result),
            Err(RetryError::Permanent(e)) => return Err(e),
            Err(RetryError::Retryable(e)) => {
                last_error = Some(e);
            }
        }
    }

    Err(last_error.unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_increases_with_attempts() {
        let config = RetryConfig {
            max_retries: 5,
            initial_delay_ms: 1000,
            backoff_multiplier: 2.0,
            max_delay_ms: 30_000,
        };

        let d0 = config.delay_for_attempt(0);
        let d1 = config.delay_for_attempt(1);
        let d2 = config.delay_for_attempt(2);

        // With jitter, exact values vary, but the trend should hold
        // d0 ~= 1000ms, d1 ~= 2000ms, d2 ~= 4000ms (±20%)
        assert!(d0.as_millis() >= 800 && d0.as_millis() <= 1200);
        assert!(d1.as_millis() >= 1600 && d1.as_millis() <= 2400);
        assert!(d2.as_millis() >= 3200 && d2.as_millis() <= 4800);
    }

    #[test]
    fn delay_capped_at_max() {
        let config = RetryConfig {
            max_retries: 10,
            initial_delay_ms: 10_000,
            backoff_multiplier: 10.0,
            max_delay_ms: 30_000,
        };

        let d5 = config.delay_for_attempt(5);
        // Should be capped at ~30s (±20% jitter)
        assert!(d5.as_millis() <= 36_000);
    }

    #[test]
    fn retryable_status_codes() {
        assert!(is_retryable_status(429));
        assert!(is_retryable_status(500));
        assert!(is_retryable_status(502));
        assert!(is_retryable_status(503));
        assert!(is_retryable_status(504));
        assert!(!is_retryable_status(400));
        assert!(!is_retryable_status(401));
        assert!(!is_retryable_status(403));
        assert!(!is_retryable_status(404));
        assert!(!is_retryable_status(200));
    }

    #[tokio::test]
    async fn retry_succeeds_on_second_attempt() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let call_count = AtomicUsize::new(0);

        let config = RetryConfig {
            max_retries: 3,
            initial_delay_ms: 10,
            backoff_multiplier: 1.0,
            max_delay_ms: 100,
        };

        let result: Result<&str, String> = retry_async(
            &config,
            |_, _, _| {},
            || {
                let count = call_count.fetch_add(1, Ordering::SeqCst);
                async move {
                    if count == 0 {
                        Err(RetryError::Retryable("transient error".to_string()))
                    } else {
                        Ok("success")
                    }
                }
            },
        )
        .await;

        assert_eq!(result.unwrap(), "success");
        assert_eq!(call_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn retry_exhausts_all_attempts() {
        let config = RetryConfig {
            max_retries: 2,
            initial_delay_ms: 10,
            backoff_multiplier: 1.0,
            max_delay_ms: 100,
        };

        let result: Result<(), String> = retry_async(
            &config,
            |_, _, _| {},
            || async { Err(RetryError::Retryable("transient error".to_string())) },
        )
        .await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "transient error");
    }

    #[tokio::test]
    async fn permanent_error_stops_immediately() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let call_count = AtomicUsize::new(0);

        let config = RetryConfig {
            max_retries: 5,
            initial_delay_ms: 10,
            backoff_multiplier: 1.0,
            max_delay_ms: 100,
        };

        let result: Result<(), String> = retry_async(
            &config,
            |_, _, _| {},
            || {
                call_count.fetch_add(1, Ordering::SeqCst);
                async { Err(RetryError::Permanent("not found".to_string())) }
            },
        )
        .await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "not found");
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }
}
