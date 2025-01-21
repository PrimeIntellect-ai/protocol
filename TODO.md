
# General for MVP

# Features for MVP:
- [x] API to list nodes and status
- [ ] Heartbeat based status updater
- [ ] Ability to dispatch tasks to nodes
- [ ] Heartbeat Verification 

- [ ] What if a node never becomes healthy? 

# Cleanup TODOs:
- [ ] contract call cleanup
- [ ] Discovery service still using old env?
- [ ] Latest state storing could make sense to simplify heartbeat dev 
- [ ] Hardcoded addresses in conracts
- [ ] GPU detection / memory - what if we do not detect a gpu?
- [ ] Share abi files in one folder
- [ ] Secure API for platform (manveer access)
- [ ] Cleanup contracts builder
- [ ] Signature creation and cleanup
- [ ] Cleanup orchestrator script
- [ ] List and deploy api on compute coordinator
- [ ] Proper hardware detection
- [ ] Miner termination (and restart)
- [ ] GPU mounting to docker
- [ ] Validator functionality testing
- [ ] Edge case testing when services are down (especially discovery service)
- [ ] Proper logging in discovery service
- [ ] pubsub listening in discovery
- [ ] better startup of discovery service
- [ ] node loosing heartbeat state after restart 
- [ ] readd dry-mode
- [ ] Secure nodes / orchestrator api
- [ ] check docker versiont pu

- [x] Random dangling processes?
- [x] Heartbeat loop is lost and still running
- [x] Signature verification currently allows access if you simply have a valid signature

# To be discussed
- [ ] Termination cases

# Optional nice to have for launch 
- [ ] Master election 
- [ ] Startup script can fails starting compute pool 
# Important training / synthetic data generation considerations
- [ ] How to handle secrets?
- [ ] How to handle logs / metrics?