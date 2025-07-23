#!/usr/bin/env python3
"""Example usage of the Prime Protocol Orchestrator Client."""

import os
import logging
import time
from primeprotocol import OrchestratorClient

# Configure logging
FORMAT = '%(levelname)s %(name)s %(asctime)-15s %(filename)s:%(lineno)d %(message)s'
logging.basicConfig(format=FORMAT)
logging.getLogger().setLevel(logging.INFO)

def main():
    # Get configuration from environment variables
    rpc_url = os.getenv("RPC_URL", "http://localhost:8545")
    private_key = os.getenv("PRIVATE_KEY_ORCHESTRATOR")
    discovery_urls_str = os.getenv("DISCOVERY_URLS", "http://localhost:8089")
    discovery_urls = [url.strip() for url in discovery_urls_str.split(",")]
    pool_id = int(os.getenv("POOL_ID", "0"))
    p2p_port = int(os.getenv("ORCHESTRATOR_P2P_PORT", "8180"))
    
    if not private_key:
        print("Error: PRIVATE_KEY_ORCHESTRATOR environment variable is required")
        return
    
    print(f"Initializing orchestrator client...")
    print(f"RPC URL: {rpc_url}")
    print(f"Discovery URLs: {discovery_urls}")
    print(f"Pool ID: {pool_id}")
    print(f"P2P Port: {p2p_port}")
    
    # Initialize and start the orchestrator
    orchestrator = OrchestratorClient(
        rpc_url=rpc_url,
        private_key=private_key,
        discovery_urls=discovery_urls,
    )
    
    print("Starting orchestrator client...")
    orchestrator.start(p2p_port=p2p_port)
    print(f"Orchestrator started with peer ID: {orchestrator.get_peer_id()}")
    
    print("\nStarting orchestrator loop...")
    print("Press Ctrl+C to stop\n")
    
    try:
        while True:
            print(f"{'='*50}")
            print(f"Cycle at {time.strftime('%H:%M:%S')}")
            
            # Check for a single message
            message = orchestrator.get_next_message()
            print(f"Got message - python orchestrator: {message}")
            
            # 1. Invite validated but inactive nodes
            pool_nodes = orchestrator.list_nodes_for_pool(pool_id)
            validated_inactive = [n for n in pool_nodes if n.is_validated and not n.is_active and n.worker_p2p_id]
            
            if validated_inactive:
                print(f"\nInviting {len(validated_inactive)} validated inactive nodes...")
                for node in validated_inactive:
                    try:
                        print(f"  📨 Inviting {node.id[:8]}...")
                        orchestrator.invite_node(
                            peer_id=node.worker_p2p_id,
                            worker_address=node.id,
                            pool_id=pool_id,
                            multiaddrs=node.worker_p2p_addresses,
                            domain_id=0,
                            orchestrator_url=None,
                            expiration_seconds=1000
                        )
                    except Exception as e:
                        print(f"  ❌ Error inviting {node.id[:8]}: {e}")
            
            # 2. Send messages to active nodes
            active_nodes = [n for n in pool_nodes if n.is_active and n.worker_p2p_id]
            
            if active_nodes:
                print(f"\nMessaging {len(active_nodes)} active nodes...")
                for node in active_nodes:
                    try:
                        orchestrator.send_message(
                            peer_id=node.worker_p2p_id,
                            multiaddrs=node.worker_p2p_addresses,
                            data=b"Hello from orchestrator!",
                        )
                        print(f"  💬 Sent to {node.id[:8]}...")
                    except Exception as e:
                        print(f"  ❌ Error messaging {node.id[:8]}: {e}")
            
            # Wait before next cycle
            print(f"\nWaiting 10 seconds...")
            time.sleep(10)
            print()
            
    except KeyboardInterrupt:
        print("\n\nShutting down orchestrator...")
        orchestrator.stop()
        print("Orchestrator stopped")

if __name__ == "__main__":
    main()