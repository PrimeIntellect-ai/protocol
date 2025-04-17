{{/* Helm helper templates */}}

{{- define "validator.fullname" -}}
{{- if .Values.computePoolId -}}
validator-{{ .Values.computePoolId }}
{{- else -}}
validator-hw
{{- end -}}
{{- end -}}

{{- define "validator.redisName" -}}
{{- if .Values.computePoolId -}}
  validator-{{ .Values.computePoolId }}-redis
{{- else -}}
  validator-hw-redis
{{- end -}}
{{- end -}}

{{- define "validator.secretName" -}}
{{- if .Values.computePoolId -}}
validator-{{ .Values.computePoolId }}-secret
{{- else -}}
validator-hw-secret
{{- end -}}
{{- end -}}

{{- define "validator.backendConfigName" -}}
{{- if .Values.computePoolId -}}
validator-{{ .Values.computePoolId }}-backend-config
{{- else -}}
validator-hw-backend-config
{{- end -}}
{{- end -}}

{{- define "validator.redisPVCName" -}}
{{- if .Values.computePoolId -}}
validator-{{ .Values.computePoolId }}-redis-data
{{- else -}}
validator-hw-redis-data
{{- end -}}
{{- end -}}
