SSH_CONN ?=

.PHONY: gpu-setup gpu-run gpu-watch

# Setup GPU server with required dependencies
gpu-setup:
	@if [ -z "$(SSH_CONN)" ]; then \
		echo "Error: SSH_CONN not set. Use: make gpu-setup SSH_CONN='user@ip -p port -i key.pem'"; \
		exit 1; \
	fi
	@echo "🔧 Setting up GPU server..."
	ssh $(SSH_CONN) 'apt-get update && apt-get install -y rsync'
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
	ssh $(SSH_CONN) 'cd ~/miner && . ~/.cargo/env && cargo build --release'
	@echo "▶️  Running on GPU..."
	ssh -t $(SSH_CONN) 'cd ~/miner && ./target/release/miner check'

# Watch mode - automatically deploy and run on file changes  
gpu-watch:
	cargo watch -x "make gpu-run"