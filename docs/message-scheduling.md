# Profiling local message scheduling

The follow-up [full benchmark comparison](full-benchmark-report.md) measures all
62 registered cases, with repeated comparisons and additional contention checks.

The `message_scheduling` benchmark compares ordinary Tokio scheduling with explicit
placement of communicating tasks. It also measures raw Tokio unbounded channels as
a control for the actor envelope, lifecycle admission, and handler dispatch costs.

## Workloads

Each timed iteration waits for **65,536 messages to be processed**. Actors,
producer tasks, channels, and runtimes persist across iterations. Startup and
shutdown are outside the timed section. Mailbox consumers read every payload and
check the message count and checksum before acknowledging completion. A batch
finishes before the next one begins, so measuring enqueue speed cannot hide an
increasing backlog.

| Workload | Traffic |
| --- | --- |
| `single` | One producer task sends to one consumer |
| `fan_in_4` | Four producer tasks send to one consumer |
| `independent_4` | Four independent producer/consumer groups |
| `ping_pong_4` | Four actor pairs, each passing a token for 16,384 deliveries |

The first three workloads have both `actor` and `tokio_channel` implementations.
Actor mailbox payloads contain one `u64`; ping-pong messages use an enum with a
token count and a peer configuration variant. Handlers perform minimal work.
Producer tasks send their entire batch without yielding, representing burst
traffic. The channel controls use the same producer and completion machinery,
but omit actor dispatch, boxing, supervision, and signal/stop handling.

## Placements

| Placement | Execution |
| --- | --- |
| `current_thread` | All tasks share the calling thread's Tokio runtime |
| `multi_1`, `multi_4`, `multi_16` | One Tokio runtime with the specified worker count |
| `shards_4_colocated` | Four current-thread runtimes on four dedicated threads; communicating tasks share a shard |
| `shards_4_crossed` | Four current-thread runtimes; communicating tasks cross shard boundaries |

For independent groups and actor pairs, colocation assigns one group to each
shard. For fan-in, colocation puts all producers and the consumer on shard zero;
crossed placement distributes producers across the four shards. A single group
uses shard zero in both shard configurations.

The coordinator only starts batches and waits for completion. In the multithread
and shard configurations, it runs on an additional calling thread. This does not
mean all threads use CPU continuously. The benchmark confines tasks to OS
threads; it does **not** pin those threads to CPU cores. The OS can move threads
between cores, including performance and efficiency cores on Apple Silicon.

## Running and profiling

Run the full suite with the default sample count and timings:

```bash
cargo bench -p ractor --bench message_scheduling
```

For a short exploratory run:

```bash
cargo bench -p ractor --bench message_scheduling -- \
  --sample-size 20 --warm-up-time 0.2 --measurement-time 1 \
  --save-baseline placement-initial
```

Repeat a selected workload for confirmation:

```bash
cargo bench -p ractor --bench message_scheduling -- \
  'actor/independent_4/' --sample-size 30 --warm-up-time 1 \
  --measurement-time 3 --save-baseline placement-confirmed
```

Build with symbols for sampling, then keep one configuration busy:

```bash
CARGO_PROFILE_BENCH_DEBUG=1 cargo bench -p ractor --bench message_scheduling -- \
  'actor/independent_4/multi_4$' --profile-time 20
```

On macOS, sample the **benchmark process**, rather than the Cargo process:

```bash
sample <benchmark-pid> 5 1 -file target/message-scheduling-multi-4.sample.txt
```

On Linux, `perf` can collect stacks and hardware counters from the benchmark
executable. Keep profiling and throughput measurements in separate runs; a
sampler affects the workload it observes. Criterion saves measurements under
`target/criterion/`. Its confidence intervals describe batch timing, and are not
per-message latency percentiles.

The suite requires `tokio_runtime` and runs on native targets with the Tokio
backend. It is skipped with `async-std` or on WASM. To compare tracing envelope
overhead, repeat with `--no-default-features -F tokio_runtime`; the default run has
span propagation enabled but no tracing subscriber.

## Placing actors with existing APIs

Ractor starts its processing task on the active Tokio runtime. An application
can select a runtime through its `Handle`, without changing Ractor's scheduler:

```rust
// Keep the selected runtime running for the entire actor lifetime.
let (actor, actor_task) = shard_handle
    .spawn(async move { ractor::Actor::spawn(None, MyActor, startup_args).await })
    .await??;
```

The outer task places initialization and the actor's processing task on that
runtime. Use a long-lived current-thread runtime to confine actors, and place
frequently communicating actors together. Runtime handles for a multithreaded
runtime select the pool, rather than an individual worker. Changing placement
also changes where handlers run: mailbox consumption and handler execution
belong to the same actor task.

Independent groups can use different shards. A hot single actor still processes
one handler at a time. Splitting communicating actors across shards introduces
cross-thread channel synchronization and wakeups. Workload imbalance and long
synchronous handlers can make fixed placement perform worse; applications must
evaluate their own throughput, latency, and utilization.

## Initial results

