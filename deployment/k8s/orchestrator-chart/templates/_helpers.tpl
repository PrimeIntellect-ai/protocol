{{/* Helm helper templates */}}

{{- define "orchestrator.fullname" -}}
orchestrator-{{ .Values.computePoolId }}
{{- end -}}

{{- define "orchestrator.redisName" -}}
  orchestrator-{{ .Values.computePoolId }}-redis
{{- end -}}

{{- define "orchestrator.secretName" -}}
orchestrator-{{ .Values.computePoolId }}-secret
{{- end -}}

{{- define "orchestrator.backendConfigName" -}}
orchestrator-{{ .Values.computePoolId }}-backend-config
{{- end -}}

{{- define "orchestrator.redisPVCName" -}}
orchestrator-{{ .Values.computePoolId }}-redis-data
{{- end -}}
