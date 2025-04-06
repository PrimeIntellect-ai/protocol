# K8s Helm charts

## Deploy orchestrator
0. Create a values.yaml file based on the values.example.yaml
1. deploy orchestrator for compute pool <ID e.g. 1 here>:
```
helm install orchestrator-1 ./orchestrator-chart/ --values orchestrator-chart/values.yaml
```
2 make sure to adjust any ingress / domain mappings


### Uninstall orchestrator:
```
helm uninstall orchestrator-1
```