"""Basic tests for the Prime Protocol Python client."""

import pytest
from primeprotocol import PrimeProtocolClient


def test_client_creation():
    """Test that client can be created with valid RPC URL."""
    client = PrimeProtocolClient("http://localhost:8545")
    assert client is not None


def test_client_creation_with_empty_url():
    """Test that client creation fails with empty RPC URL."""
    with pytest.raises(ValueError):
        PrimeProtocolClient("")


def test_client_creation_with_invalid_url():
    """Test that client creation fails with invalid RPC URL."""
    with pytest.raises(ValueError):
        PrimeProtocolClient("not-a-valid-url")


def test_has_compute_pool_exists_method():
    """Test that the client has the compute_pool_exists method."""
    client = PrimeProtocolClient("http://example.com:8545")
    assert hasattr(client, 'compute_pool_exists')
    assert callable(getattr(client, 'compute_pool_exists')) 