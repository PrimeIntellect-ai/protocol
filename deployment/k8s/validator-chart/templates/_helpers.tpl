{{/* Helm helper templates */}}

{{- define "validator.fullname" -}}
validator-{{ .Values.computePoolId }}
{{- end -}}

{{- define "validator.redisName" -}}
  validator-{{ .Values.computePoolId }}-redis
{{- end -}}

{{- define "validator.secretName" -}}
validator-{{ .Values.computePoolId }}-secret
{{- end -}}

{{- define "validator.backendConfigName" -}}
validator-{{ .Values.computePoolId }}-backend-config
{{- end -}}

{{- define "validator.redisPVCName" -}}
validator-{{ .Values.computePoolId }}-redis-data
{{- end -}}
