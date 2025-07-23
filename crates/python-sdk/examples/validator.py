#!/usr/bin/env python3
"""Example usage of the Prime Protocol Validator Client."""

import os
import logging
import time
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
    p2p_port = int(os.getenv("VALIDATOR_P2P_PORT", "8665"))
    
    if not private_key:
        print("Error: PRIVATE_KEY_VALIDATOR environment variable is required")
        return
    
    print(f"Initializing validator client...")
    print(f"RPC URL: {rpc_url}")
    print(f"Discovery URLs: {discovery_urls}")
    print(f"P2P Port: {p2p_port}")
    
    # Initialize and start the validator
    validator = ValidatorClient(
        rpc_url=rpc_url,
        private_key=private_key,
        discovery_urls=discovery_urls,
    )
    
    print("Starting validator client...")
    validator.start(p2p_port=p2p_port)
    print(f"Validator started with peer ID: {validator.get_peer_id()}")
    
    print("\nStarting validator loop...")
    print("Press Ctrl+C to stop\n")
    
    try:
        while True:
            print(f"{'='*50}")
            print(f"Cycle at {time.strftime('%H:%M:%S')}")
            # Check for a single message
            message = validator.get_next_message()
            print(f"Message: {message}")
            if message:
                msg_data = message.get('message', {})
                if msg_data.get('type') == 'general':
                    data = bytes(msg_data.get('data', []))
                    print(f"Message payload: {data}")
            non_validated = validator.list_non_validated_nodes()
            if non_validated:
                print(f"\nValidating {len(non_validated)} nodes...")
                for node in non_validated:
                    try:
                        print(f"  ✅ Validating {node.id[:8]}...")
                        validator.validate_node(node.id, node.provider_address)
                    except Exception as e:
                        print(f"  ❌ Error validating {node.id[:8]}: {e}")
            else:
                print("\nNo nodes to validate")
            
            # 2. Send messages to validated nodes
            all_nodes = validator.list_all_nodes_dict()
            validated = [n for n in all_nodes if n.get('is_validated') and n.get('worker_p2p_id')]
            
            if validated:
                print(f"\nMessaging {len(validated)} validated nodes...")
                for node in validated:
                    try:
                        validator.send_message(
                            peer_id=node['worker_p2p_id'],
                            multiaddrs=node.get('worker_p2p_addresses', []),
                            data=b"Hello from validator!",
                        )
                        print(f"  💬 Sent to {node['id'][:8]}...")
                    except Exception as e:
                        print(f"  ❌ Error messaging {node['id'][:8]}: {e}")
            
            # Wait before next cycle
            print(f"\nWaiting 10 seconds...")
            time.sleep(10)
            print()
            
    except KeyboardInterrupt:
        print("\n\nShutting down validator...")
        # ValidatorClient doesn't have a stop() method
        print("Validator stopped")

if __name__ == "__main__":
    main()