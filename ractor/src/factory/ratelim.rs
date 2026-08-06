// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Rate limiting protocols for factory routers

use std::collections::HashMap;

use crate::concurrency::Duration;
use crate::concurrency::Instant;
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
            if let Some(wid) = worker_hint {
                if worker_pool
                    .get(&wid)
                    .is_some_and(WorkerProperties::is_available)
                {
                    self.router.on_worker_availability_change(wid, true);
                }
            }
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

    fn on_worker_availability_change(&mut self, wid: WorkerId, available: bool) {
        self.router.on_worker_availability_change(wid, available);
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
    deadline: Option<Instant>,
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
            balance: initial.unwrap_or(max).min(max),
            deadline: Instant::now().checked_add(interval),
        }
    }

    fn refresh(&mut self, now: Instant) {
        let Some(deadline) = self.deadline else {
            return;
        };
        if now < deadline {
            return;
        }

        let interval_nanos = self.interval.as_nanos();
        if interval_nanos == 0 {
            self.balance = self.balance.saturating_add(self.refill).min(self.max);
            self.deadline = Some(now);
            return;
        }

        let since_nanos = now.saturating_duration_since(deadline).as_nanos();

        let periods =
            usize::try_from((since_nanos / interval_nanos).saturating_add(1)).unwrap_or(usize::MAX);

        let tokens = periods.saturating_mul(self.refill).min(MAX_LB_BALANCE);

        let nanos_since_last_period = since_nanos % interval_nanos;
        let seconds = u64::try_from(nanos_since_last_period / 1_000_000_000).unwrap_or(u64::MAX);
        let subsec_nanos =
            u32::try_from(nanos_since_last_period % 1_000_000_000).unwrap_or(u32::MAX);
        self.deadline = now.checked_add(
            self.interval
                .saturating_sub(Duration::new(seconds, subsec_nanos)),
        );
        self.balance = self.balance.saturating_add(tokens).min(self.max);
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
    use super::*;
    use crate::concurrency::sleep;
    use crate::factory::discard::WorkerDiscardSettings;
    use crate::factory::routing::QueuerRouting;
    use crate::factory::WorkerMessage;
    use crate::{Actor, ActorRef};

    struct RouterWorker;

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for RouterWorker {
        type Msg = WorkerMessage<(), ()>;
        type State = ();
        type Arguments = ();

        async fn pre_start(
            &self,
            _myself: ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
    }

    struct RejectAll;

    impl RateLimiter for RejectAll {
        fn check(&mut self) -> bool {
            false
        }

        fn bump(&mut self) {}
    }

    #[crate::concurrency::test]
    async fn rate_limit_rejection_restores_a_reserved_available_worker() {
        let (worker, handle) = Actor::spawn(None, RouterWorker, ()).await.unwrap();
        let mut pool = HashMap::from([(
            0,
            WorkerProperties::new(
                "test".to_string(),
                0,
                worker.clone(),
                WorkerDiscardSettings::None,
                None,
                handle,
                None,
            ),
        )]);
        let mut router = RateLimitedRouter {
            router: QueuerRouting::<(), ()>::default(),
            rate_limiter: RejectAll,
        };
        router.on_worker_availability_change(0, true);
        let job = Job::new((), ());

        let selected = router
            .choose_target_worker(&job, 1, None, &pool)
            .expect("an available worker should be reserved");
        assert!(matches!(
            router.route_message(job, 1, Some(selected), &mut pool),
            Ok(RouteResult::RateLimited(_))
        ));
        assert_eq!(
            router.choose_target_worker(&Job::new((), ()), 1, None, &pool),
            Some(0)
        );

        worker.stop(None);
        pool.get_mut(&0)
            .and_then(WorkerProperties::get_join_handle)
            .unwrap()
            .await
            .unwrap();
    }

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

    #[test]
    fn test_leaky_bucket_handles_zero_interval_and_saturates() {
        let mut limiter = LeakyBucketRateLimiter::builder()
            .refill(usize::MAX)
            .initial(usize::MAX)
            .interval(Duration::ZERO)
            .max(2)
            .build();

        assert_eq!(2, limiter.balance);
        limiter.bump();
        assert!(limiter.check());
        assert_eq!(2, limiter.balance);
    }

    #[test]
    fn test_leaky_bucket_tracks_sub_millisecond_periods() {
        let mut limiter = LeakyBucketRateLimiter::builder()
            .refill(1)
            .initial(0)
            .interval(Duration::from_micros(500))
            .max(10)
            .build();
        let first_deadline = limiter.deadline.expect("deadline should be representable");

        limiter.refresh(first_deadline + Duration::from_micros(1_250));

        assert_eq!(3, limiter.balance);
        assert_eq!(
            first_deadline + Duration::from_micros(1_500),
            limiter
                .deadline
                .expect("deadline should remain representable")
        );
    }

    #[test]
    fn test_leaky_bucket_handles_unrepresentable_interval() {
        let mut limiter = LeakyBucketRateLimiter::builder()
            .refill(1)
            .initial(0)
            .interval(Duration::MAX)
            .max(10)
            .build();

        assert!(limiter.deadline.is_none());
        assert!(!limiter.check());
    }
}
