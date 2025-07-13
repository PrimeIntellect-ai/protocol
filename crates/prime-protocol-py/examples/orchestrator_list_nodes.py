#!/usr/bin/env python3
"""
Example demonstrating how to list nodes for a specific pool using the OrchestratorClient.
"""

import os
import signal
import sys
from time import sleep
from primeprotocol import OrchestratorClient

def signal_handler(sig, frame):
    print('\nShutting down gracefully...')
    sys.exit(0)

def main():
    # Set up signal handler for Ctrl+C
    signal.signal(signal.SIGINT, signal_handler)
    
    # Replace with your actual RPC URL and private key
    RPC_URL = "http://localhost:8545"
    PRIVATE_KEY = os.getenv("ORCHESTRATOR_PRIVATE_KEY")
    DISCOVERY_URLS = ["http://localhost:8089"]  # Discovery service URLs
    
    # Create orchestrator client
    orchestrator = OrchestratorClient(
        rpc_url=RPC_URL,
        private_key=PRIVATE_KEY,
        discovery_urls=DISCOVERY_URLS
    )
    
    try:
        # Initialize the orchestrator (without P2P for this example)
        orchestrator.start(p2p_port=8180)
        # todo: temp fix for establishing p2p connections
        sleep(5)
        
        # List nodes for a specific pool (example pool ID: 0)
        pool_id = 0
        pool_nodes = orchestrator.list_nodes_for_pool(pool_id)
        print(f"Nodes in pool {pool_id}: {len(pool_nodes)}")
        
        # Print details of all nodes in the pool
        for i, node in enumerate(pool_nodes):
            print(f"\nNode {i+1}:")
            print(f"  ID: {node.id}")
            print(f"  Provider Address: {node.provider_address}")
            print(f"  IP Address: {node.ip_address}")
            print(f"  Port: {node.port}")
            print(f"  Pool ID: {node.compute_pool_id}")
            print(f"  Validated: {node.is_validated}")
            print(f"  Worker P2P Addresses: {node.worker_p2p_addresses}")
            print(f"  Active: {node.is_active}")
            if node.worker_p2p_id:
                print(f"  Worker P2P ID: {node.worker_p2p_id}")
            if node.is_active is False:
                # Invite node with required parameters
                orchestrator.invite_node(
                    peer_id=node.worker_p2p_id,
                    worker_address=node.id, 
                    pool_id=pool_id,
                    multiaddrs=node.worker_p2p_addresses,
                    # todo: automatically fetch from contract
                    domain_id=0,  
                    # tood: deprecate
                    orchestrator_url=None,  
                    expiration_seconds=1000  
                )
            else:
                try:
                    # todo: we need an actual ack
                    orchestrator.send_message(
                        peer_id=node.worker_p2p_id,
                        multiaddrs=node.worker_p2p_addresses,
                        data=b"Hello, world!",
                    )
                    print(f"Message sent to node {node.id}")
                except Exception as e:
                    print(f"Error sending message to node {node.id}: {e}")
        
        print("\nPress Ctrl+C to exit...")
        
        # Keep the program running until Ctrl+C
        while True:
            try:
                signal.pause()
            except AttributeError:
                # signal.pause() is not available on Windows
                import time
                time.sleep(1)
                
    except KeyboardInterrupt:
        print('\nShutting down gracefully...')
    finally:
        # Stop the orchestrator
        orchestrator.stop()

if __name__ == "__main__":
    main() 