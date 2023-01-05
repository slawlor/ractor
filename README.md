# ractor

A Rust actor framework.

## About

Ractor tries to solve the problem of building and maintaing an Erlang-like actor framework in Rust. It gives
a set of generic primitives and helps automate the supervision tree. It's built *heavily* on `tokio` which is a
hard requirement for `ractor`. 

## Installation

Install `ractor` by adding the following to your Cargo.toml dependencies

```toml
[dependencies]
ractor = "0.1"
```
