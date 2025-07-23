#!/usr/bin/env python3
"""
Example demonstrating orchestrator client with continuous node invitation and messaging loop.
"""

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
    
    # Orchestrator loop configuration - process every 10 seconds
    orchestrator_interval = 10
    pool_id = 0  # Example pool ID
    
    if not private_key:
        print("Error: PRIVATE_KEY_ORCHESTRATOR environment variable is required")
        return
    
    try:
        # Initialize the orchestrator client
        print(f"Initializing orchestrator client...")
        print(f"RPC URL: {rpc_url}")
        print(f"Discovery URLs: {discovery_urls}")
        print(f"Pool ID: {pool_id}")
        print(f"Orchestrator interval: {orchestrator_interval} seconds")
        
        orchestrator = OrchestratorClient(
            rpc_url=rpc_url,
            private_key=private_key,
            discovery_urls=discovery_urls
        )
        
        print("Starting orchestrator client...")
        orchestrator.start(p2p_port=8180)
        print("Orchestrator client started")
        
        # Wait for P2P connections to establish
        print("Waiting for P2P connections to establish...")
        time.sleep(5)
        
        # Continuous orchestrator loop
        print("\nStarting continuous orchestrator loop...")
        print("Press Ctrl+C to stop the orchestrator")
        
        while True:
            try:
                print(f"\n{'='*50}")
                print(f"Starting orchestrator cycle at {time.strftime('%Y-%m-%d %H:%M:%S')}")
                print(f"{'='*50}")
                
                # List nodes for the specific pool
                print(f"Fetching nodes for pool {pool_id}...")
                pool_nodes = orchestrator.list_nodes_for_pool(pool_id)
                
                if not pool_nodes:
                    print(f"No nodes found in pool {pool_id}")
                else:
                    print(f"Found {len(pool_nodes)} nodes in pool {pool_id}")
                    
                    invited_count = 0
                    messaged_count = 0
                    error_count = 0
                    
                    for i, node in enumerate(pool_nodes):
                        print(f"\nProcessing node {i+1}/{len(pool_nodes)}:")
                        print(f"  ID: {node.id}")
                        print(f"  Provider Address: {node.provider_address}")
                        print(f"  Validated: {node.is_validated}")
                        print(f"  Active: {node.is_active}")
                        
                        if not node.is_validated:
                            print(f"  ⚠ Node {node.id} is not validated, skipping")
                            continue
                            
                        if node.is_active is False:
                            # Invite inactive but validated nodes
                            try:
                                print(f"  📨 Inviting node {node.id}...")
                                orchestrator.invite_node(
                                    peer_id=node.worker_p2p_id,
                                    worker_address=node.id, 
                                    pool_id=pool_id,
                                    multiaddrs=node.worker_p2p_addresses,
                                    domain_id=0,  # todo: automatically fetch from contract
                                    orchestrator_url=None,  # todo: deprecate
                                    expiration_seconds=1000  
                                )
                                print(f"  ✓ Node {node.id} invited successfully")
                                invited_count += 1
                            except Exception as e:
                                print(f"  ✗ Error inviting node {node.id}: {e}")
                                error_count += 1
                        else:
                            # Send message to active nodes
                            try:
                                print(f"  💬 Sending message to active node {node.id}...")
                                orchestrator.send_message(
                                    peer_id=node.worker_p2p_id,
                                    multiaddrs=node.worker_p2p_addresses,
                                    data=b"Hello, world!",
                                )
                                print(f"  ✓ Message sent to node {node.id}")
                                messaged_count += 1
                            except Exception as e:
                                print(f"  ✗ Error sending message to node {node.id}: {e}")
                                error_count += 1
                
                # Get summary statistics
                active_count = sum(1 for node in pool_nodes if node.is_active)
                inactive_count = sum(1 for node in pool_nodes if not node.is_active and node.is_validated)
                unvalidated_count = sum(1 for node in pool_nodes if not node.is_validated)
                
                print(f"\nOrchestrator cycle summary:")
                print(f"  Total nodes in pool: {len(pool_nodes)}")
                print(f"  Active nodes: {active_count}")
                print(f"  Inactive (validated) nodes: {inactive_count}")
                print(f"  Unvalidated nodes: {unvalidated_count}")
                print(f"  Nodes invited: {invited_count}")
                print(f"  Messages sent: {messaged_count}")
                print(f"  Errors: {error_count}")
                
                # Wait before next orchestrator cycle
                print(f"\nWaiting {orchestrator_interval} seconds before next orchestrator cycle...")
                time.sleep(orchestrator_interval)
                
            except KeyboardInterrupt:
                print("\n\nReceived interrupt signal. Shutting down orchestrator...")
                break
            except Exception as e:
                logging.error(f"Error during orchestrator cycle: {e}")
                print(f"Waiting {orchestrator_interval} seconds before retrying...")
                time.sleep(orchestrator_interval)
        
        print("Orchestrator stopped")
    
    except Exception as e:
        logging.error(f"Fatal error: {e}")
        raise
    finally:
        # Stop the orchestrator
        try:
            orchestrator.stop()
        except:
            pass

if __name__ == "__main__":
    main()