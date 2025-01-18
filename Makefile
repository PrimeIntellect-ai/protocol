
.PHONY: setup pool domain fund

mint-tokens:
	cargo run -p dev-utils --example mint_ai_token

setup-provider:
	set -a; source .env; set +a; \
	cargo run -p dev-utils --example mint_ai_token -- --address $${PROVIDER_ADDRESS} --key $${PRIVATE_KEY_FEDERATOR} --rpc-url $${RPC_URL}

transfer-eth:
	set -a; source .env; set +a; \
	cargo run -p dev-utils --example transfer_eth -- --address $${PROVIDER_ADDRESS} --key $${PRIVATE_KEY_FEDERATOR} --rpc-url $${RPC_URL} --amount 1000000000000000000