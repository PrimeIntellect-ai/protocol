# Protocol 
Prime Protocol is a peer-to-peer compute and intelligence network that enables decentralized AI development at scale. This repository contains the core infrastructure for contributing compute resources to the network, including miners, validators, and the coordination layer.

## Setup:
### Clone the repository with submodules  
```
git clone --recurse-submodules https://github.com/prime-ai/prime-miner-validator.git
```
- Update submodules:
```
git submodule update --init --recursive
```
### Installation
- Foundry: `curl -L https://foundry.paradigm.xyz | bash` - do not forget `foundryup`
- Docker 
- tmuxinator: Install via `gem install tmuxinator` - do not use brew, apparently their brew build is broken
- Rust: Install via `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
- Install cargo watch: `cargo install cargo-watch`
- Install redis-server: `brew install redis`(mac) or `sudo apt-get install redis-server`(ubuntu)
- Adjust docker desktop setting: `Allow the default Docker socket to be used (requires password)` must be enabled
- .env in base folder and .env in discovery folder (will be replaced shortly)

### 2. Install Dependencies
```bash
# Install Foundry
curl -L https://foundry.paradigm.xyz | bash
foundryup

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install cargo-watch
cargo install cargo-watch

# Install Redis (MacOS)
brew install redis

# Install Redis (Ubuntu)
# sudo apt-get install redis-server

# Install tmuxinator (do not use brew)
gem install tmuxinator
```

### 3. Configure Environment
- Enable "Allow the default Docker socket to be used" in Docker Desktop settings
- Create `.env` files in base folder and discovery folder

## Development

### Starting the Development Environment

To start all core services:
```bash
make up
```

This will launch:
- Local blockchain node
- Discovery service
- Validator node
- Orchestrator service
- Redis instance
- Supporting infrastructure

### Running a Worker Node

Once the core services are running, you can start a worker node in a new terminal:
```bash
make watch-worker
```

The worker will automatically connect to the discovery service and begin processing tasks.
It takes a couple of seconds until the worker is whitelisted. This is done using a simple loop on the second page of tmux.

You can find more details on the APIs in the orchestrator and discovery service directory.

### Deploying a task

First, you need to create a local worker (after you have all other services running using e.g. `make up`) 

```bash
make watch-worker
```

check that the worker as been registered on the orchestrator: 

```bash
curl -X GET http://localhost:8090/nodes -H "Authorization: Bearer admin"
>>> {"nodes":[{"address":"0x66295e2b4a78d1cb57db16ac0260024900a5ba9b","ip_address":"0.0.0.0","port":8091,"status":"Healthy","task_id":null,"task_state":null}],"success":true}
```


then lets create a task

```bash
curl -X POST http://localhost:8090/tasks -H "Content-Type: application/json" -H "Authorization: Bearer admin" -d '{"name":"sample","image":"ubuntu:latest"}'
>>> {"success":true,"task":"updated_task"}% 
```

and check that the task is created

```bash
curl -X GET http://localhost:8090/nodes -H "Authorization: Bearer admin"
>>> {"nodes":[{"address":"0x66295e2b4a78d1cb57db16ac0260024900a5ba9b","ip_address":"0.0.0.0","port":8091,"status":"Healthy","task_id":"29edd356-5c48-4ba6-ab96-73d002daddff","task_state":"RUNNING"}],"success":true}%     
```

you can also check docker ps to see that the docker is running locally

```bash
docker ps
CONTAINER ID   IMAGE                               COMMAND                  CREATED          STATUS          PORTS                                         NAMES
e860c44a9989   ubuntu:latest                       "sleep infinity"         3 minutes ago    Up 3 minutes                                                  prime-task-29edd356-5c48-4ba6-ab96-73d002daddff
ef02d23b5c74   redis:alpine                        "docker-entrypoint.s…"   27 minutes ago   Up 27 minutes   0.0.0.0:6380->6379/tcp, [::]:6380->6379/tcp   prime-worker-validator-redis-1
7761ee7b6dcf   ghcr.io/foundry-rs/foundry:latest   "anvil --host 0.0.0.…"   27 minutes ago   Up 27 minutes   0.0.0.0:8545->8545/tcp, :::8545->8545/tcp     prime-worker-validator-anvil-1
```


### Stopping Services

To gracefully shutdown all services:
```bash
make down
```
# Prime Protocol

The current setup supports INTELLECT-2 with a limited number of validators and a central coordinator that manages workload distribution across miners.

## Quick Start

### Prerequisites
- [Foundry](https://book.getfoundry.sh/getting-started/installation)
- [Docker](https://docs.docker.com/get-docker/)
- [Rust](https://www.rust-lang.org/tools/install)
- [Redis](https://redis.io/docs/getting-started/)
- tmuxinator (via `gem install tmuxinator`)

### Installation

1. Clone the repository with submodules:
```bash
git clone --recurse-submodules https://github.com/prime-ai/prime-miner-validator.git
cd prime-miner-validator
```

2. Update submodules if needed:
```bash
git submodule update --init --recursive
```

3. Install dependencies:
```bash
# Install Foundry
curl -L https://foundry.paradigm.xyz | bash
foundryup

# Install cargo-watch
cargo install cargo-watch

# Set up environment files
cp .env.example .env
cp discovery/.env.example discovery/.env
```

### Docker Configuration
- Enable "Allow the default Docker socket to be used" in Docker Desktop settings
- This setting requires password authentication

## Development Setup

### First-time Setup
```bash
# Start core services
docker compose up

# Build whitelist provider
make whitelist-provider
```

### Local Development
```bash
# Start tmux environment
make up

# Start miner in tmux
make watch-miner

# Stop environment
make down
```

### Remote Deployment
```bash
# Set required environment variables
export EXTERNAL_IP="<machine-ip>"
export SSH_CONNECTION="ssh ubuntu@<ip> -i <private-key.pem>"

# Deploy miner
make remote-miner
```

## Development Testing

### Starting a Development Runner

1. Start core services and miner:
```bash
make up
make watch-miner
```

2. Verify miner registration:
```bash
curl -X GET http://localhost:8090/nodes
```
Expected response:
```json
{
  "nodes": [{
    "address": "0x66295e2b4a78d1cb57db16ac0260024900a5ba9b",
    "ip_address": "0.0.0.0",
    "port": 8091,
    "status": "Healthy",
    "task_id": null,
    "task_state": null
  }],
  "success": true
}
```

3. Create a test task:
```bash
curl -X POST http://localhost:8090/tasks \
  -H "Content-Type: application/json" \
  -d '{"name":"sample","image":"ubuntu:latest"}'
```

4. Verify task creation:
```bash
curl -X GET http://localhost:8090/nodes
docker ps  # Check running containers
```

## System Architecture

The system implements a distributed compute network with the following key components:

- **Buyer**: Initiates compute tasks via CLI/API
- **Compute Coordinator**: Master node managing task distribution
- **Compute Provider**: Nodes providing computational resources
- **Validator**: Verifies node legitimacy and performance
- **Discovery Service**: Handles node registration and discovery

See the sequence diagram below for detailed interaction flow.

[System Architecture Diagram]

Note: The current architecture is optimized for the INTELLECT-2 run and will be expanded with additional components (e.g., termination handling) in future releases.