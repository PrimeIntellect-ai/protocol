{{/* Helm helper templates */}}

{{- define "orchestrator.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else if .Values.computePoolId -}}
{{- printf "orchestrator-%s" .Values.computePoolId -}}
{{- else -}}
{{- printf "orchestrator" -}}
{{- end -}}
{{- end -}}

{{- define "orchestrator.namespace" -}}
{{- .Values.namespace | default .Release.Namespace -}}
{{- end -}}

{{- define "orchestrator.container" -}}
{{- $mode := .mode }}
{{- $root := .root }}
- name: orchestrator
  image: {{ $root.Values.image }}
  ports:
  - containerPort: {{ $root.Values.port }}
  readinessProbe:
    httpGet:
      path: /health
      port: {{ $root.Values.port }}
    initialDelaySeconds: 5
    periodSeconds: 10
  livenessProbe:
    httpGet:
      path: /health
      port: {{ $root.Values.port }}
    initialDelaySeconds: 15
    periodSeconds: 20
  resources:
    requests:
      memory: "256Mi"
      cpu: "500m"
    limits:
      memory: "512Mi"
      cpu: "2000m"
  env:
  - name: MODE
    value: {{ $mode }}
  - name: PORT
    value: {{ $root.Values.port | quote }}
  - name: URL
    value: {{ $root.Values.url }}
  - name: DOMAIN_ID
    value: {{ $root.Values.domainId }}
  {{- if .Values.computePoolId }}
  - name: COMPUTE_POOL_ID
    value: {{ $root.Values.computePoolId }}
  {{- end }}
  {{- range $key, $value := $root.Values.env }}
  - name: {{ $key }}
    value: {{ $value | quote }}
  {{- end }}
  {{- range $root.Values.envFromSecret }}
  - name: {{ .name }}
    valueFrom:
      secretKeyRef:
        name: {{ .secretKeyRef.name }}
        key: {{ .secretKeyRef.key }}
  {{- end -}}
{{- end -}}
