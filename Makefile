SHELL := /bin/bash
ENV_FILE ?= .env
.PHONY: setup pool domain fund

mint-ai-tokens-to-provider:
	set -a; source ${ENV_FILE}; set +a; \
	cargo run -p dev-utils --example mint_ai_token -- --address $${PROVIDER_ADDRESS} --key $${PRIVATE_KEY_FEDERATOR} --rpc-url $${RPC_URL} 

test-concurrent-calls:
	set -a; source ${ENV_FILE}; set +a; \
	cargo run -p dev-utils --example test_concurrent_calls -- --address $${PROVIDER_ADDRESS} --key $${PRIVATE_KEY_FEDERATOR} --rpc-url $${RPC_URL}

mint-ai-tokens-to-federator:
	set -a; source ${ENV_FILE}; set +a; \
	cargo run -p dev-utils --example mint_ai_token -- --address $${FEDERATOR_ADDRESS} --key $${PRIVATE_KEY_FEDERATOR} --rpc-url $${RPC_URL} --amount 1000000000000000000

transfer-eth-to-provider:
	set -a; source ${ENV_FILE}; set +a; \
	cargo run -p dev-utils --example transfer_eth -- --address $${PROVIDER_ADDRESS} --key $${PRIVATE_KEY_FEDERATOR} --rpc-url $${RPC_URL} --amount 100000000000000000

transfer-eth-to-pool-owner:
	set -a; source ${ENV_FILE}; set +a; \
	cargo run -p dev-utils --example transfer_eth -- --address $${POOL_OWNER_ADDRESS} --key $${PRIVATE_KEY_FEDERATOR} --rpc-url $${RPC_URL} --amount 1000000000000000000

create-domain:
	set -a; source ${ENV_FILE}; set +a; \
	cargo run -p dev-utils --example create_domain -- --domain-name "$${DOMAIN_NAME:-default_domain}" --domain-uri "$${DOMAIN_URI:-http://default.uri}" --key $${PRIVATE_KEY_FEDERATOR} --validation-logic $${WORK_VALIDATION_CONTRACT} --rpc-url $${RPC_URL}

create-training-domain:
	set -a; source ${ENV_FILE}; set +a; \
	cargo run -p dev-utils --example create_domain -- --domain-name "$${DOMAIN_NAME:-training}" --domain-uri "$${DOMAIN_URI:-http://default.uri}" --key $${PRIVATE_KEY_FEDERATOR} --rpc-url $${RPC_URL} --validation-logic $${WORK_VALIDATION_CONTRACT}

create-synth-data-domain:
	set -a; source ${ENV_FILE}; set +a; \
	cargo run -p dev-utils --example create_domain -- --domain-name "$${DOMAIN_NAME:-synth_data}" --domain-uri "$${DOMAIN_URI:-http://default.uri}" --key $${PRIVATE_KEY_FEDERATOR} --rpc-url $${RPC_URL} --validation-logic $${WORK_VALIDATION_CONTRACT}

create-compute-pool:
	set -a; source ${ENV_FILE}; set +a; \
	cargo run -p dev-utils --example compute_pool -- --domain-id "$${DOMAIN_ID:-0}" --compute-manager-key "$${POOL_OWNER_ADDRESS}" --pool-name "$${POOL_NAME:-default_pool}" --pool-data-uri "$${POOL_DATA_URI:-http://default.pool.data}" --key $${PRIVATE_KEY_FEDERATOR} --rpc-url $${RPC_URL}

start-compute-pool:
	set -a; source ${ENV_FILE}; set +a; \
	cargo run -p dev-utils --example start_compute_pool -- --key $${POOL_OWNER_PRIVATE_KEY} --rpc-url $${RPC_URL} --pool-id="$${POOL_ID:-0}"

setup: 
	make mint-ai-tokens-to-provider
	make transfer-eth-to-provider
	make transfer-eth-to-pool-owner
	make create-domain
	make create-compute-pool
	make start-compute-pool

setup-dev-env:
	make create-training-domain
	make create-synth-data-domain
	make mint-ai-tokens-to-federator
	make transfer-eth-to-pool-owner


