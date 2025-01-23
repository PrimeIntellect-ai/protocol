
# General for MVP

# Features for MVP:
- [ ] known issue - docker will kill tasks -> We should start docker runner once we have joined a pool 
- [ ] Heartbeat Verification on orchestrator (Signature with dynamic middleware list)
- [ ] Secure API for platform (manveer access)
- [ ] Proper logging in discovery service
- [ ] node loosing heartbeat state after restart 
- [ ] Startup script can fails starting compute pool 

- register provider cli logs can be improved
- node becomes unhealthy very quickly - heartbeat recovery counter should be increased
- task status currently unused
- getting redis connection looks like shit
- starting compute pool in orchestrator main file 
- orchestrator main file - tasks arc?
- node inviter code could be nicer using local state / settings from cmd 
- node inviter should use tokio for generating invites 
- types.rs file in orchestrator?
- let claude check auth middleware?
- contract comment using custom contract error 
- recheck wallet file
- check todos in compute registry contract
- compute pool contract parsing ugly af

deployment
- discovery service base url params?
