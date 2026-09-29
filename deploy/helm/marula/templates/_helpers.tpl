{{/* Chart helpers — naming and image resolution (digest-pinned per U10b). */}}

{{- define "marula.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "marula.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s" (default .Chart.Name .Values.nameOverride) | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "marula.labels" -}}
app.kubernetes.io/name: {{ include "marula.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{- /* Bare-string third-party image slot. When nonMarulaDigestPinningEnforced
is on, the value must carry an @sha256:... digest itself or have a
companion imageDigest value — the final reference is tag@digest. */ -}}
{{- define "marula.pinnedImage" -}}
{{- $image := toString .image -}}
{{- $digest := .digest -}}
{{- if and .ctx.Values.nonMarulaDigestPinningEnforced (not (contains "@sha256:" $image)) (not $digest) -}}
{{- fail (printf "image %s has no digest (nonMarulaDigestPinningEnforced=true): set the companion imageDigest value" $image) -}}
{{- end -}}
{{- if $digest -}}
{{- $base := $image | splitList "@" | first -}}
{{- printf "%s@%s" $base $digest -}}
{{- else -}}
{{- $image -}}
{{- end -}}
{{- end -}}

{{- /* Container-level least privilege (p38-container-hardening): no
privilege escalation, all capabilities dropped, seccomp RuntimeDefault.
readOnlyRootFilesystem is opt-in per workload via `readOnly` since only
some components tolerate it. */ -}}
{{- define "marula.containerSecurityContext" -}}
{{- $readOnly := default false .readOnly -}}
allowPrivilegeEscalation: false
capabilities:
  drop: ["ALL"]
seccompProfile:
  type: RuntimeDefault
readOnlyRootFilesystem: {{ $readOnly }}
{{- end -}}

{{- /* One image reference per binary. Enforces digest pinning when
digestPinningEnforced is on — renders an invalid field (fails schema
validation / admission) rather than silently floating. */ -}}
{{- define "marula.image" -}}
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

{{- define "marula.dbUrl" -}}
{{- if .Values.postgresql.deploy -}}
postgres://{{ .Values.postgresql.ownerUser }}:{{ .Values.postgresql.ownerPassword | default "changeme-owner" }}@{{ include "marula.fullname" . }}-postgres:5432/{{ .Values.postgresql.database }}
{{- else -}}
{{- required "postgresql.externalUrl required when postgresql.deploy=false" (.Values.postgresql.externalUrl) -}}
{{- end -}}
{{- end -}}

{{- define "marula.dbAppUrl" -}}
{{- if .Values.postgresql.deploy -}}
postgres://{{ .Values.postgresql.appUser }}:{{ .Values.postgresql.appPassword | default "changeme-app" }}@{{ include "marula.fullname" . }}-postgres:5432/{{ .Values.postgresql.database }}
{{- else -}}
{{- required "postgresql.externalAppUrl required when postgresql.deploy=false" (.Values.postgresql.externalAppUrl) -}}
{{- end -}}
{{- end -}}

{{- define "marula.redisUrl" -}}
{{- if .Values.redis.deploy -}}
redis://{{ include "marula.fullname" . }}-redis:6379/0
{{- else -}}
{{- required "redis.externalUrl required when redis.deploy=false" .Values.redis.externalUrl -}}
{{- end -}}
{{- end -}}
