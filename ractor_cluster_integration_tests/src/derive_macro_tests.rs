// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use ractor::message::SerializedMessage;
use ractor::Message;
use ractor::RpcReplyPort;
use ractor_cluster::RactorClusterMessage;
use ractor_cluster::RactorMessage;

#[test]
fn test_non_serializable_generation() {
    #[derive(RactorMessage)]
    enum TheMessage {
        A,
    }
    assert!(!TheMessage::serializable());

    #[derive(RactorMessage)]
    enum GenericLocal<T>
    where
        T: Send + 'static,
    {
        Data(T),
    }
    assert!(!GenericLocal::<usize>::serializable());
    let _ = GenericLocal::Data(5usize);

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

#[ractor::concurrency::test]
async fn test_generic_serializable_generation() {
    #[derive(RactorClusterMessage, Debug)]
    enum GenericMessage<T>
    where
        T: ractor::BytesConvertable + Clone + Send + 'static,
    {
        #[rpc]
        Echo(T, RpcReplyPort<T>),
        Notify(T),
    }

    assert!(GenericMessage::<String>::serializable());

    let payload = String::from("ping");

    let serialized = GenericMessage::<String>::Notify(payload.clone()).serialize();
    assert!(matches!(serialized, Ok(SerializedMessage::Cast { .. })));

    if let Ok(GenericMessage::<String>::Notify(deserialized_payload)) =
        GenericMessage::<String>::deserialize(serialized.unwrap())
    {
        assert_eq!(deserialized_payload, payload);
    } else {
        panic!("Deserialized message to incorrect type");
    }

    let payload = String::from("rpc");
    let (tx, _rx) = ractor::concurrency::oneshot::<String>();
    let serialized_call = GenericMessage::<String>::Echo(payload.clone(), tx.into()).serialize();
    assert!(matches!(serialized_call, Ok(SerializedMessage::Call { .. })));

    if let Ok(GenericMessage::<String>::Echo(deserialized_payload, _reply)) =
        GenericMessage::<String>::deserialize(serialized_call.unwrap())
    {
        assert_eq!(deserialized_payload, payload);
    } else {
        panic!("Deserialized message to incorrect type");
    }
}

#[ractor::concurrency::test]
async fn test_multi_generic_serializable_generation() {
    #[derive(RactorClusterMessage, Debug)]
    enum MultiGeneric<T, U>
    where
        T: ractor::BytesConvertable + Send + 'static,
        U: ractor::BytesConvertable + Clone + Send + 'static,
    {
        Notify(T, U),
        #[rpc]
        Fetch(T, RpcReplyPort<U>),
    }

    assert!(MultiGeneric::<String, Vec<u8>>::serializable());

    let payload_t = String::from("notify");
    let payload_u = vec![1u8, 2, 3];

    let serialized = MultiGeneric::<String, Vec<u8>>::Notify(payload_t.clone(), payload_u.clone())
        .serialize();
    assert!(matches!(serialized, Ok(SerializedMessage::Cast { .. })));

    if let Ok(MultiGeneric::<String, Vec<u8>>::Notify(deser_t, deser_u)) =
        MultiGeneric::<String, Vec<u8>>::deserialize(serialized.unwrap())
    {
        assert_eq!(deser_t, payload_t);
        assert_eq!(deser_u, payload_u);
    } else {
        panic!("Deserialized message to incorrect type");
    }

    let payload_t = String::from("fetch");
    let (tx, _rx) = ractor::concurrency::oneshot::<Vec<u8>>();
    let serialized_call =
        MultiGeneric::<String, Vec<u8>>::Fetch(payload_t.clone(), tx.into()).serialize();
    assert!(matches!(serialized_call, Ok(SerializedMessage::Call { .. })));

    if let Ok(MultiGeneric::<String, Vec<u8>>::Fetch(deser_t, _reply)) =
        MultiGeneric::<String, Vec<u8>>::deserialize(serialized_call.unwrap())
    {
        assert_eq!(deser_t, payload_t);
    } else {
        panic!("Deserialized message to incorrect type");
    }
}
