# Server & API

DataSynth includes a server (`datasynth-server`) that exposes generation capabilities over REST, gRPC, and WebSocket.

## Starting the Server

```bash
cargo run -p datasynth-server -- --port 3000 --worker-threads 4
```

## REST Endpoints

### Health & Metrics

| Method | Path | Description |
|--------|------|-------------|
| GET | `/health` | Health check |
| GET | `/ready` | Readiness probe |
| GET | `/live` | Liveness probe |
| GET | `/api/metrics` | Application metrics (JSON) |
| GET | `/metrics` | Prometheus metrics |

### Configuration

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/config` | Get current configuration |
| POST | `/api/config` | Update configuration |
| POST | `/api/config/reload` | Reload config from file |

### Generation

| Method | Path | Description |
|--------|------|-------------|
| POST | `/api/generate/bulk` | Run bulk generation job |
| POST | `/api/stream/start` | Start streaming generation |
| POST | `/api/stream/stop` | Stop streaming |
| POST | `/api/stream/pause` | Pause streaming |
| POST | `/api/stream/resume` | Resume streaming |
| POST | `/api/stream/trigger/{pattern}` | Trigger specific pattern |
| GET | `/api/stream/ndjson` | Stream output as NDJSON |

### Job Management

| Method | Path | Description |
|--------|------|-------------|
| POST | `/api/jobs/submit` | Submit a generation job |
| GET | `/api/jobs` | List all jobs |
| GET | `/api/jobs/{id}` | Get job status |
| POST | `/api/jobs/{id}/cancel` | Cancel a job |

## WebSocket

| Path | Description |
|------|-------------|
| `/ws/metrics` | Real-time metrics stream |
| `/ws/events` | Generation event stream |

Connect with any WebSocket client:

```javascript
const ws = new WebSocket("ws://localhost:3000/ws/events");
ws.onmessage = (event) => console.log(JSON.parse(event.data));
```

## Authentication

API key authentication via the `X-API-Key` header:

```bash
curl -H "X-API-Key: your-key-here" http://localhost:3000/api/config
```

Bearer token authentication is also supported via the `Authorization` header.

Configure keys in the server config or environment. Health and metrics endpoints are exempt from authentication.

## Rate Limiting

Built-in rate limiting with configurable limits per client. Supports:
- In-memory rate limiter (default)
- Redis-backed rate limiter for multi-instance deployments

Rate limits use client IP identification via `X-Forwarded-For` and `X-Real-IP` headers.

## gRPC

The gRPC service mirrors the REST API for generation and streaming. Authentication uses the `authorization` metadata key.

## Security Headers

All responses include security headers:
- `X-Content-Type-Options: nosniff`
- `X-Frame-Options: DENY`
- `Cache-Control: no-store`
- `Content-Security-Policy: default-src 'none'`
- `Referrer-Policy: strict-origin-when-cross-origin`
