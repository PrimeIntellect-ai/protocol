
# Build  
`
cargo build --release
`

# Setup chain on machine
`
ansible-playbook -i inventory.yaml playbooks/chain_setup.yaml
`

# Setup discovery service on machine

`
ansible-playbook -i inventory.yaml playbooks/discovery_setup.yaml
`

# Setup orchestrator
`
ansible-playbook -i inventory.yaml playbooks/orchestrator_setup.yaml
`

# Boot miners

`
ansible-playbook -i inventory.yaml playbooks/miner_setup.yaml
`