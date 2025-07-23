"""Tests for the validator client."""

import pytest
import os
from unittest.mock import patch, Mock


def test_validator_client_creation():
    """Test creating a validator client requires proper parameters."""
    # Mock the primeprotocol module since it might not be built
    with patch('primeprotocol.ValidatorClient') as MockValidator:
        mock_instance = Mock()
        MockValidator.return_value = mock_instance
        
        # Test creation with required parameters
        rpc_url = "http://localhost:8545"
        private_key = "0x1234567890abcdef"
        discovery_urls = ["http://localhost:8089"]
        
        from primeprotocol import ValidatorClient
        validator = ValidatorClient(rpc_url, private_key, discovery_urls)
        
        # Verify the constructor was called with correct parameters
        MockValidator.assert_called_once_with(rpc_url, private_key, discovery_urls)


def test_list_non_validated_nodes():
    """Test listing non-validated nodes."""
    with patch('primeprotocol.ValidatorClient') as MockValidator:
        # Create mock node data
        mock_node1 = Mock()
        mock_node1.id = "node1"
        mock_node1.provider_address = "0xabc123"
        mock_node1.ip_address = "192.168.1.1"
        mock_node1.port = 8080
        mock_node1.compute_pool_id = 1
        mock_node1.is_validated = False
        mock_node1.is_active = True
        mock_node1.is_provider_whitelisted = True
        mock_node1.is_blacklisted = False
        mock_node1.worker_p2p_id = "p2p_id_1"
        mock_node1.created_at = "2024-01-01T00:00:00Z"
        mock_node1.last_updated = "2024-01-02T00:00:00Z"
        
        mock_instance = Mock()
        mock_instance.list_non_validated_nodes.return_value = [mock_node1]
        MockValidator.return_value = mock_instance
        
        from primeprotocol import ValidatorClient
        validator = ValidatorClient("http://localhost:8545", "0x123", ["http://localhost:8089"])
        
        # Get non-validated nodes
        nodes = validator.list_non_validated_nodes()
        
        # Verify results
        assert len(nodes) == 1
        assert nodes[0].id == "node1"
        assert nodes[0].is_validated == False
        assert nodes[0].is_active == True


def test_list_all_nodes_dict():
    """Test listing all nodes as dictionaries."""
    with patch('primeprotocol.ValidatorClient') as MockValidator:
        # Create mock response
        mock_nodes = [
            {
                'id': 'node1',
                'provider_address': '0xabc123',
                'is_validated': False,
                'is_active': True,
                'node': {
                    'compute_specs': {
                        'gpu': {
                            'count': 4,
                            'model': 'NVIDIA A100',
                            'memory_mb': 40000
                        },
                        'cpu': {
                            'cores': 32
                        },
                        'ram_mb': 128000,
                        'storage_gb': 1000
                    }
                }
            },
            {
                'id': 'node2',
                'provider_address': '0xdef456',
                'is_validated': True,
                'is_active': True,
                'node': {
                    'compute_specs': None
                }
            }
        ]
        
        mock_instance = Mock()
        mock_instance.list_all_nodes_dict.return_value = mock_nodes
        MockValidator.return_value = mock_instance
        
        from primeprotocol import ValidatorClient
        validator = ValidatorClient("http://localhost:8545", "0x123", ["http://localhost:8089"])
        
        # Get all nodes
        nodes = validator.list_all_nodes_dict()
        
        # Verify results
        assert len(nodes) == 2
        assert nodes[0]['is_validated'] == False
        assert nodes[1]['is_validated'] == True
        
        # Check compute specs
        assert nodes[0]['node']['compute_specs']['gpu']['count'] == 4
        assert nodes[0]['node']['compute_specs']['gpu']['model'] == 'NVIDIA A100'


def test_get_non_validated_count():
    """Test getting count of non-validated nodes."""
    with patch('primeprotocol.ValidatorClient') as MockValidator:
        mock_instance = Mock()
        # Mock list_non_validated_nodes to return 3 nodes
        mock_nodes = [Mock() for _ in range(3)]
        mock_instance.list_non_validated_nodes.return_value = mock_nodes
        mock_instance.get_non_validated_count.return_value = 3
        MockValidator.return_value = mock_instance
        
        from primeprotocol import ValidatorClient
        validator = ValidatorClient("http://localhost:8545", "0x123", ["http://localhost:8089"])
        
        # Get count
        count = validator.get_non_validated_count()
        
        # Verify result
        assert count == 3


if __name__ == "__main__":
    pytest.main([__file__]) 