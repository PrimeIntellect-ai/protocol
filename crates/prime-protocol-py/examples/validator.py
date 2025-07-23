#!/usr/bin/env python3
"""Example usage of the Prime Protocol Validator Client to continuously validate nodes."""

import os
import logging
import time
from typing import List
from primeprotocol import ValidatorClient

# Configure logging
FORMAT = '%(levelname)s %(name)s %(asctime)-15s %(filename)s:%(lineno)d %(message)s'
logging.basicConfig(format=FORMAT)
logging.getLogger().setLevel(logging.INFO)

def main():
    # Get configuration from environment variables
    rpc_url = os.getenv("RPC_URL", "http://localhost:8545")
    private_key = os.getenv("PRIVATE_KEY_VALIDATOR")
    discovery_urls_str = os.getenv("DISCOVERY_URLS", "http://localhost:8089")
    discovery_urls = [url.strip() for url in discovery_urls_str.split(",")]

    # Validation loop configuration - validate every 10 seconds
    validation_interval = 10
    
    if not private_key:
        print("Error: VALIDATOR_PRIVATE_KEY environment variable is required")
        return
    
    try:
        # Initialize the validator client
        print(f"Initializing validator client...")
        print(f"RPC URL: {rpc_url}")
        print(f"Discovery URLs: {discovery_urls}")
        print(f"Validation interval: {validation_interval} seconds")
        
        validator = ValidatorClient(
            rpc_url=rpc_url,
            private_key=private_key,
            discovery_urls=discovery_urls,
        )
        print("Starting validator client...")
        validator.start()
        print("Validator client started")
        
        # Continuous validation loop
        print("\nStarting continuous validation loop...")
        print("Press Ctrl+C to stop the validator")
        
        while True:
            try:
                print(f"\n{'='*50}")
                print(f"Starting validation cycle at {time.strftime('%Y-%m-%d %H:%M:%S')}")
                print(f"{'='*50}")
                
                # List all non-validated nodes
                print("Fetching non-validated nodes from discovery service...")
                non_validated_nodes = validator.list_non_validated_nodes()
                
                if not non_validated_nodes:
                    print("No non-validated nodes found")
                else:
                    print(f"Found {len(non_validated_nodes)} non-validated nodes")
                    
                    for node in non_validated_nodes:
                        print(f"Processing node {node.id}...")
                        if node.is_validated is False:
                            print(f"  Validating node {node.id}...")
                            validator.validate_node(node.id, node.provider_address)
                            print(f"  ✓ Node {node.id} validated successfully")
                        else:
                            print(f"  ℹ Node {node.id} is already validated")
                
                # Get summary statistics
                all_nodes = validator.list_all_nodes_dict()
                validated_count = sum(1 for node in all_nodes if node['is_validated'])
                non_validated_count = len(all_nodes) - validated_count
                
                print(f"\nValidation cycle summary:")
                print(f"  Total nodes: {len(all_nodes)}")
                print(f"  Validated: {validated_count}")
                print(f"  Non-validated: {non_validated_count}")
                
                # Wait before next validation cycle
                print(f"\nWaiting {validation_interval} seconds before next validation cycle...")
                time.sleep(validation_interval)
                
            except KeyboardInterrupt:
                print("\n\nReceived interrupt signal. Shutting down validator...")
                break
            except Exception as e:
                logging.error(f"Error during validation cycle: {e}")
                print(f"Waiting {validation_interval} seconds before retrying...")
                time.sleep(validation_interval)
        
        print("Validator stopped")
    
    except Exception as e:
        logging.error(f"Fatal error: {e}")
        raise


if __name__ == "__main__":
    main()