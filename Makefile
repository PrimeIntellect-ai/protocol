SHELL := /bin/bash
ENV_FILE ?= .env
.PHONY: setup pool domain fund

set-min-stake-amount:
	set -a; source ${ENV_FILE}; set +a; \
	cargo run -p dev-utils --example set_min_stake_amount -- --min-stake-amount $${MIN_STAKE_AMOUNT} --key $${PRIVATE_KEY_FEDERATOR} --rpc-url $${RPC_URL}

mint-ai-tokens-to-provider:
	set -a; source ${ENV_FILE}; set +a; \
	cargo run -p dev-utils --example mint_ai_token -- --address $${PROVIDER_ADDRESS} --key $${PRIVATE_KEY_FEDERATOR} --rpc-url $${RPC_URL}

transfer-eth-to-provider:
	set -a; source ${ENV_FILE}; set +a; \
	cargo run -p dev-utils --example transfer_eth -- --address $${PROVIDER_ADDRESS} --key $${PRIVATE_KEY_FEDERATOR} --rpc-url $${RPC_URL} --amount 1000000000000000000

transfer-eth-to-pool-owner:
	set -a; source ${ENV_FILE}; set +a; \
	cargo run -p dev-utils --example transfer_eth -- --address $${POOL_OWNER_ADDRESS} --key $${PRIVATE_KEY_FEDERATOR} --rpc-url $${RPC_URL} --amount 1000000000000000000

create-domain:
	set -a; source ${ENV_FILE}; set +a; \
	cargo run -p dev-utils --example create_domain -- --domain-name "$${DOMAIN_NAME:-default_domain}" --domain-uri "$${DOMAIN_URI:-http://default.uri}" --key $${PRIVATE_KEY_FEDERATOR} --rpc-url $${RPC_URL}

create-training-domain:
	set -a; source ${ENV_FILE}; set +a; \
	cargo run -p dev-utils --example create_domain -- --domain-name "$${DOMAIN_NAME:-training}" --domain-uri "$${DOMAIN_URI:-http://default.uri}" --key $${PRIVATE_KEY_FEDERATOR} --rpc-url $${RPC_URL}

create-synth-data-domain:
	set -a; source ${ENV_FILE}; set +a; \
	cargo run -p dev-utils --example create_domain -- --domain-name "$${DOMAIN_NAME:-synth_data}" --domain-uri "$${DOMAIN_URI:-http://default.uri}" --key $${PRIVATE_KEY_FEDERATOR} --rpc-url $${RPC_URL}

create-compute-pool:
	set -a; source ${ENV_FILE}; set +a; \
	cargo run -p dev-utils --example compute_pool -- --domain-id "$${DOMAIN_ID:-0}" --compute-manager-key "$${POOL_OWNER_ADDRESS}" --pool-name "$${POOL_NAME:-default_pool}" --pool-data-uri "$${POOL_DATA_URI:-http://default.pool.data}" --key $${PRIVATE_KEY_FEDERATOR} --rpc-url $${RPC_URL}

start-compute-pool:
	set -a; source ${ENV_FILE}; set +a; \
	cargo run -p dev-utils --example start_compute_pool -- --key $${POOL_OWNER_PRIVATE_KEY} --rpc-url $${RPC_URL} --pool-id="$${POOL_ID:-0}"

setup: 
	make set-min-stake-amount
	make mint-ai-tokens-to-provider
	make transfer-eth-to-provider
	make transfer-eth-to-pool-owner
	make create-domain
	make create-compute-pool
	make start-compute-pool

setup-dev-env:
	make set-min-stake-amount
	make create-training-domain
	make create-synth-data-domain


up:
	tmuxinator start prime-dev
down:
	docker-compose down
	tmuxinator stop prime-dev
	pkill -f "target/debug/miner" 2>/dev/null || true
	pkill -f "target/debug/orchestrator" 2>/dev/null || true
	pkill -f "target/debug/validator" 2>/dev/null || true
	pkill -f "target/debug/discovery" 2>/dev/null || true

whitelist-provider:
	set -a; source ${ENV_FILE}; set +a; \
	cargo run -p dev-utils --example whitelist_provider -- --provider-address $${PROVIDER_ADDRESS} --key $${PRIVATE_KEY_VALIDATOR} --rpc-url $${RPC_URL}

