# ractor_cluster

*A companion crate to `ractor` for supporting remote actors*

[<img alt="github" src="https://img.shields.io/badge/github-slawlor/ractor-8da0cb?style=for-the-badge&labelColor=555555&logo=github" height="20">](https://github.com/slawlor/ractor)
[<img alt="crates.io" src="https://img.shields.io/crates/v/ractor_cluster.svg?style=for-the-badge&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/ractor_cluster)
[<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-ractor_cluster-66c2a5?style=for-the-badge&labelColor=555555&logo=docs.rs" height="20">](https://docs.rs/ractor_cluster)
[![CI/main](https://github.com/slawlor/ractor/actions/workflows/ci.yaml/badge.svg?branch=main)](https://github.com/slawlor/ractor/actions/workflows/ci.yaml)
[![codecov](https://codecov.io/gh/slawlor/ractor/branch/main/graph/badge.svg?token=61AGYYPWBA)](https://codecov.io/gh/slawlor/ractor)
![ractor_cluster Downloads](https://img.shields.io/crates/d/ractor_cluster.svg)

This crate contains extensions to `ractor`, a pure-Rust actor framework. Inspired from [Erlang's `gen_server`](https://www.erlang.org/doc/man/gen_server.html). 

## About

`ractor_cluster` expands upon `ractor` actors to support transmission over a network link and synchronization of actors on remote systems.

## Installation

Install `ractor_cluster` by adding the following to your Cargo.toml dependencies

```toml
[dependencies]
ractor = { version = "0.7", features = ["cluster"] }
ractor_cluster = "0.7"
```

## Ractor in distribucted clusters

Ractor actors can be built in a network-distributed pool of actors, similar to [Erlang's EPMD](https://www.erlang.org/doc/man/epmd.html) which manages inter-node connections + node naming. In our implementation, we have [`ractor_cluster`](https://crates.io/crates/ractor_cluster) in order to facilitate distributed `ractor` actors.

`ractor_cluster` has a single main type in it, namely the `NodeServer` which represents a host of a `node()` process. It additionally has some macros and a procedural macros to facilitate developer efficiency when building distributed actors. The `NodeServer` is responsible for:

1. Managing all incoming and outgoing `NodeSession` actors which represent a remote node connected to this host.
2. Managing the `TcpListener` which hosts the server socket to accept incoming session requests.

The bulk of the logic for node interconnections however is held in the `NodeSession` which manages

1. The underlying TCP connection managing reading and writing to the stream.
2. The authentication between this node and the connection to the peer
3. Managing actor lifecycle for actors spawned on the remote system.
4. Transmitting all inter-actor messages between nodes.
5. Managing PG group synchronization

etc..

The `NodeSession` makes local actors available on a remote system by spawning `RemoteActor`s which are essentially untyped actors that only handle serialized messages, leaving message deserialization up to the originating system. It also keeps track of pending RPC requests, to match request to response upon reply. There are special extension points in `ractor` which are added to specifically support `RemoteActor`s that aren't generally meant to be used outside of the standard

```rust
Actor::spawn(Some("name".to_string()), MyActor).await
```

pattern.

### Designing remote-supported actors

**Note** not all actors are created equal. Actors need to support having their message types sent over the network link. This is done by overriding specific methods of the `ractor::Message` trait all messages need to support. Due to the lack of specialization support in Rust, if you choose to use `ractor_cluster` you'll need to derive the `ractor::Message` trait for **all** message types in your crate. However to support this, we have a few procedural macros to make this a more painless process

#### Deriving the basic Message trait for in-process only actors

Many actors are going to be local-only and have no need sending messages over the network link. This is the most basic scenario and in this case the default `ractor::Message` trait implementation is fine. You can derive it quickly with:

```rust
use ractor_cluster::RactorMessage;
use ractor::RpcReplyPort;

#[derive(RactorMessage)]
enum MyBasicMessageType {
    Cast1(String, u64),
    Call1(u8, i64, RpcReplyPort<Vec<String>>),
}
```

The will implement the default ```ractor::Message``` trait for you without you having to write it out by hand.

#### Deriving the network serializable message trait for remote actors

If you want your actor to *support* remoting, then you should use a different derive statement, namely:

```rust
use ractor_cluster::RactorClusterMessage;
use ractor::RpcReplyPort;

#[derive(RactorClusterMessage)]
enum MyBasicMessageType {
    Cast1(String, u64),
    #[rpc]
    Call1(u8, i64, RpcReplyPort<Vec<String>>),
}
```

which adds a significant amount of underlying boilerplate (take a look yourself with `cargo expand`!) for the implementation. But the short answer is, each enum variant needs to serialize to a byte array of arguments, a variant name, and if it's an RPC give a port that receives a byte array and de-serialize the reply back. Each of the types inside of either the arguments or reply type need to implement the ```ractor_cluster::BytesConvertable``` trait which just says this value can be written to a byte array and decoded from a byte array. If you're using `prost` for your message type definitions (protobuf), we have a macro to auto-implement this for your types.

```rust
ractor_cluster::derive_serialization_for_prost_type! {MyProtobufType}
```

Besides that, just write your actor as you would. The actor itself will live where you define it and will be capable of receiving messages sent over the network link from other clusters!

## Contributors

The original authors of `ractor` are Sean Lawlor (@slawlor), Dillon George (@dillonrg), and Evan Au (@afterdusk). To learn more about contributing to `ractor` please see [CONTRIBUTING.md](https://github.com/slawlor/ractor/blob/main/CONTRIBUTING.md)

## License

This project is licensed under [MIT](https://github.com/slawlor/ractor/blob/main/LICENSE).
