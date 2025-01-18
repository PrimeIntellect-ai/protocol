
.PHONY: setup pool domain fund

mint-tokens:
	cargo run -p dev-utils --example mint_ai_token

setup-provider:
	set -a; source .env; set +a; \
	cargo run -p dev-utils --example mint_ai_token -- --address $${PROVIDER_ADDRESS} --key $${PRIVATE_KEY_FEDERATOR} --rpc-url $${RPC_URL}

transfer-eth:
	set -a; source .env; set +a; \
	cargo run -p dev-utils --example transfer_eth -- --address $${PROVIDER_ADDRESS} --key $${PRIVATE_KEY_FEDERATOR} --rpc-url $${RPC_URL} --amount 1000000000000000000

create-domain:
	@read -p "Enter domain name: " DOMAIN_NAME; \
	read -p "Enter domain URI: " DOMAIN_URI; \
	set -a; source .env; set +a; \
	cargo run -p dev-utils --example create_domain -- --domain-name "$$DOMAIN_NAME" --domain-uri "$$DOMAIN_URI" --key $${PRIVATE_KEY_FEDERATOR} --rpc-url $${RPC_URL}
