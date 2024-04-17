// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Lifecycle hooks support interjecting external logic into the factory's
//! lifecycle (startup/shutdown/etc) such that users can intercept and
//! adjust factory functionality at key interjection points.

use super::FactoryMessage;
use super::JobKey;
use crate::ActorProcessingErr;
use crate::ActorRef;
use crate::Message;
use crate::State;

/// Hooks for [super::Factory] lifecycle events based on the
/// underlying actor's lifecycle.
#[crate::async_trait]
pub trait FactoryLifecycleHooks<TKey, TMsg>: State + Sync
where
    TKey: JobKey,
    TMsg: Message,
{
    /// Called when the factory has completed it's startup routine but
    /// PRIOR to processing any messages. Just before this point, the factory
    /// is ready to accept and process requests and all workers are started.
    ///
    /// This hook is there to provide custom startup logic you want to make sure has run
    /// prior to processing messages on workers
    ///
    /// WARNING: An error or panic returned here WILL shutdown the factory and notify supervisors
    #[allow(unused_variables)]
    async fn on_factory_started(
        &self,
        factory_ref: ActorRef<FactoryMessage<TKey, TMsg>>,
    ) -> Result<(), ActorProcessingErr> {
        Ok(())
    }

    /// Called when the factory has completed it's shutdown routine but
    /// PRIOR to fully exiting and notifying any relevant supervisors. Just prior
    /// to this call the factory has processed its last message and will process
    /// no more messages.
    ///
    /// This hook is there to provide custom shutdown logic you want to make sure has run
    /// prior to the factory fully exiting
    async fn on_factory_stopped(&self) -> Result<(), ActorProcessingErr> {
        Ok(())
    }

    /// Called when the factory has received a signal to drain requests and exit after
    /// draining has completed.
    ///
    /// This hook is to provide the ability to notify external services that the factory
    /// is in the process of shutting down. If the factory is never "drained" formally,
    /// this hook won't be called.
    ///
    /// WARNING: An error or panic returned here WILL shutdown the factory and notify supervisors
    #[allow(unused_variables)]
    async fn on_factory_draining(
        &self,
        factory_ref: ActorRef<FactoryMessage<TKey, TMsg>>,
    ) -> Result<(), ActorProcessingErr> {
        Ok(())
    }
}
