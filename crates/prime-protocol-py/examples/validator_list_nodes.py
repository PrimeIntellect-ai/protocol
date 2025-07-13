#!/usr/bin/env python3
"""Example usage of the Prime Protocol Validator Client to list non-validated nodes."""

import os
import logging
from typing import List
from primeprotocol import ValidatorClient

# Configure logging
FORMAT = '%(levelname)s %(name)s %(asctime)-15s %(filename)s:%(lineno)d %(message)s'
logging.basicConfig(format=FORMAT)
logging.getLogger().setLevel(logging.INFO)

def main():
    # Get configuration from environment variables
    rpc_url = os.getenv("RPC_URL", "http://localhost:8545")
    private_key = os.getenv("VALIDATOR_PRIVATE_KEY")
    discovery_urls_str = os.getenv("DISCOVERY_URLS", "http://localhost:8089")
    discovery_urls = [url.strip() for url in discovery_urls_str.split(",")]
    
    if not private_key:
        print("Error: VALIDATOR_PRIVATE_KEY environment variable is required")
        return
    
    try:
        # Initialize the validator client
        print(f"Initializing validator client...")
        print(f"RPC URL: {rpc_url}")
        print(f"Discovery URLs: {discovery_urls}")
        
        validator = ValidatorClient(
            rpc_url=rpc_url,
            private_key=private_key,
            discovery_urls=discovery_urls,
        )
        print("Starting validator client...")
        validator.start()
        print("Validator client started")
        
        # List all non-validated nodes
        print("\nFetching non-validated nodes from discovery service...")
        non_validated_nodes = validator.list_non_validated_nodes()
        for node in non_validated_nodes:
            print(node.id)
            if node.is_validated is False:
                print(f"Validating node {node.id}...")
                validator.validate_node(node.id, node.provider_address)
                print(f"Node {node.id} validated")
            else:
                print(f"Node {node.id} is already validated")
        
        # You can also get all nodes as dictionaries for more flexibility
        print("\n\nFetching all nodes as dictionaries...")
        all_nodes = validator.list_all_nodes_dict()
        print(all_nodes)
        
        # Count validated vs non-validated
        validated_count = sum(1 for node in all_nodes if node['is_validated'])
        non_validated_count = len(all_nodes) - validated_count
        
        print(f"\nTotal nodes: {len(all_nodes)}")
        print(f"Validated: {validated_count}")
        print(f"Non-validated: {non_validated_count}")
    
    except Exception as e:
        logging.error(f"Error: {e}")
        raise


if __name__ == "__main__":
    main() 