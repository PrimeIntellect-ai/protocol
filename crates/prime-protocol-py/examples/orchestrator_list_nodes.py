#!/usr/bin/env python3
"""
Example demonstrating how to list nodes for a specific pool using the OrchestratorClient.
"""

import os
from primeprotocol import OrchestratorClient

def main():
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
    
    # Initialize the orchestrator (without P2P for this example)
    orchestrator.start()
    
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
        print(f"  Active: {node.is_active}")
        if node.worker_p2p_id:
            print(f"  Worker P2P ID: {node.worker_p2p_id}")
    
    # Stop the orchestrator
    orchestrator.stop()

if __name__ == "__main__":
    main() 