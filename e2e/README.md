# Automated e2e setup on remote machines 
> ⚠️ **Warning:** This setup is for testing only as it copies test keys to machines! 

# setup chain
ansible-playbook -i inventory.yml tasks/chain_setup.yml

# run setup locally
(manual soon automated)
adjust .env.remote with rpc url of chain 
run `ENV_FILE=".env.remote" make setup` 

# setup discovery 
ansible-playbook -i inventory.yml tasks/discovery_setup.yml

# setup validator 

# setup orchestrator with compute pool id