# Start development environment
.PHONY: up
up:
	@echo "Starting Prime development environment..."
	@# Start Docker services
	@docker compose up -d reth redis --wait --wait-timeout 180
	@# Deploy contracts
	@cd smart-contracts && sh deploy.sh && sh deploy_work_validation.sh && cd ..
	@# Run setup
	@$(MAKE) setup
	@# Kill any existing session
	@tmux kill-session -t prime-dev 2>/dev/null || true
	@# Create new tmux session
	@tmux new-session -d -s prime-dev -n services
	@# Enable pane titles and borders
	@tmux set -t prime-dev pane-border-status top
	@tmux set -t prime-dev pane-border-format " #{pane_title} "
	@# Start Worker pane first (pane 0)
	@tmux select-pane -t prime-dev:services.0 -T "Worker"
	@# Discovery pane (pane 1)
	@tmux split-window -h -t prime-dev:services
	@tmux select-pane -t prime-dev:services.1 -T "Discovery"
	@tmux send-keys -t prime-dev:services.1 'make watch-discovery' C-m
	@# Validator pane (pane 2)
	@tmux split-window -h -t prime-dev:services.1
	@tmux select-pane -t prime-dev:services.2 -T "Validator"
	@tmux send-keys -t prime-dev:services.2 'make watch-validator' C-m
	@# Orchestrator pane (pane 3)
	@tmux split-window -h -t prime-dev:services.2
	@tmux select-pane -t prime-dev:services.3 -T "Orchestrator"
	@tmux send-keys -t prime-dev:services.3 'make watch-orchestrator' C-m
	@tmux select-layout -t prime-dev:services even-horizontal
	@# Create background window for docker logs
	@tmux new-window -t prime-dev -n background
	@tmux send-keys -t prime-dev:background 'docker compose logs -f reth redis' C-m
	@# Switch back to first window before attaching
	@tmux select-window -t prime-dev:services
	@# Attach to session
	@tmux attach-session -t prime-dev

# Start Docker services and deploy contracts only
.PHONY: bootstrap
bootstrap:
	@echo "Starting Docker services and deploying contracts..."
	@# Start Docker services
	@docker compose up -d reth redis discovery --wait --wait-timeout 180
	@# Deploy contracts
	@cd smart-contracts && sh deploy.sh && sh deploy_work_validation.sh && cd ..
	@# Run setup
	@$(MAKE) setup
	@echo "Bootstrap complete - Docker services running and contracts deployed"

# Stop development environment
.PHONY: down
down:
	@docker compose down
	@tmux kill-session -t prime-dev 2>/dev/null || true
	@pkill -f "target/debug/worker" 2>/dev/null || true
	@pkill -f "target/debug/orchestrator" 2>/dev/null || true
	@pkill -f "target/debug/validator" 2>/dev/null || true
	@pkill -f "target/debug/discovery" 2>/dev/null || true
	@pkill -9 -f "cargo run --bin discovery" 2>/dev/null || true
	@pkill -9 -f "cargo watch" 2>/dev/null || true

# Whitelist provider
.PHONY: whitelist-provider
whitelist-provider:
	set -a; source ${ENV_FILE}; set +a; \
	cargo run -p dev-utils --example whitelist_provider -- --provider-address $${PROVIDER_ADDRESS} --key $${PRIVATE_KEY_VALIDATOR} --rpc-url $${RPC_URL}

watch-discovery:
	set -a; source .env; set +a; \
	cargo watch -w crates/discovery/src -x "run --bin discovery -- --rpc-url $${RPC_URL} --max-nodes-per-ip $${MAX_NODES_PER_IP:-2} $${LOCATION_SERVICE_URL:+--location-service-url $${LOCATION_SERVICE_URL}} $${LOCATION_SERVICE_API_KEY:+--location-service-api-key $${LOCATION_SERVICE_API_KEY}}"

