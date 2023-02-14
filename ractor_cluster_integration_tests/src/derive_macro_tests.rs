// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use ractor::message::SerializedMessage;
use ractor::{Message, RpcReplyPort};
use ractor_cluster::{RactorClusterMessage, RactorMessage};

#[test]
fn test_non_serializable_generation() {
    #[derive(RactorMessage)]
    enum TheMessage {
        A,
    }
    assert!(!TheMessage::serializable());

    let serialize = TheMessage::A.serialize();
    assert!(serialize.is_err());
    let data = SerializedMessage::Cast {
        variant: "A".to_string(),
        args: vec![],
        metadata: None,
    };
    assert!(TheMessage::deserialize(data).is_err());
}

#[test]
fn test_serializable_generation() {
    #[derive(RactorClusterMessage)]
    enum TheMessage {
        A,
    }
    assert!(TheMessage::serializable());

    let serialize = TheMessage::A.serialize();
    assert!(matches!(serialize, Ok(SerializedMessage::Cast { .. })));

    let data = SerializedMessage::Cast {
        variant: "A".to_string(),
        args: vec![],
        metadata: None,
    };
    assert!(matches!(TheMessage::deserialize(data), Ok(TheMessage::A)));
}

#[ractor::concurrency::test]
async fn test_complex_serializable_generation() {
    #[derive(RactorClusterMessage, Debug)]
    enum TheMessage {
        A,
        #[rpc]
        B(RpcReplyPort<String>),
        C(Vec<u128>),
        #[rpc]
        D(Vec<u8>, Vec<u32>, RpcReplyPort<Vec<i8>>),
    }
    assert!(TheMessage::serializable());

    // Variant A
    let serialize = TheMessage::A.serialize();
    assert!(matches!(serialize, Ok(SerializedMessage::Cast { .. })));
    let data = SerializedMessage::Cast {
        variant: "A".to_string(),
        args: vec![],
        metadata: None,
    };
    assert!(matches!(TheMessage::deserialize(data), Ok(TheMessage::A)));
    let data = SerializedMessage::Cast {
        variant: "B".to_string(),
        args: vec![],
        metadata: None,
    };
    assert!(TheMessage::deserialize(data).is_err());

    // Variant B
    let serialize = TheMessage::B(ractor::concurrency::oneshot().0.into()).serialize();
    assert!(matches!(serialize, Ok(SerializedMessage::Call { .. })));
    let data = SerializedMessage::Call {
        variant: "B".to_string(),
        reply: ractor::concurrency::oneshot().0.into(),
        args: vec![],
        metadata: None,
    };
    assert!(matches!(
        TheMessage::deserialize(data),
        Ok(TheMessage::B(_))
    ));
    let data = SerializedMessage::Call {
        variant: "A".to_string(),
        args: vec![],
        reply: ractor::concurrency::oneshot().0.into(),
        metadata: None,
    };
    assert!(TheMessage::deserialize(data).is_err());

    // Variant C
    let serialize = TheMessage::C(vec![0u128, 1u128]).serialize();
    assert!(matches!(serialize, Ok(SerializedMessage::Cast { .. })));
    let data = serialize.unwrap();
    if let TheMessage::C(data) =
        TheMessage::deserialize(data).expect("Failed to deserialize serialized message")
    {
        assert_eq!(data, vec![0u128, 1u128]);
    } else {
        panic!("Deserialized message to incorrect type");
    }

    // Variant D
    let serialize = TheMessage::D(
        vec![0u8, 5u8],
        vec![1, 2, 3, 4],
        ractor::concurrency::oneshot().0.into(),
    )
    .serialize();
    assert!(matches!(serialize, Ok(SerializedMessage::Call { .. })));
    if let TheMessage::D(data1, data2, _port) = TheMessage::deserialize(serialize.unwrap())
        .expect("Failed to deserialize previously serialized message")
    {
        assert_eq!(data1, vec![0u8, 5u8]);
        assert_eq!(data2, vec![1, 2, 3, 4]);
    } else {
        panic!("Deserialized message to incorrect type");
    }
}
