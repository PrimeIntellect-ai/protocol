# Prime Protocol Usage Guide

This guide provides instructions for deploying and managing tasks on the Prime Protocol network.

## Table of Contents
- [Deploying Tasks](#deploying-tasks)
- [Monitoring Tasks](#monitoring-tasks)
- [Task Management](#task-management)

## Deploying Tasks

### Prerequisites

Before deploying tasks, ensure you have:
1. All core services running (`make up`)
2. A worker node registered on the network

### Start a Worker Node

Start a local worker:

```bash
make watch-worker
```

### Verify Worker Registration

Check that the worker has been registered with the orchestrator:

```bash
curl -X GET http://localhost:8090/nodes -H "Authorization: Bearer admin"
```

Successful response:
```json
{
  "nodes": [
    {
      "address": "0x66295e2b4a78d1cb57db16ac0260024900a5ba9b",
      "ip_address": "0.0.0.0",
      "port": 8091,
      "status": "Healthy",
      "task_id": null,
      "task_state": null
    }
  ],
  "success": true
}
```

### Create a Task

Deploy a simple task using the API:

```bash
curl -X POST http://localhost:8090/tasks \
     -H "Content-Type: application/json" \
     -H "Authorization: Bearer admin" \
     -d '{"name":"sample","image":"ubuntu:latest"}'
```

Successful response:
```json
{
  "success": true,
  "task": "updated_task"
}
```

## Monitoring Tasks

### Check Task Status

Verify that the task is running:

```bash
curl -X GET http://localhost:8090/nodes -H "Authorization: Bearer admin"
```

Successful response:
```json
{
  "nodes": [
    {
      "address": "0x66295e2b4a78d1cb57db16ac0260024900a5ba9b",
      "ip_address": "0.0.0.0",
      "port": 8091,
      "status": "Healthy",
      "task_id": "29edd356-5c48-4ba6-ab96-73d002daddff",
      "task_state": "RUNNING"
    }
  ],
  "success": true
}
```

### Verify Container Status

Check that the Docker container is running:

```bash
docker ps
```

Example output:
```
CONTAINER ID   IMAGE                               COMMAND                  CREATED          STATUS          PORTS                                         NAMES
e860c44a9989   ubuntu:latest                       "sleep infinity"         3 minutes ago    Up 3 minutes                                                  prime-task-29edd356-5c48-4ba6-ab96-73d002daddff
ef02d23b5c74   redis:alpine                        "docker-entrypoint.s…"   27 minutes ago   Up 27 minutes   0.0.0.0:6380->6379/tcp, [::]:6380->6379/tcp   prime-worker-validator-redis-1
7761ee7b6dcf   ghcr.io/foundry-rs/foundry:latest   "anvil --host 0.0.0.…"   27 minutes ago   Up 27 minutes   0.0.0.0:8545->8545/tcp, :::8545->8545/tcp     prime-worker-validator-anvil-1
```