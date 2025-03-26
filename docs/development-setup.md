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

# Reload .bashrc (or .bash_profile, depends on the system)
source ~/.bashrc

foundryup
```

#### Rust
```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install cargo-watch
cargo install cargo-watch
```

#### Redis
```bash
# Install Redis (MacOS)
brew install redis

# Install Redis (Ubuntu)
sudo apt-get install redis-server
```

#### Ruby and Tmux
```bash
# Install Ruby (MacOS)
brew install ruby

# Install Ruby (Ubuntu)
sudo apt-get install ruby

# Install tmuxinator (do not use brew)
gem install tmuxinator

# Install Tmux (MacOS)
brew install tmux

# Install Tmux (Ubuntu)
sudo apt install tmux
sudo apt-get install libssl-dev
```

### 3. Configure Environment
- Enable "Allow the default Docker socket to be used" in Docker Desktop settings (MacOS)
- On Ubuntu, add your user to the docker group:
  ```bash
  sudo usermod -aG docker $USER
  ```
- Create `.env` files in base folder and discovery folder

### 4. Launch Core Services
```bash
make up
```

This will start:
- Local blockchain node
- Discovery service
- Validator node
- Orchestrator service
- Redis instance
- Supporting infrastructure

## Docker Compose Setup

You can run all supporting services (chain, validator, discovery, orchestrator) in docker compose.

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

## Running a Worker Node

Once the core services are running, you can start a worker node in a new terminal:
```bash
make watch-worker
```

The worker will automatically connect to the discovery service and begin processing tasks.
It takes a couple of seconds until the worker is whitelisted. This is done using a simple loop on the second page of tmux.

You can find more details on the APIs in the orchestrator and discovery service directory.

## Remote GPU Development

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