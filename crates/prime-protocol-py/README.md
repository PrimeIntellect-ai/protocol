# Prime Protocol Python Client

## Build

```bash
# Install uv (one-time)
curl -LsSf https://astral.sh/uv/install.sh | sh

# Setup and build
cd crates/prime-protocol-py
make install
```

## Usage

### Worker Client with Message Queue

The Worker Client provides a message queue system for handling P2P messages from pool owners and validators. Messages are processed in a FIFO (First-In-First-Out) manner.

```python
from primeprotocol import WorkerClient
import asyncio

# Initialize the worker client
client = WorkerClient(
    compute_pool_id=1,
    rpc_url="http://localhost:8545",
    private_key_provider="your_provider_key",
    private_key_node="your_node_key",
)

# Start the client (registers on-chain and starts message listener)
client.start()

# Poll for messages in your application loop
async def process_messages():
    while True:
        # Get next message from pool owner queue
        pool_msg = client.get_pool_owner_message()
        if pool_msg:
            print(f"Pool owner message: {pool_msg}")
            # Process the message...
        
        # Get next message from validator queue
        validator_msg = client.get_validator_message()
        if validator_msg:
            print(f"Validator message: {validator_msg}")
            # Process the message...
        
        await asyncio.sleep(0.1)

# Run the message processing loop
asyncio.run(process_messages())

# Gracefully shutdown
client.stop()
```

### Message Queue Features

- **Background Listener**: Rust protocol listens for P2P messages in the background
- **FIFO Queue**: Messages are processed in the order they are received
- **Message Types**: Separate queues for pool owner, validator, and system messages
- **Mock Mode**: Currently generates mock messages for testing (P2P integration coming soon)
- **Thread-Safe**: Safe to use from async Python code

See `examples/message_queue_example.py` for a complete working example.

## Development

```bash
make build         # Build development version
make test          # Run tests
make example       # Run example
make clean         # Clean artifacts
make help          # Show all commands
```

## Installing in other projects

```bash
# Build the wheel
make build-release

# Install with uv (recommended)
uv pip install target/wheels/primeprotocol-*.whl

# Or install directly from source
uv pip install /path/to/prime-protocol-py/
``` 