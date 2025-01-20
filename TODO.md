
# General for MVP
- [ ] Starting compute pool seems a bit flaky - currently called from orchestrator script
- [ ] Discovery service still using old env?
- [ ] Latest state storing could make sense to simplify heartbeat dev
- [ ] Heartbeat loop is lost and still running
- [ ] heartbeat loop and heartbeat api 
- [ ] Heartbeat based status updater
- [ ] Hardcoded addresses in conracts
- [x] Random dangling processes?
- [ ] GPU detection / memory - what if we do not detect a gpu?
- [ ] Share abi files in one folder
- [ ] Bug overwriting nodes in redis
- [ ] Signature verification currently allows access if you simply have a valid signature
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

# To be discussed
- [ ] Termination cases

# Optional nice to have for launch 
- [ ] Master election 

# Important training / synthetic data generation considerations
- [ ] How to handle secrets?
- [ ] How to handle logs / metrics?