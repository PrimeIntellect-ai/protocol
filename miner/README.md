

## Miner Node

### Quick Start
``` bash
# Run the miner
cargo run -- run \
  --subnet-id <subnet-id> \
  --wallet-address 0x... \  # Your Ethereum wallet address (42 characters starting with 0x)
  --private-key ./keys/eth-private-key.json \  # Path to your Ethereum keystore file
  --port 8080 \  
  --external-ip <your-public-ip> \  # Your node's public IP address
```

