// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Specification for a [Job] sent to a factory

use crate::SystemTime;
use std::fmt::Debug;
use std::hash::Hash;
use std::panic::RefUnwindSafe;
use std::sync::Arc;

use bon::Builder;
use tracing::Span;

use crate::{concurrency::Duration, Message};
use crate::{ActorRef, RpcReplyPort};

#[cfg(feature = "cluster")]
use crate::{message::BoxedDowncastErr, BytesConvertable};

use super::FactoryMessage;

/// Represents a key to a job. Needs to be hashable for routing properties. Additionally needs
/// to be serializable for remote factories
#[cfg(feature = "cluster")]
pub trait JobKey:
    Debug + Hash + Send + Sync + Clone + Eq + PartialEq + BytesConvertable + 'static
{
}
#[cfg(feature = "cluster")]
impl<T: Debug + Hash + Send + Sync + Clone + Eq + PartialEq + BytesConvertable + 'static> JobKey
    for T
{
}

/// Represents a key to a job. Needs to be hashable for routing properties
#[cfg(not(feature = "cluster"))]
pub trait JobKey: Debug + Hash + Send + Sync + Clone + Eq + PartialEq + 'static {}
#[cfg(not(feature = "cluster"))]
impl<T: Debug + Hash + Send + Sync + Clone + Eq + PartialEq + 'static> JobKey for T {}

/// Represents options for the specified job
#[derive(Debug, PartialEq, Clone)]
pub struct JobOptions {
    /// Time job was submitted from the client
    submit_time: SystemTime,
    /// Time job was processed by the factory
    factory_time: SystemTime,
    /// Time job was sent to a worker
    worker_time: SystemTime,
    /// Time-to-live for the job
    ttl: Option<Duration>,
    /// The parent span we want to propagate to the worker.
    /// Spans don't propagate over the wire in networks
    span: Option<Span>,
}

impl JobOptions {
    /// Create a new [JobOptions] instance, optionally supplying the ttl for the job
    ///
    /// * `ttl` - The Time-to-live specification for this job, which is the maximum amount
    ///   of time the job can remain in the factory's (or worker's) queue before being expired
    ///   and discarded
    ///
    /// Returns a new [JobOptions] instance.
    pub fn new(ttl: Option<Duration>) -> Self {
        let span = {
            #[cfg(feature = "message_span_propogation")]
            {
                Some(Span::current())
            }
            #[cfg(not(feature = "message_span_propogation"))]
            {
                None
            }
        };
        Self {
            submit_time: SystemTime::now(),
            factory_time: SystemTime::now(),
            worker_time: SystemTime::now(),
            ttl,
            span,
        }
    }

    /// Retrieve the TTL for this job
    pub fn ttl(&self) -> Option<Duration> {
        self.ttl
    }

    /// Set the TTL for this job
    pub fn set_ttl(&mut self, ttl: Option<Duration>) {
        self.ttl = ttl;
    }

    /// Time the job was submitted to the factory
    /// (i.e. the time `cast` was called)
    pub fn submit_time(&self) -> SystemTime {
        self.submit_time
    }

    /// Time the job was dispatched to a worker
    pub fn worker_time(&self) -> SystemTime {
        self.worker_time
    }

    /// Time the job was received by the factory and first either dispatched
    /// or enqueued to the factory's queue
    pub fn factory_time(&self) -> SystemTime {
        self.factory_time
    }

    /// Clone the [Span] and return it which is attached
    /// to this [JobOptions] instance.
    pub fn span(&self) -> Option<Span> {
        self.span.clone()
    }

    pub(crate) fn take_span(&mut self) -> Option<Span> {
        self.span.take()
    }
}

impl Default for JobOptions {
    fn default() -> Self {
        Self::new(None)
    }
}

#[cfg(feature = "cluster")]
impl BytesConvertable for JobOptions {
    fn into_bytes(self) -> Vec<u8> {
        let submit_time = (self
            .submit_time
            .duration_since(std::time::UNIX_EPOCH)
            .expect("Time went backwards")
            .as_nanos() as u64)
            .to_be_bytes();
        let ttl = self
            .ttl
            .map(|t| t.as_nanos() as u64)
            .unwrap_or(0)
            .to_be_bytes();

        let mut data = vec![0u8; 16];
        data[0..8].copy_from_slice(&submit_time);
        data[8..16].copy_from_slice(&ttl);
        data
    }

