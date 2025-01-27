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

create-compute-pool:
	set -a; source ${ENV_FILE}; set +a; \
	cargo run -p dev-utils --example compute_pool -- --domain-id "$${DOMAIN_ID:-0}" --compute-manager-key "$${POOL_OWNER_ADDRESS}" --pool-name "$${POOL_NAME:-default_pool}" --pool-data-uri "$${POOL_DATA_URI:-http://default.pool.data}" --key $${POOL_OWNER_PRIVATE_KEY} --rpc-url $${RPC_URL}

setup: 
	make set-min-stake-amount
	make mint-ai-tokens-to-provider
	make transfer-eth-to-provider
	make transfer-eth-to-pool-owner
	make create-domain
	make create-compute-pool

up:
	tmuxinator start prime-dev
down:
	docker-compose down
<<<<<<< HEAD
	tmuxinator stop prime-dev
	pkill -f "target/debug/miner"
	pkill -f "target/debug/orchestrator"
	pkill -f "target/debug/validator"
=======
	pkill -f "target/debug/miner" 2>/dev/null || true
	pkill -f "target/debug/orchestrator" 2>/dev/null || true
	pkill -f "target/debug/validator" 2>/dev/null || true
	pkill -f "target/debug/discovery" 2>/dev/null || true
>>>>>>> develop

whitelist-provider:
	set -a; source ${ENV_FILE}; set +a; \
	cargo run -p dev-utils --example whitelist_provider -- --provider-address $${PROVIDER_ADDRESS} --key $${PRIVATE_KEY_VALIDATOR} --rpc-url $${RPC_URL}

watch-discovery:
	set -a; source .env; set +a; \
	cargo watch -w discovery/src -x "run --bin discovery -- --validator-address $${VALIDATOR_ADDRESS} --rpc-url $${RPC_URL}"

watch-miner:
	set -a; source ${ENV_FILE}; set +a; \
	cargo watch -w miner/src -x "run --bin miner -- run --private-key-provider $$PROVIDER_PRIVATE_KEY --private-key-node $$NODE_PRIVATE_KEY --port 8091 --external-ip 0.0.0.0 --compute-pool-id 0"

watch-validator:
	set -a; source ${ENV_FILE}; set +a; \
	cargo watch -w validator/src -x "run --bin validator"

watch-orchestrator:
	set -a; source ${ENV_FILE}; set +a; \
	cargo watch -w orchestrator/src -x "run --bin orchestrator -- -r $$RPC_URL -k $$POOL_OWNER_PRIVATE_KEY -d 0 -e 0.0.0.0 -p 8090 -i 10"

# Release
build-miner:
	cargo build --release --bin miner

run-miner-bin:
	set -a; source .env; set +a; \
	./target/release/miner run --private-key-provider $$PROVIDER_PRIVATE_KEY --private-key-node $$NODE_PRIVATE_KEY --port 8091 --external-ip 0.0.0.0 --compute-pool-id 0 
