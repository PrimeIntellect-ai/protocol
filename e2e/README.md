# Automated E2E Setup for Remote Machines

This guide describes the process for setting up end-to-end testing environments on remote machines using Ansible playbooks.

> ⚠️ **Warning:** This setup is intended for testing environments only, as it involves copying test keys to remote machines. Do not use in production.

## Prerequisites

Before beginning the setup process, ensure you have:
- Ansible installed on your local machine
- SSH access to target remote machines
- Required test keys and configurations

## Setup Process

### 1. Configure Inventory

1. Obtain your inventory configuration
2. Update `inventory.yml` with your machine details

### 2. Chain Setup

Run the chain setup playbook:

```bash
ansible-playbook -i inventory.yml tasks/chain_setup.yml
```

### 3. Local Configuration

Configure the local environment:

1. Update `.env.remote` with the RPC URL from your chain server specified in the Ansible inventory
2. Run the setup command:
   ```bash
   ENV_FILE=".env.remote" make setup
   ```

### 4. Service Deployment

Deploy the required services in the following order:

1. Discovery Service:
   ```bash
   ansible-playbook -i inventory.yml tasks/discovery_setup.yml
   ```

2. Orchestrator:
   ```bash
   ansible-playbook -i inventory.yml tasks/orchestrator_setup.yml
   ```

3. Validator:
   ```bash
   ansible-playbook -i inventory.yml tasks/validator_setup.yml
   ```

## Notes

- The RPC URL in `.env.remote` typically needs to be updated to match the chain server from your Ansible inventory
- Automated local setup is planned for future implementation (currently manual)