watch-discovery:
	set -a; source .env; set +a; \
	cargo watch -w discovery/src -x "run --bin discovery -- --validator-address $${VALIDATOR_ADDRESS} --rpc-url $${RPC_URL}"

watch-miner:
	set -a; source ${ENV_FILE}; set +a; \
	cargo watch -w miner/src -x "run --bin miner -- run --private-key-provider $$PROVIDER_PRIVATE_KEY --private-key-node $$NODE_PRIVATE_KEY --port 8091 --external-ip 0.0.0.0 --compute-pool-id 0 --validator-address $$VALIDATOR_ADDRESS"

watch-validator:
	set -a; source ${ENV_FILE}; set +a; \
	cargo watch -w validator/src -x "run --bin validator -- --validator-key $${PRIVATE_KEY_VALIDATOR} --rpc-url $${RPC_URL} "

watch-orchestrator:
	set -a; source ${ENV_FILE}; set +a; \
	cargo watch -w orchestrator/src -x "run --bin orchestrator -- -r $$RPC_URL -k $$POOL_OWNER_PRIVATE_KEY -d 0  -p 8090 -i 10 -u http://localhost:8090"

build-miner:
	cargo build --release --bin miner

run-miner-bin:
	set -a; source .env; set +a; \
	./target/release/miner run --private-key-provider $$PROVIDER_PRIVATE_KEY --private-key-node $$NODE_PRIVATE_KEY --port 8091 --external-ip 0.0.0.0 --compute-pool-id 0 --validator-address $$VALIDATOR_ADDRESS

SSH_CONNECTION ?= your-ssh-conn string
EXTERNAL_IP ?= 0.0.0.0
PORT = 8091

# Remote setup
# Remote setup
.PHONY: setup-remote
setup-remote:
	$(SSH_CONNECTION) '\
		sudo apt-get update; \
		sudo apt-get install pkg-config libssl-dev; \
		if ! command -v rustc > /dev/null; then \
			curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y; \
			. "$$HOME/.cargo/env"; \
		fi; \
		. "$$HOME/.cargo/env"; \
		if ! command -v cargo-watch > /dev/null; then \
			cargo install cargo-watch; \
		fi'

# Setup SSH tunnel
.PHONY: setup-tunnel
setup-tunnel:
	$(SSH_CONNECTION) -f -N \
		-R 8545:localhost:8545 \
		-R 8090:localhost:8090 \
		-R 8089:localhost:8089

# Sync project to remote
.PHONY: sync-remote
sync-remote:
	rsync -avz -e "$(SSH_CONNECTION)" \
		--exclude 'target/' \
		--exclude '.git/' \
		--exclude 'node_modules/' \
		. :~/$(notdir $(CURDIR))

# Run miner on remote GPU
.PHONY: watch-miner-remote
watch-miner-remote: setup-remote setup-tunnel sync-remote
	$(SSH_CONNECTION) -t "cd ~/$(notdir $(CURDIR)) && \
		export PATH=\"\$$HOME/.cargo/bin:\$$PATH\" && \
		. \"\$$HOME/.cargo/env\" && \
		set -a && source .env && set +a && \
		export EXTERNAL_IP=$(EXTERNAL_IP) && \
		RUST_BACKTRACE=1 RUST_LOG=debug cargo watch -w miner/src -x \"run --bin miner -- run \
			--private-key-provider \$$PROVIDER_PRIVATE_KEY \
			--private-key-node \$$NODE_PRIVATE_KEY \
			--port $(PORT) \
			--external-ip \$$EXTERNAL_IP \
			--compute-pool-id 0 \
			--validator-address \$$VALIDATOR_ADDRESS  2>&1 | tee miner.log\""
# Kill SSH tunnel
.PHONY: kill-tunnel
kill-tunnel:
	pkill -f "ssh.*$(SSH_HOST).*-[NR]" || true
	$(SSH_CONNECTION) "pkill -f \"sshd.*:8091\"" || true

# Full remote execution with cleanup
.PHONY: remote-miner
remote-miner:
	@trap 'make kill-tunnel' EXIT; \
	make watch-miner-remote
