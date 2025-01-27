# Update inventory

- create a release

# setup chain
ansible-playbook -i inventory.yml tasks/chain_setup.yml

// # manual soon automated
// adjust .env.remote with rpc url of chain 
// run `ENV_FILE=".env.remote" make setup` 

# setup discovery 
ansible-playbook -i inventory.yml tasks/discovery_setup.yml

