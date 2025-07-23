# Prime Protocol Python SDK

## Local Development Setup
Startup the local dev chain with contracts using the following Make cmd from the base folder:
```bash
make bootstrap
```

Within this folder run the following to install the setup:
```bash
make install
```

Whenever you change something in your code, simply run `make build`.

## Example flow: 
The following example flow implements the typical prime protocol toplogy with an orchestrator, validator and worker node.
It still uses a centralized discovery service which will be replaced shortly.

```bash
export PRIVATE_KEY_NODE=XYZ
export PRIVATE_KEY_PROVIDER=XYZ
```
(for the private keys you can just use the keys from the `.env.example`)

```bash
uv run examples/worker.py 
```

You'll likely need to whitelist the provider from the base folder:
```bash
make whitelist-provider
```

You should be able to see the node now on the discovery service:
```bash
curl http://localhost:8089/api/platform -H "Authorization: Bearer prime" | jq
```

### Validator:
Next run the validator to ensure nodes are validated on chain. This is very basic logic. The pool owner could come up with something more sophisticated in the future.
```bash
uv run examples/validator.py
```

On underlying python side all the logic here is within:
```python
from primeprotocol import ValidatorClient
```

You should actually see that the node is validated now on the discovery service.

### Orchestrator / pool owner
Once we actually have validated nodes, we still want to invite them to the compute pool and have them contribute work. 

Use the pool owner private key from the .env.example and export:
```bash
export PRIVATE_KEY_ORCHESTRATOR=xyz
```
To run the orchestrator simply execute:
```bash
uv run examples/orchestrator.py 
```
You should see that the orchestrator invites the worker node that you started in the earlier step and keeps sending messages via p2p to this node. 

## TODOs:
- [ ] restarting node - no longer getting messages (since p2p id changed)?
- [ ] can validator send message? (special validation message)
- [ ] can orchestrator send message?
- [ ] whitelist using python api?
- [ ] borrow bug?
- [ ] restart keeps increasing provider stake?
- [ ] p2p cleanup

## Known Limitations:
1. Orchestrator can no longer send messages to a node when the worker node restarts.
This is because the discovery info is not refreshed when the node was already active in the pool. This will change when introducing a proper DHT layer.
Simply run `make down` and restart the setup from scatch.