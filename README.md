# systems-lab

A Cargo workspace of small, from-scratch implementations of classic
systems-engineering exercises, pulled from systems books/courses. Each idea
is its own workspace member, built one at a time.

## Members

| Crate | Status | Source | Goal |
|---|---|---|---|
| `malloc-allocator` | in progress | CS:APP, Malloc Lab | Custom `GlobalAlloc` impl (bump, then free-list) |
| `tcp-stack` | not started | Stanford CS144 (Sponge) | TCP from scratch over raw IP |
| `reverse-proxy` | not started | nginx / APISIX | Async TCP/HTTP reverse proxy, load balancing, health checks |
| `chaos-injector` | not started | Toxiproxy / tc netem | Fault injection plugin for reverse-proxy |
| `swim-membership` | not started | SWIM paper | Gossip-based cluster membership / failure detection |
| `raft` | not started | DDIA / MIT 6.5840 Lab 3 | Leader election + log replication |
| `sharded-kv` | not started | MIT 6.5840 Lab 4 | Fault-tolerant sharded KV store on top of raft |
| `mini-mapreduce` | not started | MIT 6.5840 Lab 1 | Coordinator + worker MapReduce |
| `lsm-storage-engine` | not started | DDIA, storage internals | LSM-tree embedded KV store: WAL, memtable, SSTables, compaction |
| `buffer-pool-manager` | not started | CMU 15-445 (BusTub) | Buffer pool manager (LRU-K) + B+-tree index |
| `tracing-collector` | not started | OpenTelemetry data model | Span ingestion + trace correlation, wired into reverse-proxy |
| `unix-shell` | not started | OSTEP, Shell Lab | Shell with job control: fg/bg, signals, process groups |
| `green-threads` | not started | OSTEP, concurrency | User-level threads: manual stack + context switch |
| `flamegraph-profiler` | not started | Systems Performance (Gregg) | Sampling profiler via perf_event_open |
| `strace-clone` | not started | The Linux Programming Interface | Syscall tracer via ptrace |

## Workspace structure