    fn from_bytes(mut data: Vec<u8>) -> Self {
        if data.len() != 16 {
            Self {
                span: None,
                ..Default::default()
            }
        } else {
            let ttl_bytes = data.split_off(8);

            let submit_time = u64::from_be_bytes(data.try_into().unwrap()); //Unwrap should be safe since we checked length earlier
            let ttl = u64::from_be_bytes(ttl_bytes.try_into().unwrap());

            Self {
                submit_time: std::time::UNIX_EPOCH + Duration::from_nanos(submit_time),
                ttl: if ttl > 0 {
                    Some(Duration::from_nanos(ttl))
                } else {
                    None
                },
                span: None,
                ..Default::default()
            }
        }
    }
}

/// Represents a job sent to a factory
///
/// Depending on the [super::Factory]'s routing scheme the
/// [Job]'s `key` is utilized to dispatch the job to specific
/// workers.
#[derive(Builder)]
pub struct Job<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    /// The key of the job
    pub key: TKey,
    /// The message of the job
    pub msg: TMsg,
    /// The job's options, mainly related to timing
    /// information of the job
    ///
    /// Default = [JobOptions::default()]
    #[builder(default = JobOptions::default())]
    pub options: JobOptions,
    /// If provided, this channel can be used to block pushes
    /// into the factory until the factory can "accept" the message
    /// into its internal processing. This can be used to synchronize
    /// external threadpools to the Tokio processing pool and prevent
    /// overloading the unbounded channel which fronts all actors.
    ///
    /// The reply channel return [None] if the job was accepted, or
    /// [Some(`Job`)] if it was rejected & loadshed, and then the
    /// job may be retried by the caller at a later time (if desired).
    ///
    /// Default = [None]
    pub accepted: Option<RpcReplyPort<Option<Self>>>,
}

impl<TKey, TMsg> Debug for Job<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Job")
            .field("options", &self.options)
            .field("has_accepted", &self.accepted.is_some())
            .finish()
    }
}

#[cfg(feature = "cluster")]
impl<TKey, TMsg> Job<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    fn serialize_meta(self) -> (Vec<u8>, TMsg) {
        // exactly 16 bytes
        let options_bytes = self.options.into_bytes();
        // variable length bytes based on user-defined encoding
        let key_bytes = self.key.into_bytes();
        // build the metadata
        let mut meta = vec![0u8; 16 + key_bytes.len()];
        meta[0..16].copy_from_slice(&options_bytes);
        meta[16..].copy_from_slice(&key_bytes);
        (meta, self.msg)
    }

    fn deserialize_meta(
        maybe_bytes: Option<Vec<u8>>,
    ) -> Result<(TKey, JobOptions), BoxedDowncastErr> {
        if let Some(mut meta_bytes) = maybe_bytes {
            let key_bytes = meta_bytes.split_off(16);
            Ok((
                TKey::from_bytes(key_bytes),
                JobOptions::from_bytes(meta_bytes),
            ))
        } else {
            Err(BoxedDowncastErr)
        }
    }
}

#[cfg(feature = "cluster")]
impl<TKey, TMsg> Message for Job<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    fn serializable() -> bool {
        // The job is serializable if the inner-message is serializable. The key and options
        // are always serializable
        TMsg::serializable()
    }

    fn serialize(self) -> Result<crate::message::SerializedMessage, BoxedDowncastErr> {
        let (meta_bytes, msg) = self.serialize_meta();
        // A serialized message so as-expected
        let inner_message = msg.serialize()?;

        match inner_message {
            crate::message::SerializedMessage::CallReply(_, _) => Err(BoxedDowncastErr),
            crate::message::SerializedMessage::Call {
                variant,
                args,
                reply,
                ..
            } => Ok(crate::message::SerializedMessage::Call {
                variant,
                args,
                reply,
                metadata: Some(meta_bytes),
            }),
            crate::message::SerializedMessage::Cast { variant, args, .. } => {
                Ok(crate::message::SerializedMessage::Cast {
                    variant,
                    args,
                    metadata: Some(meta_bytes),
                })
            }
        }
    }

    fn deserialize(bytes: crate::message::SerializedMessage) -> Result<Self, BoxedDowncastErr> {
        match bytes {
            crate::message::SerializedMessage::CallReply(_, _) => Err(BoxedDowncastErr),
            crate::message::SerializedMessage::Cast {
                variant,
                args,
                metadata,
            } => {
                let (key, options) = Self::deserialize_meta(metadata)?;
                let msg = TMsg::deserialize(crate::message::SerializedMessage::Cast {
                    variant,
                    args,
                    metadata: None,
                })?;
                Ok(Self {
                    msg,
                    key,
                    options,
                    accepted: None,
                })
            }
            crate::message::SerializedMessage::Call {
                variant,
                args,
                reply,
                metadata,
            } => {
                let (key, options) = Self::deserialize_meta(metadata)?;
                let msg = TMsg::deserialize(crate::message::SerializedMessage::Call {
                    variant,
                    args,
                    reply,
                    metadata: None,
                })?;
                Ok(Self {
                    msg,
                    key,
                    options,
                    accepted: None,
                })
            }
        }
    }
}

