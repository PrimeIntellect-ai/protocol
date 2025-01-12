

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

### Run Hardware check 
``` 
# Run all checks
cargo run -- check

# Hardware checks only
cargo run -- check --hardware-only

# Software checks only 
cargo run -- check --software-only 

```
### Develop on GPU Server

The miner can be deployed and run on a remote GPU server using the provided Makefile commands.

1. Set up SSH connection details:
```
export SSH_CONN="root@78.130.201.2 -p 10100 -i private_key.pem"
```

2. Install required dependencies on the GPU server:
```
make gpu-setup
```

3. Deploy and run the miner:
```
make gpu-run 
```

You can also use `make gpu-watch` to automatically redeploy and run when files change.
