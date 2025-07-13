# Prime Protocol Python SDK

Python bindings for the Prime Protocol, providing easy access to worker, orchestrator, and validator functionalities.

## Installation

```bash
# Clone the repository
git clone https://github.com/primeprotocol/protocol.git
cd protocol/crates/prime-protocol-py

# Build and install the package
pip install maturin
maturin develop
```

## Validator Client

The Validator Client allows validators to interact with the Prime Protocol network, particularly for listing and validating nodes.

### Basic Usage

```python
from primeprotocol import ValidatorClient

# Initialize the validator client
validator = ValidatorClient(
    rpc_url="http://localhost:8545",
    private_key="YOUR_PRIVATE_KEY",
    discovery_urls=["http://localhost:8089"]  # Can specify multiple discovery services
)

# List all non-validated nodes
non_validated_nodes = validator.list_non_validated_nodes()

for node in non_validated_nodes:
    print(f"Node ID: {node.id}")
    print(f"Provider: {node.provider_address}")
    print(f"Address: {node.ip_address}:{node.port}")
    print(f"Active: {node.is_active}")
    print(f"Whitelisted: {node.is_provider_whitelisted}")
    print()

# Get count of non-validated nodes
count = validator.get_non_validated_count()
print(f"Total non-validated nodes: {count}")

# Get all nodes as dictionaries (includes compute specs)
all_nodes = validator.list_all_nodes_dict()
for node in all_nodes:
    if not node['is_validated']:
        print(f"Node {node['id']} needs validation")
        
        # Access compute specs if available
        if 'node' in node and 'compute_specs' in node['node']:
            specs = node['node']['compute_specs']
            if specs:
                print(f"  RAM: {specs.get('ram_mb', 'N/A')} MB")
                print(f"  Storage: {specs.get('storage_gb', 'N/A')} GB")
```

### Node Details

Each node returned by `list_non_validated_nodes()` has the following attributes:

- `id`: Unique identifier for the node
- `provider_address`: On-chain address of the node provider
- `ip_address`: IP address of the node
- `port`: Port number for node communication
- `compute_pool_id`: ID of the compute pool the node belongs to
- `is_validated`: Whether the node has been validated
- `is_active`: Whether the node is currently active
- `is_provider_whitelisted`: Whether the provider is whitelisted
- `is_blacklisted`: Whether the node is blacklisted
- `worker_p2p_id`: P2P identifier (optional)
- `last_updated`: Last update timestamp (optional)
- `created_at`: Creation timestamp (optional)

### Environment Variables

The validator client can be configured using environment variables:

```bash
export RPC_URL="http://localhost:8545"
export VALIDATOR_PRIVATE_KEY="your_private_key_here"
export DISCOVERY_URLS="http://localhost:8089,http://backup-discovery:8089"
```

### Example Scripts

See the `examples/` directory for complete examples:
- `validator_list_nodes.py`: Lists all non-validated nodes with detailed output

## Worker Client

[Documentation for WorkerClient...]

## Orchestrator Client

[Documentation for OrchestratorClient...]

## Development

### Running Tests

```bash
# Install development dependencies
pip install -r requirements-dev.txt

# Run tests
pytest tests/
```

### Building from Source

```bash
# Build in release mode
maturin build --release

# Build and install locally
maturin develop
``` 