impl<TKey, TMsg> Job<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    /// Determine if this job's TTL is expired
    ///
    /// Expiration only takes effect prior to the job being
    /// started execution on a worker.
    pub fn is_expired(&self) -> bool {
        if let Some(ttl) = self.options.ttl {
            self.options.submit_time.elapsed().unwrap() > ttl
        } else {
            false
        }
    }

    /// Set the time the factory received the job
    pub(crate) fn set_factory_time(&mut self) {
        self.options.factory_time = SystemTime::now();
    }

    /// Set the time the worker began processing the job
    pub(crate) fn set_worker_time(&mut self) {
        self.options.worker_time = SystemTime::now();
    }

    /// Accept the job (if needed, telling the submitter that the job
    /// was accepted and enqueued to the factory)
    pub(crate) fn accept(&mut self) {
        if let Some(port) = self.accepted.take() {
            let _ = port.send(None);
        }
    }

    /// Reject the job. Consumes the job and returns it to the caller (if needed).
    pub(crate) fn reject(mut self) {
        if let Some(port) = self.accepted.take() {
            let _ = port.send(Some(self));
        }
    }
}

/// The retry strategy for a [RetriableMessage].
#[derive(Debug)]
pub enum MessageRetryStrategy {
    /// Retry the message forever, without limit.
    ///
    /// IMPORTANT: This requires that some other mode is provided
    /// to mark messages as eventually being `completed()`, be it
    /// discarding or it being successfully handled. Otherwise the
    /// message may spin-lock in your factory, never succeeding, and
    /// constantly being retried
    RetryForever,
    /// Retry up to the provided number of times
    Count(usize),
    /// No retries (or used to track if retries have been used internally)
    NoRetry,
}

impl MessageRetryStrategy {
    fn has_retries(&self) -> bool {
        match self {
            Self::RetryForever => true,
            Self::Count(n) if *n > 0 => true,
            _ => false,
        }
    }

    fn decrement(&self) -> Self {
        match self {
            Self::Count(n) if *n > 1 => Self::Count(*n - 1),
            Self::RetryForever => Self::RetryForever,
            _ => Self::NoRetry,
        }
    }
}

/// A retriable message is a job message which will automatically be resubmitted to the factory in the
/// event of a factory worker dropping the message due to failure (panic or unhandled error). This wraps
/// the inner message in a struct which captures the drop, and if there's still some retries left,
/// will reschedule the work to the factory with captured state information.
///
/// IMPORTANT CAVEATS:
///
/// 1. Regular loadshed events will cause this message to be retried if there is still retries left
///    and the job isn't expired unless you explicitely call `completed()` in the discard handler.
/// 2. Consumable types are not well supported here without some wrapping in Option types, which is
///    because the value is handled everywhere as `&mut ref` due to the drop implementation requiring that
///    it be so. This means that RPCs using [crate::concurrency::oneshot]s likely won't work without
///    some real painful ergonomics.
/// 3. Upon successful handling of the job, you need to mark it as `completed()` at the end of your
///    handling or discarding logic to state that it shouldn't be retried on drop.
pub struct RetriableMessage<TKey: JobKey, TMessage: Message> {
    /// The key to the retriable job
    pub key: TKey,
    /// The message, which will be retried until it's completed.
    pub message: Option<TMessage>,
    /// The retry strategy
    pub strategy: MessageRetryStrategy,
    /// A function which will be executed upon the job's retry flow being executed
    /// (helpful for logging, etc)
    ///
    /// SAFETY: We utilize [std::panic::catch_unwind] here to best-effort prevent
    /// a potential user-level panic from causing a process abort. However if the handler
    /// logic panics, then the retry hook won't be executed, but the job will still be
    /// retried.
    #[allow(clippy::type_complexity)]
    pub retry_hook: Option<Arc<dyn Fn(&TKey) + 'static + Send + Sync + RefUnwindSafe>>,

