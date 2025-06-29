{{/* Helm helper templates */}}

{{- define "discovery.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "discovery" -}}
{{- end -}}
{{- end -}}

{{- define "discovery.namespace" -}}
{{- .Values.namespace | default .Release.Namespace -}}
{{- end -}}

{{- define "discovery.container" -}}
{{- $mode := .mode }}
{{- $root := .root }}
- name: discovery
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
