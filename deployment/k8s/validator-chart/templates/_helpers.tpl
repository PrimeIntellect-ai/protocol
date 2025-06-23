{{/* Helm helper templates */}}

{{- define "validator.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else if .Values.computePoolId -}}
{{- printf "validator-%s" .Values.computePoolId -}}
{{- else -}}
{{- printf "validator" -}}
{{- end -}}
{{- end -}}

{{- define "validator.namespace" -}}
{{- .Values.namespace | default .Release.Namespace -}}
{{- end -}}