    retry_state: Option<(JobOptions, ActorRef<FactoryMessage<TKey, Self>>)>,
}

impl<TKey, TMsg> Debug for RetriableMessage<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetriableMessage")
            .field("key", &self.key)
            .field("strategy", &self.strategy)
            .field("message", &self.message.is_some())
            .field("retry_hook", &self.retry_hook.is_some())
            .field("retry_state", &self.retry_state.is_some())
            .finish()
    }
}

#[cfg(feature = "cluster")]
impl<TKey: JobKey, TMessage: Message> Message for RetriableMessage<TKey, TMessage> {}

impl<TKey: JobKey, TMessage: Message> Drop for RetriableMessage<TKey, TMessage> {
    fn drop(&mut self) {
        tracing::trace!("Drop handler for retriable message executing {self:?}");
        if !self.strategy.has_retries() || self.message.is_none() {
            // no more retries left (None or Some(>0) mean there's still retries left)
            // or the payload has been consumed
            return;
        }
        let Some((options, factory)) = self.retry_state.as_ref() else {
            // can't do a retry if the factory and options are not available
            return;
        };
        // construct the new retriable message
        let msg = Self {
            key: self.key.clone(),
            message: self.message.take(),
            strategy: self.strategy.decrement(),
            retry_state: Some((options.clone(), factory.clone())),
            retry_hook: self.retry_hook.take(),
        };
        let job = Job {
            accepted: None, // should have been accepted on the first try (if accepted at all)
            key: self.key.clone(),
            msg,
            options: options.clone(),
        };
        // Execute the custom retry hook, if provided
        if let Some(handler) = job.msg.retry_hook.as_ref() {
            let key = std::panic::AssertUnwindSafe(&job.key);
            let f = handler.clone();
            _ = std::panic::catch_unwind(move || (f)(*key));
        }
        tracing::trace!(
            "A retriable job is being resubmitted to the factory. Number of retries left {:?}",
            self.strategy
        );
        // SAFETY: A silent-drop here is OK should the dispatch to the factory fail. This is
        // because if a worker died, it would be a silent drop anyways so there's no loss
        // in functionality
        _ = factory.cast(FactoryMessage::Dispatch(job));
    }
}

impl<TKey, TMsg> ActorRef<FactoryMessage<TKey, RetriableMessage<TKey, TMsg>>>
where
    TKey: JobKey,
    TMsg: Message,
{
    /// When you're talking to a factory, which accepts "retriable" jobs, this
    /// convenience function sets up the retriable state for you and converts
    /// your job to a retriable equivalent.
    ///
    /// * `job`: The traditional [Job] which will be auto-converted into a [Job] of [RetriableMessage]
    ///   for you however the `accepted` field, if set, will be dropped as [RetriableMessage]s do not
    ///   support `accepted` replies, except on the first iteration
    /// * `strategy`: The [MessageRetryStrategy] to use for this retriable message
    ///
    /// Returns the result from the underlying `cast` operation to the factory's [ActorRef].
    #[allow(clippy::type_complexity)]
    pub fn submit_retriable_job(
        &self,
        job: Job<TKey, TMsg>,
        strategy: MessageRetryStrategy,
    ) -> Result<(), crate::MessagingErr<FactoryMessage<TKey, RetriableMessage<TKey, TMsg>>>> {
        let job = RetriableMessage::from_job(job, strategy, self.clone());
        self.cast(FactoryMessage::Dispatch(job))
    }
}

impl<TKey: JobKey, TMessage: Message> RetriableMessage<TKey, TMessage> {
    /// Construct a new retriable message with the provided number of retries. If the
    /// retries are [None], that means retry forever. [Some(0)] will mean no retries
    pub fn new(key: TKey, message: TMessage, strategy: MessageRetryStrategy) -> Self {
        Self {
            key,
            message: Some(message),
            strategy,
            retry_state: None,
            retry_hook: None,
        }
    }

