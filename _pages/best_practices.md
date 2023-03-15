---
permalink: /faq/
title: "Frequently asked questions"
toc: true
layout: single
author_profile: false
---

## Should actors be long lived or short lived?

General guidance is that actors should be long lived. There's a historical reason here (Erlang patterns are typically long-lived `gen_server`s).
The other reason, internally to ractor, is there's non-trivial overhead spawning and creating the actor. Not only from the real actor spawn, creating
necessary dependent resources and actor ref's, but also from any overhead incurred in the actor's `pre_start` routine.

The tl;dr; here is spawning off an actor to do a single task is generally discouraged.

## Why is the actor's state separate from the actor's `self`?

This was an initial design choice when building `ractor`. The reason is somewhat complex, but it relates to creation of the state. For one thing, if the actor's state was just the actor's `self`, that would mean that the creation of that actor instance occurred somewhere else. This means that if during the initialization, it were to have an unhandled error or `panic!` it wouldn't be caught in that actor's startup routine.

Either that or there would need to be a lot of boiler-plate logic so that `self` is half-initialized elsewhere and then finished initialization in `pre_start` of the fallible bits. Therefore it was decided to utilize a separate state struct which is fully-owned by the actor runtime, and is only instantiated within the wrapped `pre_start` routine. This way unhandled `panic!`s and errors are handled and can be cleanly managed.

## More to come
