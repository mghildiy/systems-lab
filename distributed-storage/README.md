# Big picture
We set out to understand the building blocks of distributed systems by building a mini-S3 from first principles, 
deriving each block from the problem that forces it. S3 is our vehicle, not the goal.

What we’ve covered, mapped to the original building blocks

Building block Where we are
Storage engine (WAL, durability, crash safety) Done at the single-node level: fsync, atomic rename, ordering, checksums (Problem 1)
Replication Introduced: why copies exist, quorum writes, the W/R/N tradeoff (2.1, 2.2)
Logical time / ordering Touched: why wall clocks fail, logical clocks as one answer (2.3)
Consensus / leader election Just reached the doorstep: Approach A, and the open question of how a node knows it’s still leader (2.4)
Failure detection and membership Hinted at (slow vs. dead), not yet built
Partitioning / sharding Not yet
Atomic commit (2PC) Not yet
Anti-entropy / repair Only scrubbing mentioned
Erasure coding Not yet


## Design problem 1: Store a file reliably on a single machine

1.1 “Done” is sent to client before the data is really on disk, hence durability is not ensured.

write() only puts bytes in the OS page cache (RAM), so a power cut loses them
Solution: write(), then fsync(), then ack. Persist first, ack second.
Side note: group commit lets many clients share one fsync, but nobody is acked until it returns.

1.2 Program can’t know if the later flush to disk failed

Solution: fsync() blocks until the disk confirms and returns an error if it failed. It is the only way to find out.

1.3 Crash mid-write leaves a partial file under the real name

Solution: a “complete” marker. Either a footer (length plus checksum), or write to file.tmp, fsync, then atomic rename to file.
Principle: if marker is absent, reject the file as corrupted. Discarding unpromised work is safe.
Pending: the rename itself needs a directory fsync to be durable.

1.4 The marker can reach disk before the data

The OS can flush pages in any order between two fsyncs.
Solution: write(data), fsync, write(flag), fsync, then ack.
Principle: to make A durable before B, fsync between them.

1.5 Silent corruption (bits flip, the file looks complete)

Solution: store a checksum and verify it on every read.

1.6 Checksumming a huge file without holding it all in RAM

Solution: streaming checksum, where a small running state is updated as chunks pass through.
Refinement: per-block checksums (e.g. 64 KB), so small reads verify only the blocks they touch and damage is localized.


## Design problem 2: Survive loss or corruption of the only copy

2.1 A checksum detects corruption but can’t repair it

Solution: keep multiple copies in independent failure domains (different disks, machines, racks).
On read: verify the checksum, fall back to another copy if it’s bad, then repair the bad copy.
Copies serve three goals: availability, durability, and integrity.
Background scrubbing finds silent corruption before a read does.

2.2 Copies on different machines; when to ack?

Ack after 1 copy: fast, but if rpelication fails, we fail on durability, integrity promises
Ack after all N: safe, but high latency, and one dead machine blocks writes.
Solution: quorum, ack after W of N copies are durable. Make W (and R) configurable; W + R > N ensures reads overlap the latest write.


2.3 With stale copies around, how does system know which copy is latest?

Naive idea: compare timestamps (last-write-wins). Used by Cassandra by default.
Why it breaks: each machine has its own clock, and clocks drift. A genuinely newer write can carry a smaller timestamp and silently lose. NTP narrows the gap but doesn’t remove it.
Principle: wall-clock time is the wrong tool for ordering events across machines.
Approach A: a single node decides the order. A leader numbers every write 1, 2, 3... and the number is the version. This leads toward Raft / consensus.
Approach B: logical clocks. Lamport or vector clocks track “happened before” instead of real time. This is the Dynamo branch (leaderless).
Tradeoff to explore later:
A gives a simple total order but needs a leader (and a way to survive its failure).
B needs no leader, but concurrent writes can’t always be ordered, so conflicts must be detected and resolved.

2.4 In a leader-based system, how does a client find the leader?

