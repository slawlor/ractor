// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Rate limiting protocols for factory routers

use std::collections::HashMap;

use crate::concurrency::{Duration, Instant};
use crate::factory::routing::RouteResult;
use crate::factory::routing::Router;
use crate::factory::Job;
use crate::factory::JobKey;
use crate::factory::WorkerId;
use crate::factory::WorkerProperties;
use crate::ActorProcessingErr;
use crate::Message;
use crate::State;

/// The maximum supported balance for leaky bucket rate limiting.
pub const MAX_LB_BALANCE: usize = isize::MAX as usize;

/// A basic trait which allows controlling rate limiting of message routing
pub trait RateLimiter: State {
    /// Check if we have not violated the rate limiter
    ///
    /// Returns [false] if we're in violation and should start rate-limiting traffic
    /// [true] otherwise
    fn check(&mut self) -> bool;

    /// Bump the rate limit internal counter, as we've routed a message
    /// to a worker
    fn bump(&mut self);
}

/// A generic struct which wraps the message router and adds support for a rate-limiting implementation to rate limit
/// jobs processed by the factory. This handles the plubming around wrapping a rate limited message router
#[derive(Debug, bon::Builder)]
pub struct RateLimitedRouter<TRouter, TRateLimit> {
    /// The underlying message router which does NOT implement rate limiting
    pub router: TRouter,
    /// The rate limiter to apply to the message routing
    pub rate_limiter: TRateLimit,
}

impl<TKey, TMsg, TRouter, TRateLimit> Router<TKey, TMsg> for RateLimitedRouter<TRouter, TRateLimit>
where
    TKey: JobKey,
    TMsg: Message,
    TRouter: Router<TKey, TMsg>,
    TRateLimit: RateLimiter,
{
    fn route_message(
        &mut self,
        job: Job<TKey, TMsg>,
        pool_size: usize,
        worker_hint: Option<WorkerId>,
        worker_pool: &mut HashMap<WorkerId, WorkerProperties<TKey, TMsg>>,
    ) -> Result<RouteResult<TKey, TMsg>, ActorProcessingErr> {
        if !self.rate_limiter.check() {
            Ok(RouteResult::RateLimited(job))
        } else {
            let result = self
                .router
                .route_message(job, pool_size, worker_hint, worker_pool);
            if matches!(result, Ok(RouteResult::Handled)) {
                // only bump the internal state if we successfully routed a message
                self.rate_limiter.bump();
            }
            result
        }
    }

    fn choose_target_worker(
        &mut self,
        job: &Job<TKey, TMsg>,
        pool_size: usize,
        worker_hint: Option<WorkerId>,
        worker_pool: &HashMap<WorkerId, WorkerProperties<TKey, TMsg>>,
    ) -> Option<WorkerId> {
        self.router
            .choose_target_worker(job, pool_size, worker_hint, worker_pool)
    }

    fn is_factory_queueing(&self) -> bool {
        self.router.is_factory_queueing()
    }
}

/// A basic leaky-bucket rate limiter. This is a synchronous implementation
/// with no interior locking since it's only used by the [RateLimitedRouter]
/// uniquely and doesn't share its state
#[derive(Debug)]
pub struct LeakyBucketRateLimiter {
    /// Tokens to add every `per` duration.
    pub refill: usize,
    /// Interval in milliseconds to add tokens.
    pub interval: Duration,
    /// Max number of tokens associated with the rate limiter.
    pub max: usize,
    /// The "balance" of the rate limiter, i.e. the number of tokens still available
    pub balance: usize,
    /// The deadline to perform another refill
    deadline: Instant,
}

#[bon::bon]
impl LeakyBucketRateLimiter {
    /// Create a new [LeakyBucketRateLimiter] instance
    ///
    /// * `refill` - Tokens to add every `per` duration.
    /// * `interval` - Interval to add tokens.
    /// * `max` - The maximum number of tokens associated with the rate limiter. Default = [MAX_LB_BALANCE]
    /// * `initial` - The initial starting balance. If [None] will be = to max
    ///
    /// Returns a new [LeakyBucketRateLimiter] instance
    #[builder]
    pub fn new(
        refill: usize,
        interval: Duration,
        #[builder(default = MAX_LB_BALANCE)] max: usize,
        initial: Option<usize>,
    ) -> LeakyBucketRateLimiter {
        LeakyBucketRateLimiter {
            refill,
            interval,
            max,
            balance: initial.unwrap_or(max),
            deadline: Instant::now() + interval,
        }
    }

    fn refresh(&mut self, now: Instant) {
        if now < self.deadline {
            return;
        }

        // Time elapsed in milliseconds since the last deadline.
        let millis = self.interval.as_millis();
        let since = now.saturating_duration_since(self.deadline).as_millis();

        let periods = usize::try_from(since / millis + 1).unwrap_or(usize::MAX);

        let tokens = periods
            .checked_mul(self.refill)
            .unwrap_or(MAX_LB_BALANCE)
            .min(MAX_LB_BALANCE);

        let remaining_millis_until_next_deadline =
            u64::try_from(since % millis).unwrap_or(u64::MAX);
        self.deadline = now
            + self
                .interval
                .saturating_sub(Duration::from_millis(remaining_millis_until_next_deadline));
        self.balance = (self.balance + tokens).min(self.max);
    }
}

impl RateLimiter for LeakyBucketRateLimiter {
    fn check(&mut self) -> bool {
        let now = Instant::now();
        self.refresh(now);
        self.balance > 0
    }

    fn bump(&mut self) {
        if self.balance > 0 {
            self.balance -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::concurrency::sleep;

    use super::*;

    #[crate::concurrency::test]
    async fn test_basic_leaky_bucket() {
        let mut limiter = LeakyBucketRateLimiter::builder()
            .refill(1)
            .initial(1)
            .interval(Duration::from_millis(100))
            .build();

        assert!(limiter.check());
        limiter.bump();
        assert!(!limiter.check());

        sleep(limiter.interval * 2).await;

        assert!(limiter.check());
        limiter.bump();
        assert!(limiter.check());
        limiter.bump();
        assert!(!limiter.check());
    }

    #[crate::concurrency::test]
    async fn test_leaky_bucket_max() {
        let mut limiter = LeakyBucketRateLimiter::builder()
            .refill(1)
            .initial(1)
            .max(1)
            .interval(Duration::from_millis(100))
            .build();

        assert!(limiter.check());
        limiter.bump();
        assert!(!limiter.check());

        sleep(limiter.interval * 2).await;

        assert!(limiter.check());
        limiter.bump();
        assert!(!limiter.check());
    }
}
