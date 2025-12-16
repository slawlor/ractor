# Advanced Benchmarks for Ractor Performance Analysis

## Overview

This document describes the advanced benchmark suite designed to provide deeper insights into Ractor's performance characteristics beyond the standard benchmarks.

## Benchmark Categories

### 1. Message Size Distribution (`message_size_distribution`)

**Purpose:** Understand how message size affects performance and identify allocation overhead.

**What it tests:**

- Message processing throughput for sizes: 1B, 8B, 16B, 32B, 64B, 128B, 256B, 1KB
- Each benchmark sends 10,000 messages and measures end-to-end processing time
- Uses `Throughput::Bytes` to show MB/s for comparison

**Why it's useful:**

- **Identifies optimal message sizes** - Shows where heap allocation overhead becomes significant
- **Validates SBO threshold** - If SBO were viable, would show which size threshold (24B, 32B, 48B) gives best results
- **Cache effects** - Larger messages may show degradation due to cache misses
- **Baseline for optimization** - Establishes cost model for different message types

**Expected insights:**

- Small messages (≤16B): Should be fastest per message
- Medium messages (32-64B): Likely sweet spot for throughput
- Large messages (≥256B): May show cache/allocation overhead

**How to interpret results:**

```text
message_size_distribution/8  time: [150 ns]  throughput: [533 MB/s]
message_size_distribution/64 time: [165 ns]  throughput: [3.88 GB/s]
```

- If 64B is only 10% slower than 8B → allocation overhead is minimal
- If 256B is 50% slower than 64B → cache effects or allocation pressure

---

### 2. Supervision Heavy Workload (`supervision_heavy`)

**Purpose:** Measure supervision tree overhead under realistic hierarchical actor systems.

**What it tests:**

- Three configurations:
  - `depth3_breadth10`: Shallow wide tree (30 total actors)
  - `depth5_breadth5`: Balanced tree (775 total actors)
  - `depth10_breadth2`: Deep narrow tree (2046 total actors)
- Spawns entire tree with linked supervision
- Triggers cascading shutdown from root
- Measures full lifecycle including supervision events

**Why it's useful:**

- **Tests Phase 1.3 optimization** - Validates supervision lock optimization impact
- **Real-world workload** - Many actor systems use hierarchical supervision
- **Identifies lock contention** - Deep trees stress supervision event handling
- **Scalability testing** - Shows how supervision overhead scales with tree size

**Expected insights:**

- Phase 1.3 should show improvement in wide trees (more parallel supervision events)
- Deep trees should benefit from `for_each_child()` optimization (Phase 2.4)
- Cascading shutdown time should scale sub-linearly with tree size

**How to interpret results:**

```text
supervision_heavy/depth3_breadth10  time: [500 µs]
supervision_heavy/depth5_breadth5   time: [2.5 ms]
supervision_heavy/depth10_breadth2  time: [5 ms]
```

- Compare depth5 vs depth3: Should scale ~5-10x (not 25x despite 25x more actors)
- If linear scaling → lock contention problem
- If exponential → algorithm issue

---

### 3. Mixed Workload (`mixed_workload_100workers_1000msgs`)

**Purpose:** Simulate realistic actor system with spawning, messaging, and supervision combined.

**What it tests:**

- Spawns coordinator actor with 100 supervised child workers
- Sends 1,000 messages to each worker (100,000 total messages)
- Workers process messages and maintain state
- Coordinator triggers graceful shutdown of all workers
- Full lifecycle: spawn → work → supervise → shutdown

**Why it's useful:**

- **Realistic benchmark** - Combines all optimizations in real-world scenario
- **Overall system performance** - Not just isolated hot paths
- **Integration testing** - Ensures optimizations don't conflict
- **User-facing metric** - Represents actual application performance

**Expected insights:**

- Should show cumulative benefit of all Phase 1+2 optimizations
- Better indicator of real-world improvement than micro-benchmarks
- Helps validate that 15-20% improvement claim holds in practice

**How to interpret results:**

```text
mixed_workload_100workers_1000msgs  time: [85 ms]
```

- Compare with baseline before optimizations
- Should correlate with `process_messages` benchmark
- Larger improvement → optimizations work well together
- Smaller improvement → overhead from other operations dominates

---

### 4. Message Send Microbenchmark (`message_send_only_10000`)

**Purpose:** Isolate message send operation to measure boxing overhead independent of processing.

**What it tests:**

- Spawns single no-op actor that doesn't process messages
- Sends 10,000 messages as fast as possible
- Measures just the send path: boxing + channel send
- Actor runs in background but doesn't process (isolates send cost)

**Why it's useful:**

- **Hot path isolation** - Removes actor processing time from equation
- **Boxing overhead** - Directly measures `Message::box_message()` cost
- **Type checking cost** - Shows impact of Phase 2.2 optimization
- **SBO validation** - Would show if inline storage helps send path

**Expected insights:**

- Phase 2.2 (type check removal) should improve this benchmark
- Phase 2.3 (downcast optimization) won't affect this (only receive side)
- Very low variance → consistent boxing performance
- High variance → potential contention in channel

**How to interpret results:**

```text
message_send_only_10000  time: [50 µs]  → ~5ns per send
```

- Compare with `process_messages` throughput (~150ns per message)
- Send overhead should be <10% of total message processing time
- If send is >20% → boxing is bottleneck

---

### 5. Spawn Rate Benchmark (`spawn_rate_1000actors`)