watch-worker:
	set -a; source ${ENV_FILE}; set +a; \
	cargo watch -w crates/worker/src -x "run --bin worker -- run --port 8091 --discovery-url $${DISCOVERY_URLS:-$${DISCOVERY_URL:-http://localhost:8089}} --compute-pool-id $$WORKER_COMPUTE_POOL_ID --skip-system-checks $${LOKI_URL:+--loki-url $${LOKI_URL}} --log-level $${LOG_LEVEL:-info}"

watch-worker-two:
	set -a; source ${ENV_FILE}; set +a; \
	cargo watch -w crates/worker/src -x "run --bin worker -- run --port 8092 --discovery-url $${DISCOVERY_URLS:-$${DISCOVERY_URL:-http://localhost:8089}} --private-key-node $${PRIVATE_KEY_NODE_2} --private-key-provider $${PRIVATE_KEY_PROVIDER} --compute-pool-id $$WORKER_COMPUTE_POOL_ID --skip-system-checks $${LOKI_URL:+--loki-url $${LOKI_URL}} --log-level $${LOG_LEVEL:-info} --disable-state-storing --no-auto-recover"

watch-check:
	cargo watch -w crates/worker/src -x "run --bin worker -- check"	

watch-validator:
	set -a; source ${ENV_FILE}; set +a; \
	cargo watch -w crates/validator/src -x "run --bin validator -- --validator-key $${PRIVATE_KEY_VALIDATOR} --rpc-url $${RPC_URL} --discovery-urls $${DISCOVERY_URLS:-$${DISCOVERY_URL:-http://localhost:8089}} --pool-id $${WORKER_COMPUTE_POOL_ID} $${BUCKET_NAME:+--bucket-name $${BUCKET_NAME}} -l $${LOG_LEVEL:-info} --toploc-grace-interval $${TOPLOC_GRACE_INTERVAL:-30} --incomplete-group-grace-period-minutes $${INCOMPLETE_GROUP_GRACE_PERIOD_MINUTES:-1} --use-grouping"

watch-orchestrator:
	set -a; source ${ENV_FILE}; set +a; \
	cargo watch -w crates/orchestrator/src -x "run --bin orchestrator -- -r $$RPC_URL -k $$POOL_OWNER_PRIVATE_KEY -d 0  -p 8090 -i 10 -u http://localhost:8090 --discovery-urls $${DISCOVERY_URLS:-$${DISCOVERY_URL:-http://localhost:8089}} --compute-pool-id $$WORKER_COMPUTE_POOL_ID $${BUCKET_NAME:+--bucket-name $$BUCKET_NAME} -l $${LOG_LEVEL:-info} --hourly-s3-upload-limit $${HOURLY_S3_LIMIT:-3} --max-healthy-nodes-with-same-endpoint $${MAX_HEALTHY_NODES_WITH_SAME_ENDPOINT:-2}"

build-worker:
	cargo build --release --bin worker

run-worker-bin:
	@test -n "$$PRIVATE_KEY_PROVIDER" || (echo "PRIVATE_KEY_PROVIDER is not set" && exit 1)
	@test -n "$$PRIVATE_KEY_NODE" || (echo "PRIVATE_KEY_NODE is not set" && exit 1)
	set -a; source .env; set +a; \
	./target/release/worker run --port 8091 --external-ip 0.0.0.0 --compute-pool-id $$WORKER_COMPUTE_POOL_ID --validator-address $$VALIDATOR_ADDRESS

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
		fi; \
		if ! groups | grep -q docker; then \
			sudo usermod -aG docker $$USER; \
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

# Run worker on remote GPU
.PHONY: watch-worker-remote
watch-worker-remote: setup-remote setup-tunnel sync-remote
	$(SSH_CONNECTION) -t "cd ~/$(notdir $(CURDIR)) && \
		export PATH=\"\$$HOME/.cargo/bin:\$$PATH\" && \
		. \"\$$HOME/.cargo/env\" && \
		export TERM=xterm-256color && \
		bash --login -i -c '\
			set -a && source .env && set +a && \
			export EXTERNAL_IP=$(EXTERNAL_IP) && \
			clear && \
			RUST_BACKTRACE=1 RUST_LOG=debug cargo watch -w crates/worker/src -x \"run --bin worker -- run \
				--port $(PORT) \
				--compute-pool-id \$$WORKER_COMPUTE_POOL_ID \
				--auto-accept \
				2>&1 | tee worker.log\"'"

