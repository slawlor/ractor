// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Rate limiting protocols for factory routers

#[cfg(feature = "leaky-bucket")]
mod factory_leaky_bucket {
    use std::collections::HashMap;

    use crate::factory::routing::RouteResult;
    use crate::factory::routing::Router;
    use crate::ActorProcessingErr;
    use crate::Message;
    use crate::State;

    use crate::factory::*;

    /// A leaky-bucket rate limiter wraps the message router and is based on the [leaky_bucket] crate which provides
    /// the rate limiting fundamentals to provide rate-limited message routing for all router implementations
    #[derive(Debug)]
    pub struct LeakyBucketRateLimiter<TRouter: State> {
        router: TRouter,
        rate_limiter: leaky_bucket::RateLimiter,
    }

    impl<TKey, TMsg, TRouter> Router<TKey, TMsg> for LeakyBucketRateLimiter<TRouter>
    where
        TKey: JobKey,
        TMsg: Message,
        TRouter: Router<TKey, TMsg>,
    {
        fn route_message(
            &mut self,
            job: Job<TKey, TMsg>,
            pool_size: usize,
            worker_hint: Option<WorkerId>,
            worker_pool: &mut HashMap<WorkerId, WorkerProperties<TKey, TMsg>>,
        ) -> Result<RouteResult<TKey, TMsg>, ActorProcessingErr> {
            if self.rate_limiter.balance() == 0 {
                Ok(RouteResult::RateLimited(job))
            } else {
                let result = self
                    .router
                    .route_message(job, pool_size, worker_hint, worker_pool);
                if matches!(result, Ok(RouteResult::Handled)) {
                    // only acquire a permit if we successfully routed a request.
                    // Since this is synchronous, we shouldn't have any delta from
                    // checking the balance just above
                    _ = self.rate_limiter.try_acquire(1);
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
}

#[cfg(feature = "leaky-bucket")]
pub use factory_leaky_bucket::*;
