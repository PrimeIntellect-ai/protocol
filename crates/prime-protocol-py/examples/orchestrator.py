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
    
    if not private_key:
        print("Error: PRIVATE_KEY_ORCHESTRATOR environment variable is required")
        return
    
    print(f"Initializing orchestrator client...")
    print(f"RPC URL: {rpc_url}")
    print(f"Discovery URLs: {discovery_urls}")
    print(f"Pool ID: {pool_id}")
    
    # Initialize and start the orchestrator
    orchestrator = OrchestratorClient(
        rpc_url=rpc_url,
        private_key=private_key,
        discovery_urls=discovery_urls,
    )
    
    print("Starting orchestrator client...")
    orchestrator.start(p2p_port=8180)
    print("Orchestrator client started")
    
    # Wait for P2P to initialize
    time.sleep(5)
    
    print("\nStarting orchestrator loop...")
    print("Press Ctrl+C to stop\n")
    
    # Track nodes in our pool
    pool_node_ids = set()
    
    try:
        while True:
            print(f"{'='*50}")
            print(f"Cycle at {time.strftime('%H:%M:%S')}")
            
            # Check for any pending messages first
            print("Checking for pending messages...")
            message = orchestrator.get_next_message()
            while message:
                peer_id = message['peer_id']
                msg_data = message.get('message', {})
                
                if msg_data.get('type') == 'general':
                    data = bytes(msg_data.get('data', []))
                    sender_type = "VALIDATOR" if message.get('is_sender_validator') else \
                                 "POOL_OWNER" if message.get('is_sender_pool_owner') else \
                                 "WORKER"
                    print(f"  📨 From {peer_id[:16]}... ({sender_type}): {data}")
                elif msg_data.get('type') == 'authentication_complete':
                    print(f"  ✓ Auth complete with {peer_id[:16]}...")
                else:
                    print(f"  📋 {msg_data.get('type')} from {peer_id[:16]}...")
                
                # Check for more messages
                message = orchestrator.get_next_message()
            
            # Get nodes in the pool
            pool_nodes = orchestrator.list_nodes_for_pool(pool_id)
            print(f"\nFound {len(pool_nodes)} nodes in pool {pool_id}")
            
            # Update our tracking of pool nodes
            pool_node_ids = {node.worker_p2p_id for node in pool_nodes if node.worker_p2p_id}
            
            # Process each node
            for node in pool_nodes:
                if not node.worker_p2p_id:
                    continue
                    
                try:
                    if not node.is_active and node.is_validated:
                        # Invite inactive but validated nodes
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
                    elif node.is_active:
                        # Send message to active nodes
                        print(f"  💬 Messaging {node.id[:8]}...")
                        orchestrator.send_message(
                            peer_id=node.worker_p2p_id,
                            multiaddrs=node.worker_p2p_addresses,
                            data=b"Hello from orchestrator!",
                        )
                except Exception as e:
                    print(f"  ❌ Error with {node.id[:8]}: {e}")
            
            # Check for messages throughout the wait period
            print(f"\nWaiting 30 seconds (checking for messages)...")
            end_time = time.time() + 30
            messages_during_wait = 0
            
            while time.time() < end_time:
                message = orchestrator.get_next_message()
                if message:
                    peer_id = message['peer_id']
                    msg_data = message.get('message', {})
                    
                    if msg_data.get('type') == 'general':
                        data = bytes(msg_data.get('data', []))
                        sender_type = "VALIDATOR" if message.get('is_sender_validator') else \
                                     "POOL_OWNER" if message.get('is_sender_pool_owner') else \
                                     "WORKER"
                        
                        # Check if message is from a pool node
                        if peer_id in pool_node_ids:
                            print(f"  📨 From pool node {peer_id[:16]}... ({sender_type}): {data}")
                        else:
                            print(f"  📨 From external {peer_id[:16]}... ({sender_type}): {data}")
                        messages_during_wait += 1
                else:
                    time.sleep(0.1)
            
            if messages_during_wait > 0:
                print(f"Received {messages_during_wait} messages during wait")
            print()
            
    except KeyboardInterrupt:
        print("\n\nShutting down orchestrator...")
        orchestrator.stop()
        print("Orchestrator stopped")

if __name__ == "__main__":
    main()