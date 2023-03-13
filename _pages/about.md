---
permalink: /about/
title: "About"
excerpt: "Ractor is an actor framework for Rust"
toc: true
layout: single
---

`ractor` tries to solve the problem of building and maintaining an Erlang-like actor framework in Rust. It gives
a set of generic primitives and helps automate the supervision tree and management of our actors along with the traditional actor message processing logic. It's built *heavily* on `tokio` which is a
hard requirement for `ractor` (today).

`ractor` is a modern actor framework written in 100% rust with no additional `unsafe` code.

Additionally `ractor` has a companion library, `ractor_cluster` which is needed for `ractor` to be deployed in a distributed (cluster-like) scenario. `ractor_cluster` shouldn't be considered production ready, but it is relatively stable and we'd love your feedback!

## Why ractor?

There are other actor frameworks written in Rust ([Actix](https://github.com/actix/actix), [riker](https://github.com/riker-rs/riker), or [just actors in Tokio](https://ryhl.io/blog/actors-with-tokio/)) plus a bunch of others like this list compiled on [this Reddit post](https://www.reddit.com/r/rust/comments/n2cmvd/there_are_a_lot_of_actor_framework_projects_on/).

Ractor tries to be different by modelling more on a pure Erlang `gen_server`. This means that each actor can also simply be a supervisor to other actors with no additional cost (simply link them together!). Additionally we're aiming to maintain close logic with Erlang's patterns, as they work quite well and are well utilized in the industry.

Additionally we wrote `ractor` without building on some kind of "Runtime" or "System" which needs to be spawned. Actors can be run independently, in conjunction with other basic `tokio` runtimes with little additional overhead.

We currently have full support for:

1. Single-threaded message processing
2. Actor supervision tree
3. Remote procedure calls to actors in the `rpc` module
4. Timers in the `time` module
5. Named actor registry (`registry` module) from [Erlang's `Registered processes`](https://www.erlang.org/doc/reference_manual/processes.html)
6. Process groups (`ractor::pg` module) from [Erlang's `pg` module](https://www.erlang.org/doc/man/pg.html)

On our roadmap is to add more of the Erlang functionality including potentially a distributed actor cluster.
