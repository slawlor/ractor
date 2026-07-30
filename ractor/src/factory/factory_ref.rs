// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Typed operations for communicating with a factory.
//!
//! Only job dispatch is supported for remote factories. Observation and
//! management operations are local-factory-only.

use super::{FactoryMessage, Job, JobKey, JobOptions, UpdateSettingsRequest};
use crate::concurrency::Duration;
use crate::rpc::CallResult;
use crate::{ActorRef, Message, MessagingErr, RpcReplyPort};

/// A strongly typed reference to a factory.
///
/// This alias exposes factory-specific methods such as [`FactoryRef::dispatch`]
/// and [`FactoryRef::queue_depth`] while retaining all of the operations on an
/// [`ActorRef`]. Existing `ActorRef<FactoryMessage<...>>` values can use the
/// same methods without conversion.
pub type FactoryRef<TKey, TMsg> = ActorRef<FactoryMessage<TKey, TMsg>>;

/// An error returned while sending an operation to a [`FactoryRef`].
///
/// Call and query helpers return this error unboxed, matching
/// [`ActorRef::call`]. One-way helpers use [`FactorySendResult`] instead.
pub type FactoryMessagingErr<TKey, TMsg> = MessagingErr<FactoryMessage<TKey, TMsg>>;

/// The result of sending a one-way operation to a [`FactoryRef`].
///
/// Factory messages include dynamic configuration values and can therefore be
/// large. The error is boxed so the uncommon failure path does not inflate the
/// result returned by every successful operation. If sending fails, the boxed
/// [`MessagingErr::SendErr`] still owns the recoverable [`FactoryMessage`].
pub type FactorySendResult<TKey, TMsg> = Result<(), Box<FactoryMessagingErr<TKey, TMsg>>>;

impl<TKey, TMsg> ActorRef<FactoryMessage<TKey, TMsg>>
where
    TKey: JobKey,
    TMsg: Message,
{
    /// Dispatch a message to the factory using `key` for routing.
    ///
    /// Use [`Self::dispatch_with_options`] when the job needs a TTL or other
    /// non-default [`JobOptions`], or [`Self::dispatch_job`] for full control
    /// over the [`Job`].
    pub fn dispatch(&self, key: TKey, message: TMsg) -> FactorySendResult<TKey, TMsg> {
        self.dispatch_job(Job::new(key, message))
    }

    /// Dispatch a message to the factory with explicit [`JobOptions`].
    pub fn dispatch_with_options(
        &self,
        key: TKey,
        message: TMsg,
        options: JobOptions,
    ) -> FactorySendResult<TKey, TMsg> {
        self.dispatch_job(Job::with_options(key, message, options))
    }

    /// Dispatch an already configured [`Job`] to the factory.
    ///
    /// This is the most flexible dispatch operation. For a local factory, it
    /// preserves settings configured through [`Job::builder`], including an
    /// acceptance reply port. When dispatching to a remote factory, the
    /// cluster wire format does not serialize [`Job::accepted`], so no
    /// acceptance reply is sent.
    pub fn dispatch_job(&self, job: Job<TKey, TMsg>) -> FactorySendResult<TKey, TMsg> {
        Ok(self.cast(FactoryMessage::Dispatch(job))?)
    }

    /// Call a worker through the factory and await its reply.
    ///
    /// `message_builder` receives the reply port used to construct the worker
    /// message. The factory routes the resulting job like any other dispatch.
    pub async fn call_job<TReply, TMessageBuilder>(
        &self,
        key: TKey,
        message_builder: TMessageBuilder,
        timeout: Option<Duration>,
    ) -> Result<CallResult<TReply>, FactoryMessagingErr<TKey, TMsg>>
    where
        TReply: Send + 'static,
        TMessageBuilder: FnOnce(RpcReplyPort<TReply>) -> TMsg,
    {
        self.call(
            |reply| FactoryMessage::Dispatch(Job::new(key, message_builder(reply))),
            timeout,
        )
        .await
    }

    /// Call a worker through the factory using explicit [`JobOptions`].
    pub async fn call_job_with_options<TReply, TMessageBuilder>(
        &self,
        key: TKey,
        message_builder: TMessageBuilder,
        options: JobOptions,
        timeout: Option<Duration>,
    ) -> Result<CallResult<TReply>, FactoryMessagingErr<TKey, TMsg>>
    where
        TReply: Send + 'static,
        TMessageBuilder: FnOnce(RpcReplyPort<TReply>) -> TMsg,
    {
        self.call(
            |reply| {
                FactoryMessage::Dispatch(Job::with_options(key, message_builder(reply), options))
            },
            timeout,
        )
        .await
    }

    /// Retrieve the number of jobs currently held in the factory's queue.
    ///
    /// This operation is only supported for local factories.
    pub async fn queue_depth(
        &self,
        timeout: Option<Duration>,
    ) -> Result<CallResult<usize>, FactoryMessagingErr<TKey, TMsg>> {
        self.call(FactoryMessage::GetQueueDepth, timeout).await
    }

    /// Retrieve the factory's currently available worker and queue capacity.
    ///
    /// When no queue limit is configured, this reports the number of available
    /// workers. With a queue limit, it also includes the remaining queue space.
    ///
    /// This operation is only supported for local factories.
    pub async fn available_capacity(
        &self,
        timeout: Option<Duration>,
    ) -> Result<CallResult<usize>, FactoryMessagingErr<TKey, TMsg>> {
        self.call(FactoryMessage::GetAvailableCapacity, timeout)
            .await
    }

    /// Retrieve the number of workers currently processing jobs.
    ///
    /// This operation is only supported for local factories.
    pub async fn active_workers(
        &self,
        timeout: Option<Duration>,
    ) -> Result<CallResult<usize>, FactoryMessagingErr<TKey, TMsg>> {
        self.call(FactoryMessage::GetNumActiveWorkers, timeout)
            .await
    }

    /// Resize the factory's worker pool.
    ///
    /// The resize is processed asynchronously by the factory. A subsequent
    /// request sent through the same reference is processed after this one.
    ///
    /// This operation is only supported for local factories.
    pub fn adjust_worker_pool(&self, worker_count: usize) -> FactorySendResult<TKey, TMsg> {
        Ok(self.cast(FactoryMessage::AdjustWorkerPool(worker_count))?)
    }

    /// Drain all queued and active jobs, reject new work, and then stop the factory.
    ///
    /// This differs from [`crate::ActorCell::drain`], because a factory owns an
    /// internal job queue in addition to its actor mailbox.
    ///
    /// This operation is only supported for local factories.
    pub fn drain_requests(&self) -> FactorySendResult<TKey, TMsg> {
        Ok(self.cast(FactoryMessage::DrainRequests)?)
    }

    /// Apply settings which can be changed while the factory is running.
    ///
    /// This operation is only supported for local factories.
    pub fn update_settings(
        &self,
        settings: UpdateSettingsRequest<TKey, TMsg>,
    ) -> FactorySendResult<TKey, TMsg> {
        Ok(self.cast(FactoryMessage::UpdateSettings(settings))?)
    }
}