    /// Attach a handler which will be executed when the job is retried (resubmitted to
    /// the factory).
    pub fn set_retry_hook(&mut self, f: impl Fn(&TKey) + 'static + Send + Sync + RefUnwindSafe) {
        self.retry_hook = Some(Arc::new(f));
    }

    /// Convert a regular [Job] into a [RetriableMessage] capturing all the necessary state in order
    /// to perform retries on drop.
    ///
    /// * `job`: The [Job] to convert
    /// * `strategy`: The [MessageRetryStrategy] to use for this retriable message
    /// * `factory`: The [ActorRef] of the factory which the job will be submitted to.
    ///
    /// Returns a [Job] with message payload of [RetriableMessage] which can be retried should it be dropped prior
    /// to having `job.msg.completed()` called.
    pub fn from_job(
        Job {
            key, msg, options, ..
        }: Job<TKey, TMessage>,
        strategy: MessageRetryStrategy,
        factory: ActorRef<FactoryMessage<TKey, Self>>,
    ) -> Job<TKey, Self> {
        let mut retriable = RetriableMessage::new(key.clone(), msg, strategy);
        retriable.capture_retry_state(&options, factory);
        Job::<TKey, Self> {
            accepted: None,
            key,
            msg: retriable,
            options,
        }
    }

    /// Capture the necessary state information in order to perform retries automatically
    ///
    /// * `options`: The [JobOptions] of the original job
    /// * `factory`: The [ActorRef] of the factory the job will be submitted to
    pub fn capture_retry_state(
        &mut self,
        options: &JobOptions,
        factory: ActorRef<FactoryMessage<TKey, Self>>,
    ) {
        self.retry_state = Some((options.clone(), factory));
    }

    /// Mark this message to not be retried upon being dropped, since it
    /// was handled successfully.
    ///
    /// IMPORTANT: This should be called at the end of the handling logic, prior to returning
    /// from the worker's message handler AND/OR from the discard logic. If you don't provide a custom
    /// discard handler for jobs, then they will be auto-retried until a potential TTL is hit (or forever)
    /// and you may risk a spin-lock where an TTL expired job is submitted back to the factory, expire shedded
    /// again, dropped, and therefore retried forever. This is especially dangerous if you state that jobs
    /// should be retried forever.
    pub fn completed(&mut self) {
        self.strategy = MessageRetryStrategy::NoRetry;
        self.message = None;
    }
}

#[cfg(feature = "cluster")]
#[cfg(test)]
mod tests {
    use super::super::FactoryMessage;
    use super::Job;
    use crate::{
        concurrency::Duration, factory::JobOptions, serialization::BytesConvertable, Message,
    };
    use crate::{message::SerializedMessage, RpcReplyPort};

    #[derive(Eq, Hash, PartialEq, Clone, Debug)]
    struct TestKey {
        item: u64,
    }

    impl crate::BytesConvertable for TestKey {
        fn from_bytes(bytes: Vec<u8>) -> Self {
            Self {
                item: u64::from_bytes(bytes),
            }
        }
        fn into_bytes(self) -> Vec<u8> {
            self.item.into_bytes()
        }
    }

    #[derive(Debug)]
    enum TestMessage {
        #[allow(dead_code)]
        A(String),
        #[allow(dead_code)]
        B(String, RpcReplyPort<u128>),
    }
    impl crate::Message for TestMessage {
        fn serializable() -> bool {
            true
        }
        fn serialize(
            self,
        ) -> Result<crate::message::SerializedMessage, crate::message::BoxedDowncastErr> {
            match self {
                Self::A(args) => Ok(crate::message::SerializedMessage::Cast {
                    variant: "A".to_string(),
                    args: <String as BytesConvertable>::into_bytes(args),
                    metadata: None,
                }),
                Self::B(args, _reply) => {
                    let (tx, _rx) = crate::concurrency::oneshot();
                    Ok(crate::message::SerializedMessage::Call {
                        variant: "B".to_string(),
                        args: <String as BytesConvertable>::into_bytes(args),
                        reply: tx.into(),
                        metadata: None,
                    })
                }
            }
        }
        fn deserialize(
            bytes: crate::message::SerializedMessage,
        ) -> Result<Self, crate::message::BoxedDowncastErr> {
            match bytes {
                crate::message::SerializedMessage::Cast { variant, args, .. } => {
                    match variant.as_str() {
                        "A" => Ok(Self::A(<String as BytesConvertable>::from_bytes(args))),
                        _ => Err(crate::message::BoxedDowncastErr),
                    }
                }
                crate::message::SerializedMessage::Call { variant, args, .. } => {
                    match variant.as_str() {
                        "B" => {
                            let (tx, _rx) = crate::concurrency::oneshot();
                            Ok(Self::B(
                                <String as BytesConvertable>::from_bytes(args),
                                tx.into(),
                            ))
                        }
                        _ => Err(crate::message::BoxedDowncastErr),
                    }
                }
                _ => Err(crate::message::BoxedDowncastErr),
            }
        }
    }

