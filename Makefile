
.PHONY: setup pool domain fund

mint-ai-tokens-to-provider:
	cargo run -p dev-utils --example mint_ai_token -- --address $${PROVIDER_ADDRESS} --key $${PRIVATE_KEY_FEDERATOR} --rpc-url $${RPC_URL}

setup-provider:
	set -a; source .env; set +a; \
	cargo run -p dev-utils --example mint_ai_token -- --address $${PROVIDER_ADDRESS} --key $${PRIVATE_KEY_FEDERATOR} --rpc-url $${RPC_URL}

transfer-eth-to-provider:
	set -a; source .env; set +a; \
	cargo run -p dev-utils --example transfer_eth -- --address $${PROVIDER_ADDRESS} --key $${PRIVATE_KEY_FEDERATOR} --rpc-url $${RPC_URL} --amount 1000000000000000000

transfer-eth-to-pool-owner:
	set -a; source .env; set +a; \
	cargo run -p dev-utils --example transfer_eth -- --address $${POOL_OWNER_ADDRESS} --key $${PRIVATE_KEY_FEDERATOR} --rpc-url $${RPC_URL} --amount 1000000000000000000

create-domain:
	@read -p "Enter domain name: " DOMAIN_NAME; \
	read -p "Enter domain URI: " DOMAIN_URI; \
	set -a; source .env; set +a; \
	cargo run -p dev-utils --example create_domain -- --domain-name "$$DOMAIN_NAME" --domain-uri "$$DOMAIN_URI" --key $${PRIVATE_KEY_FEDERATOR} --rpc-url $${RPC_URL}

create-compute-pool:
	@read -p "Enter domain ID: " DOMAIN_ID; \
	read -p "Enter pool name: " POOL_NAME; \
	read -p "Enter pool data URI: " POOL_DATA_URI; \
	set -a; source .env; set +a; \
	cargo run -p dev-utils --example compute_pool -- --domain-id "$$DOMAIN_ID" --compute-manager-key "$$POOL_OWNER_ADDRESS" --pool-name "$$POOL_NAME" --pool-data-uri "$$POOL_DATA_URI" --key $${POOL_OWNER_PRIVATE_KEY} --rpc-url $${RPC_URL}