Option 1: proxying. The client sends to any node. If that node isn’t the leader, it forwards the request to the leader and relays the reply. Simple for the client, but costs an extra network hop.
Option 2: redirect. The non-leader replies “the leader is node B,” and the client retries against B. Keeps the server simple. Used by Raft-based systems like etcd.
Option 3: client-side tracking. The client caches the leader’s address and sends directly. If it’s stale, the request is rejected and the client re-discovers the leader.
In practice, a mix: the client holds a list of all node addresses plus a cached guess of the leader. It tries the guess first, and on a “not leader” reply (often with a hint) it updates the guess or tries the next node. Most requests go straight to the leader, and a stale guess costs one retry.
Principle: the client’s belief about the leader can always be stale, so it can never be trusted. A node must verify “am I actually the leader right now?” before accepting a write.
Open: how does a node safely know it is still the leader? This ties back to not being able to tell slow from dead, and will come up in leader election.

2.5 What needs ordering: the data or the pointer to it? (general principle, with object stores as the example)

General principle: if data is immutable (written once, never edited), it needs no ordering or consensus. Only the small mutable reference to it (a pointer, a name, a “current version”) needs a single agreed order.
Solution pattern: split the system into two layers:
a data layer of write-once blobs that only needs durable copies, and
a small metadata layer that is ordered and leader-based.
The commit of the metadata pointer is the single atomic point where the new data “comes into existence.”
Consequence: failed writes leave unreferenced data behind, so any system using this pattern needs garbage collection.
Applies to: object stores (S3), distributed file systems (GFS, HDFS), and any system that separates metadata from bulk data.
Object-store instance: an overwrite of photo.jpg writes a new blob (blob-222) beside the old one (blob-111), and only the pointer photo.jpg -> blob-? is ordered by the metadata layer.
Does not apply as-is to: systems with in-place mutable data (e.g. a database updating a row), where the data itself needs ordering.

## Design problem 3: Store data too large for one machine, across many machines

3.1 An object may be too big for one node, and one node limits throughput

Solution: split it into fixed-size chunks (e.g. 64 MB in GFS).
Benefits: no node needs space for the whole object, chunks can be read and written in parallel, and repair only re-copies the affected chunks.

3.2 Each chunk must survive node failures

Solution: replicate each chunk to N nodes, ideally in different failure domains.
Copies of the same chunk must be on different nodes. Different chunks of one object may share a node.

3.3 Something must remember which chunks make up which object, and where they live

Solution: a metadata layer holding the mapping, e.g. photo.jpg -> [chunk-1: {A, D, F}, chunk-2: {B, C, E}, ...].
A read looks up the mapping, fetches each chunk from any healthy replica, verifies checksums, and stitches the bytes together.

3.4 The two layers have very different needs (brain vs. muscle)

Data layer (“muscle”): many nodes, scales by adding more, and losing one is routine. Does the heavy work (disk I/O, checksums, streaming, scrubbing) but makes no system-wide decisions.
Metadata layer (“brain”): small, but everything depends on it. Losing it means the bytes still exist but nobody knows what they belong to. It must never be lost, never disagree with itself, and stay available.
Consequence: the metadata layer gets the expensive machinery (leader election, consensus, replicated log). The data layer gets the cheap machinery (plain copies, checksums, repair).
Applies to: GFS, HDFS, S3, and any system that separates metadata from bulk data.

3.5 Who decides which nodes get which chunks? (placement)

The metadata layer decides, since it must record the mapping anyway (GFS, HDFS).
It holds two kinds of information: the mapping (durable, replicated through the leader) and the cluster view (live, soft state built from storage-node heartbeats: alive, free space, rack).
Algorithm: filter out dead or full nodes, then choose N from the rest, one per failure domain, preferring more free space, with some randomness to avoid hotspots.
The cluster view can be slightly stale, so a placement may pick a dead node and the write path must retry.
Alternative: consistent hashing (Dynamo, Cassandra). Placement is computed from the chunk id with no central directory, but you lose control over load, failure domains and repair. This fits the leaderless branch.


3.6 Network chunks vs. storage chunks

