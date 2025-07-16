# Prime Protocol Development Setup

## Table of Contents
- [Prerequisites](#prerequisites)
- [Full Development Setup](#full-development-setup)
- [Docker Compose Setup](#docker-compose-setup)
- [Running a Worker Node](#running-a-worker-node)
- [Remote GPU Development](#remote-gpu-development)
- [Stopping Services](#stopping-services)

## Prerequisites

Before running Prime Protocol, ensure you have the following requirements:

### Software
- [Docker Desktop](https://www.docker.com/products/docker-desktop/) - Container runtime
- [Git](https://git-scm.com/) - Version control
- [Rust](https://www.rust-lang.org/) - Programming language and toolchain
- [Redis](https://redis.io/) - In-memory data store
- [Foundry](https://book.getfoundry.sh/) - Smart contract development toolkit
- [tmuxinator](https://github.com/tmuxinator/tmuxinator) - Terminal session manager

## Full Development Setup

### 1. Clone Repository
```bash
git clone https://github.com/PrimeIntellect-ai/protocol.git
cd protocol
git submodule update --init --recursive
```

### 2. Install Dependencies

#### Foundry
```bash
# Install Foundry
curl -L https://foundry.paradigm.xyz | bash

# Reload your shell environment 
source ~/.bashrc

foundryup
```

#### Rust
```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Reload your shell environment
source ~/.bashrc

# Install cargo-watch
cargo install cargo-watch
```

#### Redis
```bash
# Install Redis (MacOS)
brew install redis

# Install Redis (Ubuntu)
sudo apt-get update
sudo apt-get install redis-server
```

#### Tmux
```bash
# Install Tmux (MacOS)
brew install tmux

# Install Tmux (Ubuntu)
sudo apt install tmux
```

### Ubuntu:
```bash
sudo apt-get install libssl-dev
```

### 3. Configure Environment
- Enable "Allow the default Docker socket to be used" in Docker Desktop settings (MacOS)
- On Ubuntu, add your user to the docker group:
  ```bash
  sudo usermod -aG docker $USER
  ```

### 4. Launch Core Services
- Copy the `.env.example` file to `.env` 
```bash
make up
```

This will start:
- Local blockchain node
- Bootnode
- Validator node
- Orchestrator service
- Redis instance
- Supporting infrastructure

## Running a Worker Node

Once the core services are running, you can start a worker node in a new terminal:
```bash
make watch-worker
```
Your worker will show an error indicating it is not whitelisted yet. You'll need to run `make whitelist-provider` to resolve this.

You should see your worker appear on the orchestrator. You can also check the orchestrator documentation at: `http://localhost:8090/docs`

Quick check in the api of the orchestrator:
```bash
curl -H "Authorization: Bearer admin" http://localhost:8090/nodes 
```

## Remote GPU Development
> ⚠️ **IMPORTANT**: The video shows the whitelist process happening automatically. Currently, this must be done manually using the command `make whitelist-provider`.

https://github.com/user-attachments/assets/8b25ad50-7183-4dd5-add6-f9acf3852b03

Start the local development environment:
```bash
make up
```

Set up your remote GPU worker:
1. Provision a GPU instance and ensure Docker is installed
2. Configure environment variables and start the remote worker:
   ```bash
   SSH_CONNECTION="ssh your-ssh-conn string"
   EXTERNAL_IP="your-external-ip"
   make remote-worker
   ```

## Stopping Services

To gracefully shutdown all services:
```bash
make down
```

## Docker Compose Setup
You can run all supporting services (chain, validator, bootnode, orchestrator) (if you only want to work on the worker sw) in docker compose.

1. Start docker compose:
   ```bash
   docker compose up
   ```

2. Run Setup:  
   ```bash
   make setup
   ```

3. Launch a worker:
   - Adjust the .env var `WORKER_EXTERNAL_IP` to: `WORKER_EXTERNAL_IP=host.docker.internal` 
   - Launch the worker using `make watch-worker`
   - Whitelist the worker once you see the whitelist alert using: `make whitelist-provider`
