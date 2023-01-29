use ractor::RpcReplyPort;

#[derive(ractor_cluster::RactorClusterMessage)]
enum TestMacro {
    Cast1,
    Cast2(u64),
    #[rpc]
    Call3(RpcReplyPort<u64>),
    #[rpc]
    Call4(u64, u64, u64, RpcReplyPort<u64>),
}

// use ractor_cluster::BytesConvertable;
// impl ractor::Message for TestMacro {
//     fn serializable() -> bool {
//         true
//     }
//     fn serialize(
//         self,
//     ) -> Result<
//         ractor::message::SerializedMessage,
//         ractor::message::BoxedDowncastErr,
//     > {
//         match self {
//             Self::Cast1 => {
//                 Ok(ractor::message::SerializedMessage::Cast {
//                     variant: "Cast1".to_string(),
//                     data: Vec::new(),
//                 })
//             }
//             Self::Cast2(field0) => {
//                 let mut data = Vec::new();
//                 {
//                     let arg_data = field0.into_bytes();
//                     let arg_len = (arg_data.len() as u64).to_be_bytes();
//                     data.extend(arg_len);
//                     data.extend(arg_data);
//                 };
//                 Ok(ractor::message::SerializedMessage::Cast {
//                     variant: "Cast2".to_string(),
//                     data: data,
//                 })
//             }
//             Self::Call3(reply) => {
//                 let target_port = {
//                     let (tx, rx) = ractor::concurrency::oneshot();
//                     let o_timeout = reply.get_timeout();
//                     ractor::concurrency::spawn(async move {
//                         if let Some(timeout) = o_timeout {
//                             if let Ok(Ok(result))
//                                 = ractor::concurrency::timeout(timeout, rx).await
//                             {
//                                 let typed_result = <u64 as ractor_cluster::BytesConvertable>::from_bytes(
//                                     result,
//                                 );
//                                 let _ = reply.send(typed_result);
//                             }
//                         } else {
//                             if let Ok(result) = rx.await {
//                                 let typed_result = <u64 as ractor_cluster::BytesConvertable>::from_bytes(
//                                     result,
//                                 );
//                                 let _ = reply.send(typed_result);
//                             }
//                         }
//                     });
//                     if let Some(timeout) = o_timeout {
//                         ractor::RpcReplyPort::<_>::from((tx, timeout))
//                     } else {
//                         ractor::RpcReplyPort::<_>::from(tx)
//                     }
//                 };
//                 Ok(ractor::message::SerializedMessage::Call {
//                     variant: "Call3".to_string(),
//                     args: Vec::new(),
//                     reply: target_port,
//                 })
//             }
//             Self::Call4(field0, field1, field2, reply) => {
//                 let mut data = Vec::new();
//                 {
//                     let arg_data = field0.into_bytes();
//                     let arg_len = (arg_data.len() as u64).to_be_bytes();
//                     data.extend(arg_len);
//                     data.extend(arg_data);
//                 };
//                 {
//                     let arg_data = field1.into_bytes();
//                     let arg_len = (arg_data.len() as u64).to_be_bytes();
//                     data.extend(arg_len);
//                     data.extend(arg_data);
//                 };
//                 {
//                     let arg_data = field2.into_bytes();
//                     let arg_len = (arg_data.len() as u64).to_be_bytes();
//                     data.extend(arg_len);
//                     data.extend(arg_data);
//                 };
//                 let target_port = {
//                     let (tx, rx) = ractor::concurrency::oneshot();
//                     let o_timeout = reply.get_timeout();
//                     ractor::concurrency::spawn(async move {
//                         if let Some(timeout) = o_timeout {
//                             if let Ok(Ok(result))
//                                 = ractor::concurrency::timeout(timeout, rx).await
//                             {
//                                 let typed_result = <u64 as ractor_cluster::BytesConvertable>::from_bytes(
//                                     result,
//                                 );
//                                 let _ = reply.send(typed_result);
//                             }
//                         } else {
//                             if let Ok(result) = rx.await {
//                                 let typed_result = <u64 as ractor_cluster::BytesConvertable>::from_bytes(
//                                     result,
//                                 );
//                                 let _ = reply.send(typed_result);
//                             }
//                         }
//                     });
//                     if let Some(timeout) = o_timeout {
//                         ractor::RpcReplyPort::<_>::from((tx, timeout))
//                     } else {
//                         ractor::RpcReplyPort::<_>::from(tx)
//                     }
//                 };
//                 Ok(ractor::message::SerializedMessage::Call {
//                     variant: "Call4".to_string(),
//                     args: data,
//                     reply: target_port,
//                 })
//             }
//         }
//     }
//     fn deserialize(
//         bytes: ractor::message::SerializedMessage,
//     ) -> Result<Self, ractor::message::BoxedDowncastErr> {
//         match bytes {
//             ractor::message::SerializedMessage::Cast { variant, data } => {
//                 match variant.as_str() {
//                     "Cast1" => Ok(Self::Cast1),
//                     "Cast2" => {
//                         let mut ptr = 0usize;
//                         let field0 = {
//                             let mut len_bytes = [0u8; 8];
//                             len_bytes.copy_from_slice(&data[ptr..ptr + 8]);
//                             let len = u64::from_be_bytes(len_bytes) as usize;
//                             ptr += 8;
//                             let data_bytes = data[ptr..ptr + len].to_vec();
//                             let t_result = <u64 as ractor_cluster::BytesConvertable>::from_bytes(
//                                 data_bytes,
//                             );
//                             ptr += len;
//                             t_result
//                         };
//                         Ok(Self::Cast2(field0))
//                     }
//                     _ => Err(ractor::message::BoxedDowncastErr),
//                 }
//             }
//             ractor::message::SerializedMessage::Call {
//                 variant,
//                 args: data,
//                 reply,
//             } => {
//                 match variant.as_str() {
//                     "Call3" => {
//                         let target_port = {
//                             let (tx, rx) = ractor::concurrency::oneshot::<u64>();
//                             let o_timeout = reply.get_timeout();
//                             ractor::concurrency::spawn(async move {
//                                 if let Some(timeout) = o_timeout {
//                                     if let Ok(Ok(result))
//                                         = ractor::concurrency::timeout(timeout, rx).await
//                                     {
//                                         let _ = reply.send(result.into_bytes());
//                                     }
//                                 } else {
//                                     if let Ok(result) = rx.await {
//                                         let _ = reply.send(result.into_bytes());
//                                     }
//                                 }
//                             });
//                             if let Some(timeout) = o_timeout {
//                                 ractor::RpcReplyPort::<_>::from((tx, timeout))
//                             } else {
//                                 ractor::RpcReplyPort::<_>::from(tx)
//                             }
//                         };
//                         Ok(Self::Call3(target_port))
//                     }
//                     "Call4" => {
//                         let mut ptr = 0usize;
//                         let field0 = {
//                             let mut len_bytes = [0u8; 8];
//                             len_bytes.copy_from_slice(&data[ptr..ptr + 8]);
//                             let len = u64::from_be_bytes(len_bytes) as usize;
//                             ptr += 8;
//                             let data_bytes = data[ptr..ptr + len].to_vec();
//                             let t_result = <u64 as ractor_cluster::BytesConvertable>::from_bytes(
//                                 data_bytes,
//                             );
//                             ptr += len;
//                             t_result
//                         };
//                         let field1 = {
//                             let mut len_bytes = [0u8; 8];
//                             len_bytes.copy_from_slice(&data[ptr..ptr + 8]);
//                             let len = u64::from_be_bytes(len_bytes) as usize;
//                             ptr += 8;
//                             let data_bytes = data[ptr..ptr + len].to_vec();
//                             let t_result = <u64 as ractor_cluster::BytesConvertable>::from_bytes(
//                                 data_bytes,
//                             );
//                             ptr += len;
//                             t_result
//                         };
//                         let field2 = {
//                             let mut len_bytes = [0u8; 8];
//                             len_bytes.copy_from_slice(&data[ptr..ptr + 8]);
//                             let len = u64::from_be_bytes(len_bytes) as usize;
//                             ptr += 8;
//                             let data_bytes = data[ptr..ptr + len].to_vec();
//                             let t_result = <u64 as ractor_cluster::BytesConvertable>::from_bytes(
//                                 data_bytes,
//                             );
//                             ptr += len;
//                             t_result
//                         };
//                         let target_port = {
//                             let (tx, rx) = ractor::concurrency::oneshot::<u64>();
//                             let o_timeout = reply.get_timeout();
//                             ractor::concurrency::spawn(async move {
//                                 if let Some(timeout) = o_timeout {
//                                     if let Ok(Ok(result))
//                                         = ractor::concurrency::timeout(timeout, rx).await
//                                     {
//                                         let _ = reply.send(result.into_bytes());
//                                     }
//                                 } else {
//                                     if let Ok(result) = rx.await {
//                                         let _ = reply.send(result.into_bytes());
//                                     }
//                                 }
//                             });
//                             if let Some(timeout) = o_timeout {
//                                 ractor::RpcReplyPort::<_>::from((tx, timeout))
//                             } else {
//                                 ractor::RpcReplyPort::<_>::from(tx)
//                             }
//                         };
//                         Ok(Self::Call4(field0, field1, field2, target_port))
//                     }
//                     _ => Err(ractor::message::BoxedDowncastErr),
//                 }
//             }
//             _ => Err(ractor::message::BoxedDowncastErr),
//         }
//     }
// }
