# Shared Discovery Utilities

This module provides shared utilities for interacting with the Prime Protocol discovery service.

## Overview

The discovery service is a temporary solution while we migrate to Kadmilla DHT. It allows nodes to register their information (IP, port, compute specs) and enables validators and orchestrators to discover nodes.

## Functions

### `fetch_nodes_from_discovery_url`
Fetches nodes from a single discovery URL with proper authentication.

### `fetch_nodes_from_discovery_urls` 
Fetches nodes from multiple discovery URLs with automatic deduplication.

### `fetch_pool_nodes_from_discovery`
Convenience function to fetch nodes for a specific compute pool.

### `fetch_validator_nodes_from_discovery`
Convenience function to fetch all validator-accessible nodes.

## Usage

### From Rust (Validator/Orchestrator)

```rust
use shared::discovery::fetch_validator_nodes_from_discovery;

let nodes = fetch_validator_nodes_from_discovery(
    &discovery_urls,
    &wallet
).await?;
```

### From Python (Worker)

```python
from prime_protocol import WorkerClient

worker = WorkerClient(compute_pool_id=1, rpc_url="http://localhost:8545")

# Configure discovery
worker.set_discovery_urls(["http://localhost:8089"])
worker.set_node_config(
    ip="192.168.1.100",
    port=8080
)

# Upload to discovery
worker.start()
worker.upload_to_discovery()
```

## Migration Plan

This is a temporary solution. Once Kadmilla DHT is fully integrated:
1. Discovery uploads will be replaced with DHT announcements
2. Discovery queries will be replaced with DHT lookups
3. The discovery service endpoints can be deprecated 