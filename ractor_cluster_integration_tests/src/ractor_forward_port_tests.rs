// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use ractor::concurrency::Duration;
use ractor::message::SerializedMessage;
use ractor::{Message, RpcReplyPort};
use ractor_cluster::RactorClusterMessage;

#[derive(RactorClusterMessage)]
enum MessageType {
    Cast(String),
    #[rpc]
    Call(String, RpcReplyPort<String>),
}

fn get_len_encoded_string(data: Vec<u8>) -> String {
    let mut ptr = 0;
    let mut len_bytes = [0u8; 8];
    len_bytes.copy_from_slice(&data[ptr..ptr + 8]);
    let len = u64::from_be_bytes(len_bytes) as usize;
    ptr += 8;
    let data_bytes = data[ptr..ptr + len].to_vec();

    <String as ractor_cluster::BytesConvertable>::from_bytes(data_bytes)
}

#[ractor::concurrency::test]
async fn no_timeout_rpc() {
    let (tx, rx) = ractor::concurrency::oneshot();
    let no_timeout = MessageType::Call("test".to_string(), tx.into());
    let no_timeout_serialized = no_timeout.serialize().expect("Failed to serialize");
    match no_timeout_serialized {
        SerializedMessage::Call { args, reply, .. } => {
            let str = get_len_encoded_string(args);
            let _ = reply.send(str.into_bytes());
        }
        _ => panic!("Invalid"),
    }

    let no_timeout_reply = rx.await.expect("Receive error");
    assert_eq!(no_timeout_reply, "test".to_string());
}

#[ractor::concurrency::test]
async fn with_timeout_rpc() {
    let (tx, rx) = ractor::concurrency::oneshot();
    let duration = Duration::from_millis(10);
    let with_timeout = MessageType::Call("test".to_string(), (tx, duration).into());

    let with_timeout_serialized = with_timeout.serialize().expect("Failed to serialize");
    match with_timeout_serialized {
        SerializedMessage::Call { args, reply, .. } => {
            let str = get_len_encoded_string(args);
            let _ = reply.send(str.into());
        }
        _ => panic!("Invalid"),
    }

    let with_timeout_reply = rx.await.expect("Receive error");
    assert_eq!(with_timeout_reply, "test".to_string());
}

#[ractor::concurrency::test]
async fn timeout_rpc() {
    let (tx, rx) = ractor::concurrency::oneshot();
    let duration = Duration::from_millis(10);
    let with_timeout = MessageType::Call("test".to_string(), (tx, duration).into());

    let with_timeout_serialized = with_timeout.serialize().expect("Failed to serialize");
    match with_timeout_serialized {
        SerializedMessage::Call { args, reply, .. } => {
            ractor::concurrency::sleep(Duration::from_millis(50)).await;
            let _ = reply.send(args);
        }
        _ => panic!("Invalid"),
    }

    let result = rx.await;
    assert!(matches!(result, Err(_)));
}

#[ractor::concurrency::test]
async fn no_timeout_rpc_decoded_reply() {
    let (tx, rx) = ractor::concurrency::oneshot();

    let no_timeout = MessageType::Call("test".to_string(), tx.into());
    let no_timeout_serialized = no_timeout.serialize().expect("Failed to serialize");
    let no_timeout_deserialized =
        MessageType::deserialize(no_timeout_serialized).expect("Failed to deserialize port");
    if let MessageType::Call(args, reply) = no_timeout_deserialized {
        let _ = reply.send(args);
    } else {
        panic!("Failed to decode with `MessageType`");
    }

    let no_timeout_reply = rx.await.expect("Receive error");
    assert_eq!(no_timeout_reply, "test".to_string());
}

#[ractor::concurrency::test]
async fn with_timeout_rpc_decoded_reply() {
    let (tx, rx) = ractor::concurrency::oneshot();

    let initial_call =
        MessageType::Call("test".to_string(), (tx, Duration::from_millis(50)).into());
    let serialized_call = initial_call.serialize().expect("Failed to serialize");
    let deserialized_call =
        MessageType::deserialize(serialized_call).expect("Failed to deserialize port");
    if let MessageType::Call(args, reply) = deserialized_call {
        let _ = reply.send(args);
    } else {
        panic!("Failed to decode with `MessageType`");
    }

    let message_reply = rx.await.expect("Receive error");
    assert_eq!(message_reply, "test".to_string());
}

#[ractor::concurrency::test]
async fn with_timeout_rpc_decoded_timeout() {
    let (tx, rx) = ractor::concurrency::oneshot();

    let initial_call =
        MessageType::Call("test".to_string(), (tx, Duration::from_millis(50)).into());
    let serialized_call = initial_call.serialize().expect("Failed to serialize");
    let deserialized_call =
        MessageType::deserialize(serialized_call).expect("Failed to deserialize port");
    if let MessageType::Call(args, reply) = deserialized_call {
        ractor::concurrency::sleep(Duration::from_millis(100)).await;
        let _ = reply.send(args);
    } else {
        panic!("Failed to decode with `MessageType`");
    }

    assert!(rx.await.is_err());
}