.PHONY: watch-worker-remote-two
watch-worker-remote-two: setup-remote setup-tunnel sync-remote
	$(SSH_CONNECTION) -t "cd ~/$(notdir $(CURDIR)) && \
		export PATH=\"\$$HOME/.cargo/bin:\$$PATH\" && \
		. \"\$$HOME/.cargo/env\" && \
		export TERM=xterm-256color && \
		bash --login -i -c '\
			set -a && source .env && set +a && \
			export EXTERNAL_IP=$(EXTERNAL_IP) && \
			clear && \
			RUST_BACKTRACE=1 RUST_LOG=debug cargo watch -w crates/worker/src -x \"run --bin worker -- run \
				--port $(PORT) \
				--compute-pool-id \$$WORKER_COMPUTE_POOL_ID \
				--auto-accept \
				--private-key-node \$$PRIVATE_KEY_NODE_2 \
				2>&1 | tee worker.log\"'"

# Kill SSH tunnel
.PHONY: kill-tunnel
kill-tunnel:
	pkill -f "ssh.*$(SSH_HOST).*-[NR]" || true
	$(SSH_CONNECTION) "pkill -f \"sshd.*:8091\"" || true

# Full remote execution with cleanup
.PHONY: remote-worker
remote-worker:
	@trap 'make kill-tunnel' EXIT; \
	make watch-worker-remote

.PHONY: remote-worker-two
remote-worker-two:
	@trap 'make kill-tunnel' EXIT; \
	make watch-worker-remote-two


# testing:
eject-node:
	set -a; source ${ENV_FILE}; set +a; \
	cargo run -p dev-utils --example eject_node -- --pool-id $${WORKER_COMPUTE_POOL_ID} --node $${NODE_ADDRESS} --provider-address $${PROVIDER_ADDRESS} --key $${POOL_OWNER_PRIVATE_KEY} --rpc-url $${RPC_URL}

sign-message:
	set -a; source ${ENV_FILE}; set +a; \
	cargo watch -w crates/worker/src -x "run --bin worker -- sign-message --message example-content --private-key-provider $$PRIVATE_KEY_PROVIDER --private-key-node $$PRIVATE_KEY_NODE"

balance:
	set -a; source ${ENV_FILE}; set +a; \
	cargo watch -w crates/worker/src -x "run --bin worker -- balance --private-key $$PRIVATE_KEY_PROVIDER --rpc-url $$RPC_URL"

get-node-info:
	set -a; source ${ENV_FILE}; set +a; \
	cargo run -p dev-utils --example get_node_info -- --provider-address $${PROVIDER_ADDRESS} --node-address $${NODE_ADDRESS} --key $${PRIVATE_KEY_FEDERATOR} --rpc-url $${RPC_URL}

submit-work:
	set -a; source ${ENV_FILE}; set +a; \
	cargo run -p dev-utils --example submit_work -- --pool-id $${POOL_ID:-0} --node $${NODE_ADDRESS} --work-key $${WORK_KEY} --key $${PRIVATE_KEY_PROVIDER} --rpc-url $${RPC_URL}

invalidate-work:
	set -a; source ${ENV_FILE}; set +a; \
	cargo run -p dev-utils --example invalidate_work -- --pool-id $${POOL_ID:-0} --penalty $${PENALTY} --work-key $${WORK_KEY} --key $${PRIVATE_KEY_VALIDATOR} --rpc-url $${RPC_URL}

deregister-worker:
	set -a; source ${ENV_FILE}; set +a; \
	cargo run --bin worker -- deregister --compute-pool-id $${WORKER_COMPUTE_POOL_ID} --private-key-provider $${PRIVATE_KEY_PROVIDER} --private-key-node $${PRIVATE_KEY_NODE} --rpc-url $${RPC_URL}

# Python Package
.PHONY: python-install
python-install:
	@cd crates/prime-protocol-py && make install

.PHONY: python-test
python-test:
	@cd crates/prime-protocol-py && make test

