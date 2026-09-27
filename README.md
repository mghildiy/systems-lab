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

# Neki-like Distributed Postgres — Project Notes

https://planetscale.com/blog/the-architecture-of-neki

# Neki Components Reference Table

| # | Component | What it is | Objective in the system |
|---|---|---|---|
| 1 | PostgresManager | A process that runs as PID 1 in the Postgres container, launching and supervising `postgres` as its child | Owns the data directory; starts Postgres as a primary or a replica; sets up replication for a new node joining a shard |
| 2 | Sidecar | A custom program (Neki's own, not PgBouncer) running beside each Postgres instance, exposing a gRPC server | Pools a small set of real Postgres connections and lets Routers share them; carries session state (role, settings) per request; reports instance health and primary/replica role |
| 3 | Shard | Not a process — a primary instance + replica instances holding the same slice of data | The unit of write scaling; provides durability (async/sync/cross-zone) and read capacity via replicas |
| 4 | Admin | A cluster-wide control-plane process (not tied to one Postgres) | Health-checks shards, promotes a replica on primary failure (failover) or on request (switchover), runs `pg_rewind` to fix diverged timelines, maintains each shard's durability policy |
| 5 | Operator | A Kubernetes controller (reconciliation loop over custom resources) | Creates, updates, and replaces pods (Postgres instances, Routers, Admin, etcd) to match the declared cluster spec; handles safe replacement of live vs. lost instances |
| 6 | Router | The cluster's entry point; speaks the Postgres wire protocol (auth, Simple/Extended protocol) to apps | Presents the whole sharded cluster as one Postgres database; parses and plans queries; routes single-shard queries and coordinates cross-shard joins/aggregates; buffers queries during failover/cutover |
| 7 | Data Topology | A JSON structure (shard groups, key ranges, shard indexes, table→group mapping) stored authoritatively in etcd | The map of "which data lives where"; read by Router (planning), Sidecars, and Admin |
| 8 | etcd | An off-the-shelf strongly consistent key-value store (Raft-based) | Holds the single source of truth for the Data Topology; pushes change notifications to Router/Sidecar/Admin without restarts |
| 9 | Replicator | A process running beside each Sidecar/Postgres | Moves data live between shards (bulk copy + logical-replication catch-up) to support MoveTables, Reshard, and OnlineDDL workflows |
| 10 | Authoritative shard | Not a separate process — one designated shard, declared in the topology config | Source of truth for schema/catalog and custom-type OID numbering, so the Router can translate other shards' OIDs to match |

Note: Data plane = App → Router → Sidecar → Postgres (hot path).
Control plane = Admin + etcd (+ arguably Operator). Data plane keeps
working even if the control plane is briefly down.

## Goals
This project has two explicit goals, and every architectural decision below is made in service of them:
1. Learn distributed systems principles by building them, not just reading about them.
2. Learn Rust.

Kubernetes-operator mechanics are explicitly **not** a primary goal — where "real Neki" would require a Kubernetes Operator, we may substitute a simpler process supervisor, since Operator's reconciliation-loop pattern is useful but not particularly novel as a DS lesson.

## Reference architecture (target, eventually)
Based on: https://planetscale.com/blog/the-architecture-of-neki

| # | Component | What it is | Objective |
|---|---|---|---|
| 1 | PostgresManager | Runs as PID 1 in the Postgres container/process, supervises `postgres` as a child | Owns the data directory; starts Postgres as primary or replica; sets up replication for new nodes |
| 2 | Sidecar | Custom gRPC server beside each Postgres instance | Pools connections; carries session state; reports instance health and primary/replica role |
| 3 | Shard | Not a process — a primary + its replicas | Unit of write scaling; provides durability and read capacity |
| 4 | Admin | Cluster-wide control-plane process | Health-checks shards, promotes replicas on failure/request, runs `pg_rewind`, maintains durability policy |
| 5 | Operator | Kubernetes controller (reconciliation loop) | Creates/updates/replaces pods to match declared cluster spec |
| 6 | Router | Cluster entry point, speaks Postgres wire protocol | Presents sharded cluster as one DB; plans and routes queries; buffers during failover |
| 7 | Data Topology | JSON structure in etcd | The map of "which data lives where" |
| 8 | etcd | Raft-based KV store | Source of truth for Data Topology; pushes change notifications without restarts |
| 9 | Replicator | Process beside each Sidecar/Postgres | Moves data live between shards for MoveTables, Reshard, OnlineDDL |
| 10 | Authoritative shard | Not a process — one designated shard | Source of truth for schema/catalog and custom-type OID numbering |

**Data plane** (hot path, must stay up): App → Router → Sidecar → Postgres
**Control plane** (can be briefly down): Admin + etcd (+ arguably Operator)

## Who triggers what, in the full system

| Decision / action | Made by | Notes |
|---|---|---|
| etcd exists | Human / deploy tooling | Bootstrapped directly, first — nothing else can exist before it |
| Operator exists | Human / deploy tooling | Started directly, given a cluster spec + etcd address |
| Which instance is primary initially | Operator | Simple assignment at shard-creation time, not a contested decision |
| Where replicas run, how many | Operator | Reconciles declared spec (e.g. "primary + 2 replicas") against actual pods |
| Creating a Postgres instance + its PostgresManager | Operator | Per the declared spec |
| Creating replication slot on primary | Whoever is adding a replica (Operator at creation time, or Admin later when growing a shard) — calls PostgresManager's API on the **primary** |
| Starting a new replica against a slot | Same caller — calls PostgresManager's API on the **replica**, passing the same slot_name |
| Which replica becomes new primary on failure | Admin | Real decision under uncertainty — health + replication lag across multiple replicas |
| Which replica to promote in planned switchover | Admin | On request (resize/upgrade), not failure-triggered |
| Publishing component addresses / topology | Operator (on creation) → etcd | So Admin/Router/Sidecar can discover each other without hardcoded addresses |
| Data movement (MoveTables/Reshard/OnlineDDL) | Triggered on demand, spins up Replicator | Not present at startup at all |

Key principle: **PostgresManager never makes placement or election decisions — it only executes what it's told** (start as primary, start as replica from X, create a slot, promote). All "who/where" decisions live above it, in Operator or Admin.

## Build strategy: walking skeleton, not horizontal layers

Rejected approach: build every component to "basic" completeness in parallel/sequence, then improve all of them together. Risk: nothing runs end-to-end until everything is half-built; scope creep before any real DS lesson is reached (esp. Operator/K8s plumbing).

Chosen approach: build the smallest possible **vertical slice** that runs end-to-end, then extend it one thin capability at a time. After every lap, the system still runs — just does a little more.

### Lap-by-lap plan

**Lap 1 — PostgresManager v0**
- `StartPrimary`, `StartReplica`, `CreateReplicationSlot`, `Promote`, `Status` RPCs.
- Caller is a human (CLI / grpcurl), standing in for Operator/Admin.
- No dynamic reconfiguration, no health reporting yet, no pg_rewind.

**Lap 2 — Minimal Sidecar**
- gRPC server per Postgres instance, one RPC: `ExecuteQuery(sql) -> rows`.
- No pooling tiers, no session-state tracking yet.

**Lap 3 — Router-stub**
- Hardcoded config file: "primary is at Sidecar address X."
- Forwards client queries to that Sidecar. No wire protocol, no parsing, no sharding.

**Lap 4 — Manual failover**
- Kill primary → run `Promote` on replica by hand → hand-edit Router-stub's config to point at new primary.
- First moment it "feels" like a distributed system: multiple independent processes coordinating through defined interfaces.

**Lap 5 — Introduce etcd**
- Replace the hardcoded topology file with etcd. Router-stub/Sidecars watch it instead of reading a file.
- Still manually written to during failover — mechanism introduced before automation.
- Also a candidate point to solve **address discovery** (components publish their own addresses to etcd) rather than hardcoding.

**Lap 6 — Automate failover (baby Admin)**
- A process health-checks Postgres, and on failure, automatically does what lap 4 did by hand.
- Introduces split-brain risk and fencing as real, felt problems — not just theory.
- With only one replica, the "which replica to promote" decision is trivial; a second replica should be added specifically to make this decision non-trivial (Admin's real job: pick the healthiest/least-lagged replica).

**Later laps (order flexible, decide when we get there)**
- Second shard → forces real sharding/routing logic into Router.
- Real connection pooling tiers in Sidecar.
- Replicator, for moving data between shards (MoveTables/Reshard/OnlineDDL).
- Operator (or a simpler process supervisor, given Operator is the least DS-educational piece relative to our goals) — deliberately deferred to last.

## Open design notes / decisions to revisit
- `slot_name`: caller-chosen vs. manager-generated-and-returned — currently leaning caller-chosen for v0 simplicity, since the (human) caller can be careful about uniqueness.
- Naming collision risk: the Neki component called **Admin** (failover control plane) vs. a **human operator/admin** running setup commands — keep these unambiguous in code and docs (e.g. don't casually say "admin does X" without specifying which).
- PostgresManager's API surface will likely need a `CreateReplicationSlot` RPC in addition to `StartPrimary`/`StartReplica`/`Promote`/`Status`, since slot creation must happen on the primary before a replica can safely start streaming.

## Status
Currently starting: **PostgresManager v0** (Lap 1), in a separate chat/thread. This document reflects the plan as of the start of that work; update laps' descriptions as they're actually completed and any decisions above get resolved differently than planned.

## Note on deployment target

The reference article assumes Kubernetes, but only one component actually depends on it: **Operator** (it's explicitly built as a Kubernetes controller/reconciliation loop over custom resources). Every other component — PostgresManager, Sidecar, Admin, Router, etcd, Replicator — is deployment-agnostic: plain processes talking over gRPC/network, indifferent to whether they run in a pod, a VM, a container, or as bare binaries on a laptop.

Our plan does **not** require Kubernetes. Laps 1–6+ run as plain processes with hardcoded or etcd-based addressing. Kubernetes (or a substitute) only becomes relevant at the very last, optional lap, where Operator would otherwise live — and Operator itself may be replaced with a simpler process supervisor, since it's the least distributed-systems-educational component relative to this project's goals (DS principles + Rust, not Kubernetes).