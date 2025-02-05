SSH_CONN ?=

.PHONY: gpu-setup gpu-run gpu-watch run

# Run locally
run-check:
	@echo "🔨 Building locally..."
	cargo build --release
	@echo "▶️  Running locally..."
	./target/release/miner check 

run:
	@echo "🔨 Building locally..."
	cargo run -- run --subnet-id 1 --private-key-provider $$PRIVATE_KEY_PROVIDER --private-key-node $$NODE_KEY --port 8090 --external-ip 0.0.0.0 --compute-pool-id 0 --validator-address $$VALIDATOR_ADDRESS

watch:
	@echo "👀 Watching for changes..."
	cargo watch -x "run -- run --subnet-id 1 --private-key-provider $$PRIVATE_KEY_PROVIDER --private-key-node $$NODE_KEY --port 8090 --external-ip 0.0.0.0 --compute-pool-id 0 --validator-address $$VALIDATOR_ADDRESS"

# Setup GPU server with required dependencies
gpu-setup:
	@if [ -z "$(SSH_CONN)" ]; then \
		echo "Error: SSH_CONN not set. Use: make gpu-setup SSH_CONN='user@ip -p port -i key.pem'"; \
		exit 1; \
	fi
	@echo "🔧 Setting up GPU server..."
	ssh $(SSH_CONN) 'command -v sudo >/dev/null 2>&1 || (apt-get update && apt-get install -y sudo)'
	ssh $(SSH_CONN) 'sudo apt-get update && sudo apt-get install -y rsync'
	ssh $(SSH_CONN) 'which cargo || curl -sSf https://sh.rustup.rs | sh -s -- -y'
	ssh $(SSH_CONN) 'mkdir -p ~/miner'

# Run on GPU server and stream output locally
gpu-run:
	@if [ -z "$(SSH_CONN)" ]; then \
		echo "Error: SSH_CONN not set. Use: make gpu-run SSH_CONN='user@ip -p port -i key.pem'"; \
		exit 1; \
	fi
	@echo "🚀 Deploying to GPU server..."
	rsync -avz --exclude 'target/' --exclude '.git/' -e "ssh $(SSH_CONN)" ./ $(SSH_CONN | cut -d@ -f2 | cut -d' ' -f1):~/miner/
	@echo "🔨 Building on GPU server..."
	ssh $(SSH_CONN) 'cd ~/miner && . ~/.cargo/env && LD_LIBRARY_PATH=/usr/lib cargo build --release'
	@echo "▶️  Running on GPU..."
	ssh -t $(SSH_CONN) 'cd ~/miner && LD_LIBRARY_PATH=/usr/lib ./target/release/miner $(MINER_CMD)'

# Watch mode - automatically deploy and run on file changes
gpu-watch:
	cargo watch -x "make gpu-run"
