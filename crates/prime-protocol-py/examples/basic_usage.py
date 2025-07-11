#!/usr/bin/env python3
"""Example usage of the Prime Protocol Python client."""

import logging
import os
from primeprotocol import PrimeProtocolClient

FORMAT = '%(levelname)s %(name)s %(asctime)-15s %(filename)s:%(lineno)d %(message)s'
logging.basicConfig(format=FORMAT)
logging.getLogger().setLevel(logging.INFO)


def main():
    rpc_url = os.getenv("RPC_URL", "http://localhost:8545")
    pool_id = os.getenv("POOL_ID", 0)
    private_key_provider = os.getenv("PRIVATE_KEY_PROVIDER", None)
    private_key_node = os.getenv("PRIVATE_KEY_NODE", None)

    logging.info(f"Connecting to: {rpc_url}")
    client = PrimeProtocolClient(pool_id, rpc_url, private_key_provider, private_key_node)
    client.start()

if __name__ == "__main__":
    main() 