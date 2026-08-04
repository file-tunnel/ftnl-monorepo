# File Tunnel MCP observability

## Process ownership

`src/main.rs` is a thin Tokio bootstrap. `src/runtime.rs` owns telemetry initialization, construction of the read-only `FtnlMcp` server, startup diagnostics, the stdio lifecycle span, peer shutdown, and conversion of transport errors into the binary's public error type.

The MCP protocol owns **stdout**. Application diagnostics and structured JSON logs always use **stderr**. A write to stdout outside `rmcp` protocol framing is a correctness and privacy defect.

## Logs

The default filter is `info,hyper=warn`; override it with `RUST_LOG`. Startup logs report only whether optional configuration is present. They never include `FTNL_API_URL`, provider tokens, pairing secrets, capabilities, event tickets, presigned URLs, file names, paths selected by users, request arguments, response bodies, or transferred bytes.

## Traces and metrics

OTLP export is optional. Without `OTEL_EXPORTER_OTLP_ENDPOINT`, the server continues with stderr JSON logging only. A malformed or unavailable exporter also fails open without printing endpoint values or exporter error details.

Stable signals:

| Signal | Attributes | Privacy/cardinality rule |
| --- | --- | --- |
| span `mcp.server` | `rpc.system=mcp`, `transport=stdio` | One lifecycle span per process. |
| span `mcp.tool.call` | `rpc.system`, `rpc.method`, fixed `mcp.tool.name`, error status | No arguments, results, file metadata, capabilities, or provider payloads. |
| counter `mcp.server.tool.calls` | fixed tool name, Boolean error flag | Bounded by the compiled tool catalog. |
| histogram `mcp.server.tool.duration` | fixed tool name, Boolean error flag; milliseconds | No request- or tenant-derived labels. |

The exporter timeout is five seconds. Owned trace and metric providers are shut down during orderly process teardown so final batches can flush.

## Collector endpoint policy

`OTEL_EXPORTER_OTLP_ENDPOINT` is trimmed and accepted only when it is at most 2 KiB, contains no control characters, parses as HTTP or HTTPS, has a host, and has no embedded username, password, query string, or fragment. Collector credentials belong in the standard OTel header variables and are never copied into telemetry.

## Resource attributes

The process owns these resource fields:

- `service.name=ftnl-mcp-server`
- `service.namespace=file-tunnel`
- `service.version`
- `deployment.environment` from `DEPLOYMENT_ENV`
- `k8s.namespace.name` from `POD_NAMESPACE`
- `k8s.pod.name` from `POD_NAME`
- `k8s.node.name` from `NODE_NAME`
- `host.name` from `HOSTNAME`

`OTEL_RESOURCE_ATTRIBUTES` is limited to 16 KiB of raw input and 32 accepted entries. Keys are at most 128 ASCII alphanumeric/dot/underscore/hyphen bytes; values are at most 256 bytes with no controls. The first valid duplicate wins. Caller-supplied values cannot override process-owned identity, and credential-, token-, session-, email-, password-, authorization-, cookie-, or key-shaped names are discarded.

## Deployment integration

Send stderr JSON to the existing Promtail/Fluent Bit pipeline for Loki. Send OTLP traces and metrics to an OpenTelemetry Collector, then route traces to the chosen trace backend and metrics to Prometheus-compatible storage. Grafana dashboards should aggregate by service, namespace, fixed tool name, and error flag only.

Recommended alerts:

- sustained increase in `mcp.server.tool.calls{mcp.tool.error=true}`;
- p95/p99 `mcp.server.tool.duration` regression by fixed tool name;
- repeated process restarts or missing lifecycle spans;
- collector/exporter failure observed externally while stderr logs remain healthy.

Do not create labels from workspace paths, repository names supplied at runtime, tunnel IDs, user IDs, session IDs, URLs, file names, capability values, request arguments, or response contents.
