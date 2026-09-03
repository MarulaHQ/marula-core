{{/* Chart helpers — naming and image resolution (digest-pinned per U10b). */}}

{{- define "meridian.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "meridian.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s" (default .Chart.Name .Values.nameOverride) | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "meridian.labels" -}}
app.kubernetes.io/name: {{ include "meridian.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{- /* One image reference per binary. Enforces digest pinning when
digestPinningEnforced is on — renders an invalid field (fails schema
validation / admission) rather than silently floating. */ -}}
{{- define "meridian.image" -}}
{{- $img := .image -}}
{{- if and .ctx.Values.digestPinningEnforced (not $img.digest) -}}
{{- fail (printf "image %s has no digest set (digestPinningEnforced=true): set images.%s.digest from the release's published attestation" $img.repository .key) -}}
{{- end -}}
{{- $ref := printf "%s/%s:%s" .ctx.Values.images.registry $img.repository $img.tag -}}
{{- if $img.digest -}}
{{- printf "%s@%s" $ref $img.digest -}}
{{- else -}}
{{- $ref -}}
{{- end -}}
{{- end -}}

{{- define "meridian.dbUrl" -}}
{{- if .Values.postgresql.deploy -}}
postgres://{{ .Values.postgresql.ownerUser }}:{{ .Values.postgresql.ownerPassword | default "changeme-owner" }}@{{ include "meridian.fullname" . }}-postgres:5432/{{ .Values.postgresql.database }}
{{- else -}}
{{- required "postgresql.externalUrl required when postgresql.deploy=false" (.Values.postgresql.externalUrl) -}}
{{- end -}}
{{- end -}}

{{- define "meridian.dbAppUrl" -}}
{{- if .Values.postgresql.deploy -}}
postgres://{{ .Values.postgresql.appUser }}:{{ .Values.postgresql.appPassword | default "changeme-app" }}@{{ include "meridian.fullname" . }}-postgres:5432/{{ .Values.postgresql.database }}
{{- else -}}
{{- required "postgresql.externalAppUrl required when postgresql.deploy=false" (.Values.postgresql.externalAppUrl) -}}
{{- end -}}
{{- end -}}

{{- define "meridian.redisUrl" -}}
{{- if .Values.redis.deploy -}}
redis://{{ include "meridian.fullname" . }}-redis:6379/0
{{- else -}}
{{- required "redis.externalUrl required when redis.deploy=false" .Values.redis.externalUrl -}}
{{- end -}}
{{- end -}}