Measured on November 6, 2026, on an Apple M4 Max (16 cores), macOS 27.2,
Rust 1.97.0, and Tokio 1.52.2. Release optimization and line-level debug symbols
were enabled. Default Ractor features were enabled, with no tracing subscriber.
These are synthetic burst workloads, rather than application performance claims.
The core implementation baseline was commit `c3895e0`, with this benchmark added.

The following table records mean throughput in **millions of completed messages
per second**, before the polling change described below. The exploratory run used
20 samples, a 200 ms warmup, and a one-second target measurement time per case.

| Actor workload | Current thread | Pool: 1 | Pool: 4 | Pool: 16 | Shards: colocated | Shards: crossed |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Single producer/consumer | 10.03 | 10.24 | 8.52 | 8.54 | 10.31 | 10.30 |
| Four producers, one consumer | 10.43 | 10.17 | 5.15 | 4.76 | 10.37 | 5.50 |
| Four independent groups | 10.39 | 10.34 | 38.82 | 33.83 | 38.54 | 35.01 |
| Four ping-pong pairs | 6.18 | 6.40 | 18.72 | 14.40 | 16.93 | 0.93 |

Raw channel controls show the same fan-in contention pattern. Current-thread
fan-in achieved 80.56 million messages/sec, compared with 10.93 million on the
four-worker pool. Four independent raw channel groups achieved 226.99 million
on the four-worker pool and 260.06 million on colocated shards. The actor/channel
gap includes actor semantics, boxing, and a different envelope size; it does not
identify any single optimization's potential gain.

Longer confirmation runs used 30 samples, one-second warmup, and a three-second
target measurement time. Independent actor groups achieved 38.44 million/sec on
four Tokio workers versus 38.20 million/sec on colocated shards. Fan-in achieved
10.31 million/sec on one thread versus 4.84 million/sec on four workers. Crossed
ping-pong remained at 0.95 million/sec. Ping-pong on the ordinary pool and
colocated shards varied more between runs, but both remained far ahead of crossed
placement. OS thread placement and Tokio's dynamic scheduling were uncontrolled.

These measurements support grouping frequently communicating actors and using
parallelism for independent groups. They provide no evidence that replacing
Tokio's scheduler would outperform an appropriately sized Tokio pool.

### Sampled hot paths

Separate profiling runs sampled each configuration for five seconds:

- `actor/single/current_thread`: priority-port polling, cooperative scheduling,
  future polling/state movement, message allocation and downcasting were visible
  in active stacks. Polling empty supervision and lifecycle ports contributes
  work even when only user messages are ready.
- `actor/fan_in_4/multi_4`: the hottest active send paths included Tokio's MPSC
  queue push and atomic waker, plus Ractor's message admission acquisition/release.
  Shared queue and lifecycle atomics are contended by the producer tasks.
- `actor/ping_pong_4/shards_4_crossed`: condition-variable waits and signals,
  runtime injection queues, and thread parking were prominent. The benchmark
  spent more CPU time in the OS than in user space during this profile.

Samples include sleeping threads and inlined code, so their counts are not
reported as precise CPU percentages. Raw profiles and Criterion logs from this
investigation are under `target/message-scheduling/`; Criterion estimates are
under `target/criterion/`.

### Direct priority polling

The first implementation change replaces the four-branch select in
`ActorPortSet::listen_in_priority` with a `poll_fn` that polls the same channels in
priority order. It avoids constructing separate receive futures and the select
state for every message. Channel waker registration and Tokio's cooperative
budget remain in the existing channel implementations. Handler cancellation
continues to use the existing `run_with_signal` select.

The comparison used the same benchmark executable settings, with a saved copy of
the original executable, 30 samples, one-second warmup, and three-second target
measurement time. Mean batch times and throughput ratios were:

| Workload / placement | Before: ms | After: ms | Throughput increase |
| --- | ---: | ---: | ---: |
| Single / current thread | 6.368 | 6.034 | 5.5% |
| Fan-in / current thread | 6.378 | 6.097 | 4.6% |
| Independent groups / four workers | 1.696 | 1.628 | 4.2% |
| Ping-pong / four workers | 2.689 | 2.469 | 8.9% |
| Ping-pong / colocated shards | 3.384 | 3.043 | 11.2% |

Ping-pong results are sensitive to scheduling and OS placement, so the smaller
mailbox improvements are a better indication of this change's repeatability.
The priority tests exercise simultaneously ready ports, and a current-thread
test checks that a ready mailbox yields before consuming an entire backlog.

A repeat with **after then before** run ordering, 30 samples, one-second warmup,
and five-second target measurement time reproduced the mailbox gains:

- Single/current-thread: 6.419 ms before, 6.100 ms after, **5.2% more throughput**.
- Independent/four workers: 1.691 ms before, 1.621 ms after, **4.3% more throughput**.

The before/after 95% confidence intervals for mean batch time were separated in
both repeats. This is evidence for a modest local improvement under these
workloads. Per-message latency percentiles, loaded applications, active tracing
subscribers, and Linux CPU affinity remain unmeasured.