The client’s stream arrives in arbitrary pieces (TCP segments, buffer sizes) that the server can’t rely on.
Solution: the server treats the stream as a flow of bytes and re-cuts it into its own fixed-size storage chunks by byte count. Each chunk is streamed to its replicas as it arrives and sealed (checksum finalized, commit marker written) when full or when the stream ends.
Variant: in S3 multipart upload, the client defines the parts, but they still don’t map one-to-one onto storage chunks.

3.7 Who sends the bytes to storage nodes?

Different chunks of one file can go to different node sets, chosen by the metadata layer.
Option 1: the client talks directly to storage nodes (GFS, HDFS), after asking the metadata service for placements.
Option 2: the client talks to a stateless front end (S3-style). The front end cuts the stream, gets placements, and forwards chunks.

3.8 Roles in the write path

Front end: stateless and scalable. It authenticates, cuts chunks, forwards bytes, and requests the commit. Losing one means the client retries.
Metadata leader: a node in the metadata cluster that orders and replicates metadata writes. The front end is its client, so the “find the leader” problem (2.4) belongs to the front end.
Storage nodes: run the chunk store and hold the bytes.
Upload flow: client sends bytes to a front end, which asks the leader for placements, streams chunks to storage nodes, sends the final commit to the leader, and then acks the client.

3.9 Control path vs. data path

Control path: front end <-> metadata layer (placement, commit). Small messages.
Data path: front end -> storage nodes. The metadata layer never sees the bytes.
Heartbeats: storage nodes -> metadata layer.
The front end counts bytes and cuts chunks, and the storage node assembles each chunk on disk (temp file, checksum, seal).

3.10 Erasure coding (introduced, deferred)

Instead of full copies, split a chunk into pieces and add parity pieces. Simplest case: 2 equal-length pieces A, B, plus P = A XOR B on a third node. Any one node can be lost and rebuilt.
Same protection for less storage (1.5x vs 2x here). General version: Reed-Solomon, e.g. 6 data + 3 parity pieces, survives any 3 failures at 1.5x.
Cost: repair means reading several pieces and computing, not copying one replica.
The metadata layer returns as many nodes as there are pieces, and records which piece is on which node.
Design hint: store locations as a list of pieces, so it can be added later.

4.1 The metadata layer must be a cluster (brain can’t be a single point of failure)

Run 3 or 5 nodes (odd, so a majority exists): one leader, the rest followers, all holding the same metadata.
This raises three questions: how does the leader get followers to agree before acking, who takes over if it dies, and what if the network splits and two nodes both think they lead (split brain)?

4.2 Leader dies: how is a new one chosen?

A node can’t just declare itself leader, since that risks split brain. It must run for election and win votes from a majority. Any two majorities overlap, and each node votes once per term, so two leaders can’t win the same term.
A term is a round number that rises with each election. A node votes at most once per term, and seeing a higher term resets its vote.
A candidate votes for itself. A node whose timer hasn’t fired grants its vote to the first valid candidate.
Split vote: possible if two nodes time out together. Fixed with randomized election timeouts, so one usually starts first.
If a majority of nodes are down, the cluster stops accepting writes rather than risk split brain.

## Component map

Component	Kind	Talks to
Front end	service	client, metadata, storage nodes
Metadata node	service	front end, other metadata nodes, receives heartbeats from storage nodes
Storage node	service	front end, metadata (heartbeats)
Chunk store	library	local disk only
Raft	library	other metadata nodes

| # | Component | Kind                                                                          | Talks to                                                                                            |
|---|---|-------------------------------------------------------------------------------|-----------------------------------------------------------------------------------------------------|
| 1 | Front end | service | client, metadata, storage nodes   |
| 2 | Metadata node | service                             | front end, other metadata nodes, receives heartbeats from storage nodes          |
| 3 | Storage store	 | service                                      | front end, metadata (heartbeats)                                        |
| 4 | Chunk store | library                                          | local disk only |
| 5 | Raft | library                                 | other metadata nodes                                       |

Suggested build order

chunk_store (testable alone, no network)
storage_node around it, with a simple RPC
metadata as a single node first, with allocate_chunks, commit_object and lookup
frontend tying them together, so upload and download work end to end
Raft replaces the single metadata node