    type TheJob = Job<TestKey, TestMessage>;

    #[test]
    #[cfg_attr(
        not(all(target_arch = "wasm32", target_os = "unknown")),
        tracing_test::traced_test
    )]
    fn test_job_serialization() {
        // Check Cast variant
        let job_a = TheJob {
            key: TestKey { item: 123 },
            msg: TestMessage::A("Hello".to_string()),
            options: JobOptions::default(),
            accepted: None,
        };
        let expected_a = TheJob {
            key: TestKey { item: 123 },
            msg: TestMessage::A("Hello".to_string()),
            options: job_a.options.clone(),
            accepted: None,
        };

        let serialized_a = job_a.serialize().expect("Failed to serialize job A");
        let deserialized_a =
            TheJob::deserialize(serialized_a).expect("Failed to deserialize job A");

        assert_eq!(expected_a.key, deserialized_a.key);
        assert_eq!(
            expected_a.options.submit_time,
            deserialized_a.options.submit_time
        );
        assert_eq!(expected_a.options.ttl, deserialized_a.options.ttl);
        if let TestMessage::A(the_msg) = deserialized_a.msg {
            assert_eq!("Hello".to_string(), the_msg);
        } else {
            panic!("Failed to deserialize the message payload");
        }

        // Check RPC variant
        let job_b = TheJob {
            key: TestKey { item: 456 },
            msg: TestMessage::B("Hi".to_string(), crate::concurrency::oneshot().0.into()),
            options: JobOptions {
                ttl: Some(Duration::from_millis(1000)),
                ..Default::default()
            },
            accepted: None,
        };
        let expected_b = TheJob {
            key: TestKey { item: 456 },
            msg: TestMessage::B("Hi".to_string(), crate::concurrency::oneshot().0.into()),
            options: job_b.options.clone(),
            accepted: None,
        };
        let serialized_b = job_b.serialize().expect("Failed to serialize job B");
        let deserialized_b =
            TheJob::deserialize(serialized_b).expect("Failed to deserialize job A");

        assert_eq!(expected_b.key, deserialized_b.key);
        assert_eq!(
            expected_b.options.submit_time,
            deserialized_b.options.submit_time
        );
        assert_eq!(expected_b.options.ttl, deserialized_b.options.ttl);
        if let TestMessage::B(the_msg, _) = deserialized_b.msg {
            assert_eq!("Hi".to_string(), the_msg);
        } else {
            panic!("Failed to deserialize the message payload");
        }
    }

    #[test]
    #[cfg_attr(
        not(all(target_arch = "wasm32", target_os = "unknown")),
        tracing_test::traced_test
    )]
    fn test_factory_message_serialization() {
        let job_a = TheJob {
            key: TestKey { item: 123 },
            msg: TestMessage::A("Hello".to_string()),
            options: JobOptions::default(),
            accepted: None,
        };
        let expected_a = TheJob {
            key: TestKey { item: 123 },
            msg: TestMessage::A("Hello".to_string()),
            options: job_a.options.clone(),
            accepted: None,
        };

        let msg = FactoryMessage::Dispatch(job_a);
        let serialized_a = msg.serialize().expect("Failed to serialize");

        let serialized_a_prime = expected_a.serialize().expect("Failed to serialize");

        if let (
            SerializedMessage::Cast {
                variant: variant1,
                args: args1,
                metadata: metadata1,
            },
            SerializedMessage::Cast {
                variant: variant2,
                args: args2,
                metadata: metadata2,
            },
        ) = (serialized_a, serialized_a_prime)
        {
            assert_eq!(variant1, variant2);
            assert_eq!(args1, args2);
            assert_eq!(metadata1, metadata2);
        } else {
            panic!("Non-cast serialization")
        }
    }
}
