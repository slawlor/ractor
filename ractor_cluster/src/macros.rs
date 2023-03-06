// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Macro helpers for remote actors

/// Derive [crate::BytesConvertable] for a given prost type. This utilizes the prost
/// extensions
///
/// Since by Rust's logic, prost::Message could be implemented for any
/// type in the future, this **could** cause a conflicting implementation
/// by defining this generic here, therefore we can't do it for non-concrete types
#[macro_export]
macro_rules! derive_serialization_for_prost_type {
    {$ty:ty} => {
        impl $crate::BytesConvertable for $ty {
            fn into_bytes(self) -> Vec<u8> {
                <Self as prost::Message>::encode_length_delimited_to_vec(&self)
            }
            fn from_bytes(bytes: Vec<u8>) -> Self {
                let buffer = bytes::Bytes::from(bytes);
                <Self as prost::Message>::decode_length_delimited(buffer).unwrap()
            }
        }
    };
}

#[cfg(test)]
mod test {
    use ractor::BytesConvertable;

    use crate::derive_serialization_for_prost_type;

    #[test]
    fn test_protobuf_message_serialization() {
        derive_serialization_for_prost_type! {crate::protocol::NetworkMessage}

        let original = crate::protocol::NetworkMessage {
            message: Some(crate::protocol::meta::network_message::Message::Node(
                crate::protocol::node::NodeMessage {
                    msg: Some(crate::protocol::node::node_message::Msg::Cast(
                        crate::protocol::node::Cast {
                            to: 3,
                            what: vec![0, 1, 2],
                            variant: "something".to_string(),
                            metadata: None,
                        },
                    )),
                },
            )),
        };

        let bytes = original.clone().into_bytes();
        let decoded = <crate::protocol::NetworkMessage as BytesConvertable>::from_bytes(bytes);
        assert_eq!(original, decoded);
    }
}
