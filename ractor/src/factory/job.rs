// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Specification for a [Job] sent to a factory

use std::fmt::Debug;
use std::{hash::Hash, time::SystemTime};

use crate::RpcReplyPort;
use crate::{concurrency::Duration, Message};

#[cfg(feature = "cluster")]
use crate::{message::BoxedDowncastErr, BytesConvertable};

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
#[derive(Debug, Eq, PartialEq, Clone)]
pub struct JobOptions {
    /// Time job was submitted from the client
    pub submit_time: SystemTime,
    /// Time job was processed by the factory
    pub factory_time: SystemTime,
    /// Time job was sent to a worker
    pub worker_time: SystemTime,
    /// Time-to-live for the job
    pub ttl: Option<Duration>,
}

impl Default for JobOptions {
    fn default() -> Self {
        Self {
            submit_time: SystemTime::now(),
            factory_time: SystemTime::now(),
            worker_time: SystemTime::now(),
            ttl: None,
        }
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
            Self::default()
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
    pub accepted: Option<RpcReplyPort<Option<Self>>>,
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
    /// Determine if this job is expired
    pub fn is_expired(&self) -> bool {
        if let Some(ttl) = self.options.ttl {
            self.options.submit_time.elapsed().unwrap() > ttl
        } else {
            false
        }
    }

    /// Set the time the factor received the job
    pub(crate) fn set_factory_time(&mut self) {
        self.options.factory_time = SystemTime::now();
    }

    /// Set the time the worker began processing the job
    pub(crate) fn set_worker_time(&mut self) {
        self.options.worker_time = SystemTime::now();
    }

    /// Accept the job (telling the submitter that the job was accepted and enqueued to the factory)
    pub(crate) fn accept(&mut self) {
        if let Some(port) = self.accepted.take() {
            let _ = port.send(None);
        }
    }

    /// Reject the job. Consumes the job and returns it to the caller under backpressure scenarios.
    pub(crate) fn reject(mut self) {
        if let Some(port) = self.accepted.take() {
            let _ = port.send(Some(self));
        }
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
    #[tracing_test::traced_test]
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
    #[tracing_test::traced_test]
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
