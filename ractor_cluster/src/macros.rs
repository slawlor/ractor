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
    use crate::derive_serialization_for_prost_type;

    #[test]
    fn test_protobuf_message_compiles() {
        derive_serialization_for_prost_type! {crate::protocol::NetworkMessage}
    }
}
