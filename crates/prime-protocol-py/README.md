# Prime Protocol Python Client

Python bindings for checking if compute pools exist.

## Build

```bash
# Install uv (one-time)
curl -LsSf https://astral.sh/uv/install.sh | sh

# Setup and build
cd crates/prime-protocol-py
make install
```

## Usage

```python
from primeprotocol import PrimeProtocolClient

client = PrimeProtocolClient("http://localhost:8545")
exists = client.compute_pool_exists(0)
```

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