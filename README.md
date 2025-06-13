# Protocol

<div align="center">
<img src="docs/assets/logo.svg" width="200" height="200" alt="Prime Protocol Logo"/>
  <h3>Decentralized Compute Infrastructure for AI</h3>
</div>

> ⚠️ **IMPORTANT**: This project is still under active development. Currently, you can only run the protocol locally - connecting to public RPCs is not yet supported. Please check back later for updates. See our [FAQ](#frequently-asked-questions) for details.

Prime Network is a peer-to-peer compute and intelligence network that enables decentralized AI development at scale. This repository contains the core infrastructure for contributing compute resources to the network, including workers, validators, and the coordination layer.

## 📚 Table of Contents
- [System Architecture](#system-architecture)
- [Getting Started](#getting-started)
- [Documentation](#documentation)
- [Frequently Asked Questions](#frequently-asked-questions)
- [Community](#community)
- [Contributing](#contributing)
- [Security](#security)
- [License](#license)

## System Architecture
The Prime Protocol follows a modular architecture designed for decentralized AI compute:

<div align="center">
  <img src="docs/assets/overview.png" alt="Prime Protocol System Architecture" width="800"/>
</div>

### Component Overview
- **Smart Contracts**: Ethereum-based contracts manage the protocol's economic layer
- **Discovery Service**: Enables secure peer discovery and metadata sharing 
- **Orchestrator**: Coordinates compute jobs across worker nodes
- **Validator Network**: Ensures quality through random challenges
- **Worker Nodes**: Execute AI workloads in secure containers

## Getting Started

### Prerequisites
- Linux operating system
- CUDA-capable GPU(s) for worker operations
- Docker (version 28.1.1 or later) and Docker Compose (version v2.35.1 or later)

For complete setup instructions, refer to our [Development Setup Guide](docs/development-setup.md).

### Install Worker CLI: 
You can install the latest worker CLI using:
```
curl -sSL https://raw.githubusercontent.com/PrimeIntellect-ai/protocol/main/crates/worker/scripts/install.sh | bash 
```
This can also be used to upgrade the current installation to the latest release.

For the latest dev build use: 
```
curl -sSL https://raw.githubusercontent.com/PrimeIntellect-ai/protocol/develop/crates/worker/scripts/install.sh | bash -s -- --dev
```

## Documentation
- [Development Setup Guide](docs/development-setup.md) - Detailed installation and environment setup instructions
- [Usage Guide](docs/usage-guide.md) - Instructions for dispatching tasks, monitoring, and system management

## Frequently Asked Questions

#### Q: What is Prime Protocol?
**A:** Prime Protocol is a peer-to-peer compute and intelligence network that enables decentralized AI development at scale. It provides infrastructure for contributing compute resources to the network through workers, validators, and a coordination layer.

#### Q: Is Prime Protocol ready for production use?
**A:** No, Prime Protocol is still under active development. Currently, you can only run the protocol locally. 

#### Q: What environment variables do I need for local development?
**A:** We have provided an .env.example file with the required variables. 

#### Q: How are private keys managed securely in the system?
**A:** We're actively developing our security practices for private key management. 

#### Q: What are the recommended network isolation strategies for the worker?
**A:** We will be providing detailed documentation on how to secure workers with firewalls for ingress / egress 

#### Q: What are the funding requirements for workers?
**A:** For the current development phase, minimal testnet ETH is sufficient. Detailed information on setting up the worker will follow.

## Community
- [Discord](https://discord.gg/primeintellect)
- [X](https://x.com/PrimeIntellect)
- [Blog](https://www.primeintellect.ai/blog)

## Contributing
We welcome contributions! Please see our [Contributing Guidelines](CONTRIBUTING.md).

## Security
See [SECURITY.md](SECURITY.md) for security policies and reporting vulnerabilities.