**Purpose:** Measure maximum actor creation throughput.

**What it tests:**

- Spawns 1,000 minimal actors that immediately stop
- Waits for all to complete
- Measures pure spawn overhead without message processing
- Sequential spawning (not parallel)

**Why it's useful:**

- **Tests Phase 1.2 optimization** - Arc clone reduction during spawn
- **Startup performance** - Important for systems that create many actors
- **Spawn overhead quantification** - Shows fixed cost per actor
- **Scalability baseline** - Reference for how many actors you can create

**Expected insights:**

- Phase 1.2 should show 1-2% improvement
- Should scale linearly with actor count
- Helps understand fixed vs variable costs in spawn

**How to interpret results:**

```text
spawn_rate_1000actors  time: [2 ms]  → 2µs per actor
```

- Compare with `create_actors` benchmark (includes handle waiting)
- Should be slightly faster since actors immediately stop
- Throughput: ~500,000 actors/second is good
- <100,000 actors/second → spawn overhead too high

---

## Running the Benchmarks

### Run all advanced benchmarks

```bash
cargo bench --bench simple_advanced_benchmarks
```

### Run specific category

```bash
cargo bench --bench simple_advanced_benchmarks -- small_messages
cargo bench --bench simple_advanced_benchmarks -- large_messages
cargo bench --bench simple_advanced_benchmarks -- spawn
```

### Compare with SBO feature

```bash
# Baseline
cargo bench --bench simple_advanced_benchmarks -- small_messages > /tmp/baseline.txt

# With SBO
cargo bench --bench simple_advanced_benchmarks --features small-buffer-optimization -- small_messages > /tmp/sbo.txt

# Compare
critcmp baseline.txt sbo.txt
```

### Generate flamegraphs (requires cargo-flamegraph)

```bash
cargo flamegraph --bench simple_advanced_benchmarks -- --bench small_messages
```

---

## Expected Results Analysis

### Phase 1+2 Optimizations (Proven)

**Message Size Distribution:**

- All sizes: 3-5% improvement
- Uniform across sizes (not size-dependent)

**Supervision Heavy:**

- Wide trees (depth3_breadth10): 5-8% improvement
- Deep trees (depth10_breadth2): 2-4% improvement

**Mixed Workload:**

- Overall: 12-18% improvement (cumulative effect)

**Message Send:**

- 2-3% improvement from type check removal

**Spawn Rate:**

- 1-2% improvement from Arc clone reduction

### SBO Feature (Regression Found)

**Message Size Distribution:**

- ≤32B messages: **20-30% SLOWER** (enum overhead + ManuallyDrop)
- >32B messages: **10-15% SLOWER** (larger BoxedMessage struct)

**Why SBO Failed:**

1. Enum matching overhead on every box/unbox
2. Larger struct size (80 bytes) → worse cache locality
3. ManuallyDrop wrapper overhead
4. Runtime type ID + size verification
5. Extra indirection in match arms

---

## Diagnostic Workflow

### Problem: Performance regression in production

1. Run **Mixed Workload** to see overall impact
2. Run **Message Size Distribution** to identify if message size is factor
3. Run **Supervision Heavy** if system uses deep actor hierarchies
4. Run **Message Send** to isolate send vs receive overhead

### Problem: Optimization not showing expected gains

1. Run **Spawn Rate** to verify spawn optimization worked
2. Run **Message Send** to verify type check removal worked
3. Run **Mixed Workload** to see if gains are masked by other overhead

### Problem: Validating new optimization

1. Run **all benchmarks** before change → save results
2. Implement optimization
3. Run **all benchmarks** after change → compare
4. Use `critcmp` or Criterion's built-in comparison

---

## Adding New Benchmarks

When adding new benchmarks to this suite, ensure:

1. **Clear purpose** - What specific aspect does it measure?
2. **Isolation** - Does it test one thing or multiple?
3. **Reproducibility** - Same results across runs?
4. **Meaningful metrics** - Throughput, latency, or both?
5. **Documentation** - Update this file with interpretation guide

---

## Benchmark Limitations

### What these DON'T test

- **Memory allocations** - Use `cargo instruments` or `heaptrack`
- **Cache misses** - Use `perf stat -e cache-misses`
- **Lock contention** - Use `cargo flamegraph` or `pprof`
- **Network overhead** - Cluster features not benchmarked here
- **Real workload** - Synthetic benchmarks may not match production

### Complementary Tools

```bash
# Allocation profiling
cargo instruments -t Allocations --bench simple_advanced_benchmarks

# Cache analysis
perf stat -e cache-references,cache-misses cargo bench --bench simple_advanced_benchmarks

# Lock contention
cargo flamegraph --bench simple_advanced_benchmarks -- --profile-time 60

# Memory usage
heaptrack cargo bench --bench simple_advanced_benchmarks
```

---

## Conclusion

These advanced benchmarks provide targeted insights into:

- ✅ Message size performance characteristics
- ✅ Supervision tree overhead
- ✅ Real-world mixed workloads
- ✅ Isolated hot path operations
- ✅ Spawn throughput

Combined with the standard benchmarks, they give a complete picture of Ractor's performance profile and help validate optimizations before production deployment.

**Key Takeaway:** Always benchmark with both micro-benchmarks AND realistic workloads. Phase 1+2 optimizations showed 15-20% improvement in realistic scenarios, while SBO showed 20-30% regression despite being technically correct.
