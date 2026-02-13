# kiro-rs

An Anthropic Claude API compatible proxy service written in Rust that converts Anthropic API requests to Kiro API requests.

## Disclaimer

This project is for research purposes only. Use at your own risk. Any consequences resulting from the use of this project are the responsibility of the user and are not related to this project.
This project is not affiliated with AWS/KIRO/Anthropic/Claude or any official entities and does not represent any official position.

## Important Notice

Since TLS has been switched from native-tls to rustls by default, you may need to install certificates before configuring an HTTP proxy. You can switch back to `native-tls` via the `tlsBackend` field in `config.json`.
If you encounter request errors, especially when unable to refresh tokens or receiving direct error responses, try switching the TLS backend to `native-tls`, which usually resolves the issue.

**Write Failed/Session Freeze**: If you encounter persistent Write File / Write Failed errors causing the session to become unusable, refer to Issue [#22](https://github.com/hank9999/kiro.rs/issues/22) and [#49](https://github.com/hank9999/kiro.rs/issues/49) for explanations and temporary solutions (usually related to output being truncated due to length; try lowering the output token limit).

## Features

- **Anthropic API Compatible**: Full support for Anthropic Claude API format
- **Streaming Responses**: Support for SSE (Server-Sent Events) streaming output
- **Automatic Token Refresh**: Automatic OAuth token management and refresh
- **Multi-Credential Support**: Configure multiple credentials with automatic priority-based failover
- **Load Balancing**: Support for `priority` (by priority) and `balanced` (even distribution) modes
- **Smart Retry**: Up to 3 retries per credential, up to 9 retries per request
- **Credential Writeback**: Automatic writeback of refreshed tokens in multi-credential format
- **Thinking Mode**: Support for Claude's extended thinking feature
- **Tool Calling**: Full support for function calling / tool use
- **WebSearch**: Built-in WebSearch tool conversion logic
- **Multi-Model Support**: Support for Sonnet, Opus, and Haiku model series
- **Admin Management**: Optional web management interface and API for credential management, balance queries, etc.
- **Multi-Level Region Configuration**: Support for global and credential-level Auth Region / API Region configuration

---

- [Getting Started](#getting-started)
  - [1. Build](#1-build)
  - [2. Minimal Configuration](#2-minimal-configuration)
  - [3. Start](#3-start)
  - [4. Verify](#4-verify)
  - [Docker](#docker)
- [OAuth Web Authentication](#oauth-web-authentication)
- [Configuration Details](#configuration-details)
  - [config.json](#configjson)
  - [credentials.json](#credentialsjson)
  - [Region Configuration](#region-configuration)
  - [Authentication Methods](#authentication-methods)
  - [Environment Variables](#environment-variables)
- [Usage with AI Tools](#usage-with-ai-tools)
  - [Claude Code CLI](#claude-code-cli)
  - [Anthropic Python SDK](#anthropic-python-sdk)
  - [Other AI Tools](#other-ai-tools)
- [API Endpoints](#api-endpoints)
  - [Standard Endpoints (/v1)](#standard-endpoints-v1)
  - [Claude Code Compatible Endpoints (/cc/v1)](#claude-code-compatible-endpoints-ccv1)
  - [Thinking Mode](#thinking-mode)
  - [Tool Calling](#tool-calling)
- [Model Mapping](#model-mapping)
- [Admin (Optional)](#admin-optional)
- [Notes](#notes)
- [Project Structure](#project-structure)
- [Tech Stack](#tech-stack)
- [License](#license)
- [Acknowledgments](#acknowledgments)

## Getting Started

### 1. Build

> Note: If you don't want to build, you can download pre-built binaries from the Releases page.

> **Prerequisites**: Before building, you need to build the Admin UI frontend (to be embedded in the binary):
> ```bash
> cd admin-ui && pnpm install && pnpm build
> ```

```bash
cargo build --release
```

### 2. Minimal Configuration

Create `config.json`:

```json
{
   "host": "127.0.0.1",
   "port": 8990,
   "apiKey": "sk-kiro-rs-qazWSXedcRFV123456",
   "region": "us-east-1"
}
```
> Note: If you need the web management panel, make sure to configure `adminApiKey`.

Create `credentials.json` (obtain credential information from Kiro IDE, etc.):
> Note: You can skip this step by configuring credentials via the web management panel.
> If you have questions about credential regions, see [Region Configuration](#region-configuration).

Social authentication:
```json
{
   "refreshToken": "your-refresh-token",
   "expiresAt": "2025-12-31T02:32:45.144Z",
   "authMethod": "social"
}
```

IdC authentication:
```json
{
   "refreshToken": "your-refresh-token",
   "expiresAt": "2025-12-31T02:32:45.144Z",
   "authMethod": "idc",
   "clientId": "your-client-id",
   "clientSecret": "your-client-secret"
}
```

### 3. Start

```bash
./target/release/kiro-rs
```

Or specify configuration file paths:

```bash
./target/release/kiro-rs -c /path/to/config.json --credentials /path/to/credentials.json
```

### 4. Verify

```bash
curl http://127.0.0.1:8990/v1/messages \
  -H "Content-Type: application/json" \
  -H "x-api-key: sk-kiro-rs-qazWSXedcRFV123456" \
  -d '{
    "model": "claude-sonnet-4-20250514",
    "max_tokens": 1024,
    "stream": true,
    "messages": [
      {"role": "user", "content": "Hello, Claude!"}
    ]
  }'
```

### Docker

You can also start via Docker:

```bash
docker-compose up
```

You need to mount `config.json` and `credentials.json` into the container. See `docker-compose.yml` for details.

## OAuth Web Authentication

Access the Kiro OAuth web interface at:

```
http://your-server:8990/v0/oauth/kiro
```

This provides a browser-based OAuth flow for Kiro (AWS CodeWhisperer) authentication with:

- **AWS Builder ID**: Personal AWS account authentication using device code flow
- **AWS Identity Center (IDC)**: Organization SSO authentication with custom start URL and region
- **Token Import**: Import refresh token from Kiro IDE (`~/.kiro/kiro-auth-token.json`)
- **Manual Refresh**: Refresh all configured tokens with one click

### OAuth Web Endpoints

| Endpoint                   | Method | Description                          |
|----------------------------|--------|--------------------------------------|
| `/v0/oauth/kiro`           | GET    | Authentication method selection page |
| `/v0/oauth/kiro/start`     | GET    | Start OAuth flow                     |
| `/v0/oauth/kiro/status`    | GET    | Poll session status (JSON)           |
| `/v0/oauth/kiro/import`    | POST   | Import refresh token                 |
| `/v0/oauth/kiro/refresh`   | POST   | Manual refresh all tokens            |

### How to Use

**Option 1: Via Admin UI (Recommended)**

1. Open Admin UI at `http://your-server:8990/admin`
2. Click "Add Credential" button
3. Click "Login via Browser (Recommended)" button
4. Follow the OAuth flow in the new window

**Option 2: Direct URL**

1. Open `http://your-server:8990/v0/oauth/kiro` in your browser
2. Choose an authentication method:
   - **AWS Builder ID**: Click the button, a new tab opens with AWS login page. Enter the verification code shown on the page.
   - **AWS Identity Center**: Enter your organization's Start URL and region, then follow the same flow.
   - **Import Token**: Copy the `refreshToken` from `~/.kiro/kiro-auth-token.json` and paste it.
3. Once authenticated, the token is automatically saved to `credentials.json`

### Token Import from Kiro IDE

If you have Kiro IDE installed and logged in:

1. Find the token file: `~/.kiro/kiro-auth-token.json`
2. Copy the `refreshToken` value (starts with `aorAAAAAG...`)
3. Paste it in the "Import RefreshToken" form on the OAuth web page
4. Click "Import Token"

## Configuration Details

### config.json

| Field                 | Type   | Default     | Description                                                                   |
|-----------------------|--------|-------------|-------------------------------------------------------------------------------|
| `host`                | string | `127.0.0.1` | Service listen address                                                        |
| `port`                | number | `8080`      | Service listen port                                                           |
| `apiKey`              | string | -           | Custom API key (for client authentication, required)                          |
| `region`              | string | `us-east-1` | AWS region                                                                    |
| `authRegion`          | string | -           | Auth Region (for token refresh), falls back to region if not configured       |
| `apiRegion`           | string | -           | API Region (for API requests), falls back to region if not configured         |
| `kiroVersion`         | string | `0.9.2`     | Kiro version number                                                           |
| `machineId`           | string | -           | Custom machine ID (64-bit hex), auto-generated if not defined                 |
| `systemVersion`       | string | random      | System version identifier                                                     |
| `nodeVersion`         | string | `22.21.1`   | Node.js version identifier                                                    |
| `tlsBackend`          | string | `rustls`    | TLS backend: `rustls` or `native-tls`                                         |
| `countTokensApiUrl`   | string | -           | External count_tokens API URL                                                 |
| `countTokensApiKey`   | string | -           | External count_tokens API key                                                 |
| `countTokensAuthType` | string | `x-api-key` | External API auth type: `x-api-key` or `bearer`                               |
| `proxyUrl`            | string | -           | HTTP/SOCKS5 proxy URL                                                         |
| `proxyUsername`       | string | -           | Proxy username                                                                |
| `proxyPassword`       | string | -           | Proxy password                                                                |
| `adminApiKey`         | string | -           | Admin API key, enables credential management API and web UI when set          |
| `loadBalancingMode`   | string | `priority`  | Load balancing mode: `priority` (by priority) or `balanced` (even dist.)      |
| `thinkingSuffix`      | string | `-thinking` | Model name suffix to trigger thinking mode (e.g., `claude-sonnet-4-thinking`) |
| `thinkingFormat`      | string | `thinking`  | Thinking output format: `thinking`, `think`, or `reasoning_content`           |
| `maxRequestBodyBytes` | number | `400000`    | Maximum request body size in bytes (0 = unlimited)                            |

Full configuration example:

```json
{
   "host": "127.0.0.1",
   "port": 8990,
   "apiKey": "sk-kiro-rs-qazWSXedcRFV123456",
   "region": "us-east-1",
   "tlsBackend": "rustls",
   "kiroVersion": "0.9.2",
   "machineId": "64-bit-hex-machine-id",
   "systemVersion": "darwin#24.6.0",
   "nodeVersion": "22.21.1",
   "authRegion": "us-east-1",
   "apiRegion": "us-east-1",
   "countTokensApiUrl": "https://api.example.com/v1/messages/count_tokens",
   "countTokensApiKey": "sk-your-count-tokens-api-key",
   "countTokensAuthType": "x-api-key",
   "proxyUrl": "http://127.0.0.1:7890",
   "proxyUsername": "user",
   "proxyPassword": "pass",
   "adminApiKey": "sk-admin-your-secret-key",
   "loadBalancingMode": "priority",
   "thinkingSuffix": "-thinking",
   "thinkingFormat": "thinking",
   "maxRequestBodyBytes": 400000
}
```

### credentials.json

Supports single object format (backward compatible) or array format (multi-credential).

#### Field Description

| Field          | Type   | Description                                                          |
|----------------|--------|----------------------------------------------------------------------|
| `id`           | number | Unique credential ID (optional, only for Admin API management)       |
| `accessToken`  | string | OAuth access token (optional, auto-refreshed)                        |
| `refreshToken` | string | OAuth refresh token                                                  |
| `profileArn`   | string | AWS Profile ARN (optional, returned on login)                        |
| `expiresAt`    | string | Token expiration time (RFC3339)                                      |
| `authMethod`   | string | Authentication method: `social` or `idc`                             |
| `clientId`     | string | IdC login client ID (required for IdC auth)                          |
| `clientSecret` | string | IdC login client secret (required for IdC auth)                      |
| `priority`     | number | Credential priority, lower number = higher priority, default is 0    |
| `region`       | string | Credential-level Auth Region, compatibility field                    |
| `authRegion`   | string | Credential-level Auth Region for token refresh, falls back to region |
| `apiRegion`    | string | Credential-level API Region for API requests                         |
| `machineId`    | string | Credential-level machine ID (64-bit hex)                             |
| `email`        | string | User email (optional, obtained from API)                             |

Notes:
- IdC / Builder-ID / IAM are treated as the same login method in this project; use `authMethod: "idc"` for configuration
- For backward compatibility, `builder-id` / `iam` are still recognized but processed as `idc`

#### Single Credential Format (Legacy, Backward Compatible)

```json
{
   "accessToken": "request-token-usually-valid-for-one-hour-optional",
   "refreshToken": "refresh-token-usually-valid-for-7-30-days",
   "profileArn": "arn:aws:codewhisperer:us-east-1:111112222233:profile/QWER1QAZSDFGH",
   "expiresAt": "2025-12-31T02:32:45.144Z",
   "authMethod": "social",
   "clientId": "required-for-idc-login",
   "clientSecret": "required-for-idc-login"
}
```

#### Multi-Credential Format (Supports Failover and Auto-Writeback)

```json
[
   {
      "refreshToken": "first-credential-refresh-token",
      "expiresAt": "2025-12-31T02:32:45.144Z",
      "authMethod": "social",
      "priority": 0
   },
   {
      "refreshToken": "second-credential-refresh-token",
      "expiresAt": "2025-12-31T02:32:45.144Z",
      "authMethod": "idc",
      "clientId": "xxxxxxxxx",
      "clientSecret": "xxxxxxxxx",
      "region": "us-east-2",
      "priority": 1
   }
]
```

Multi-credential features:
- Sorted by `priority` field, lower number = higher priority (default is 0)
- Up to 3 retries per credential, up to 9 retries per request
- Automatic failover to the next available credential
- Automatic writeback of refreshed tokens to source file in multi-credential format

### Region Configuration

Supports multi-level region configuration to separately control regions for token refresh and API requests.

**Auth Region** (Token Refresh) Priority:
`credential.authRegion` > `credential.region` > `config.authRegion` > `config.region`

**API Region** (API Requests) Priority:
`credential.apiRegion` > `config.apiRegion` > `config.region`

### Authentication Methods

When clients make requests to this service, two authentication methods are supported:

1. **x-api-key Header**
   ```
   x-api-key: sk-your-api-key
   ```

2. **Authorization Bearer**
   ```
   Authorization: Bearer sk-your-api-key
   ```

### Environment Variables

You can configure the log level via environment variables:

```bash
RUST_LOG=debug ./target/release/kiro-rs
```

## Usage with AI Tools

### Claude Code CLI

Configure Claude Code CLI to use kiro-rs as the API backend:

```bash
# Method 1: Environment variables
export ANTHROPIC_BASE_URL="http://127.0.0.1:8990"
export ANTHROPIC_API_KEY="sk-kiro-rs-qazWSXedcRFV123456"

# Method 2: Claude Code config command
claude config set --global apiBaseUrl "http://127.0.0.1:8990"
claude config set --global apiKey "sk-kiro-rs-qazWSXedcRFV123456"
```

### Cursor

Configure in Cursor settings (`settings.json`):

```json
{
  "anthropic.baseUrl": "http://127.0.0.1:8990",
  "anthropic.apiKey": "sk-kiro-rs-qazWSXedcRFV123456"
}
```

Or via UI: Settings > Models > Anthropic > Base URL

### Cline (VS Code Extension)

1. Open VS Code Settings
2. Search for "Cline"
3. Set API Provider to "Anthropic"
4. Set Base URL: `http://127.0.0.1:8990`
5. Set API Key: `sk-kiro-rs-qazWSXedcRFV123456`

### Continue

Edit `~/.continue/config.json`:

```json
{
  "models": [
    {
      "title": "Claude via kiro-rs",
      "provider": "anthropic",
      "model": "claude-sonnet-4-20250514",
      "apiBase": "http://127.0.0.1:8990",
      "apiKey": "sk-kiro-rs-qazWSXedcRFV123456"
    }
  ]
}
```

### Roo Code / Other Tools

Any tool that supports custom Anthropic API endpoints can be configured:

| Tool       | Base URL Setting              | API Key Setting          |
|------------|-------------------------------|--------------------------|
| Roo Code   | Settings > API > Base URL     | Settings > API > API Key |
| Kilo Code  | Extension Settings > Base URL | Extension Settings > Key |
| aider      | `--anthropic-api-base` flag   | `ANTHROPIC_API_KEY` env  |
| OpenRouter | Custom endpoint configuration | API Key field            |

### Anthropic Python SDK

```python
from anthropic import Anthropic

client = Anthropic(
    base_url="http://127.0.0.1:8990",
    api_key="sk-kiro-rs-qazWSXedcRFV123456"
)

# Non-streaming
message = client.messages.create(
    model="claude-sonnet-4-20250514",
    max_tokens=1024,
    messages=[{"role": "user", "content": "Hello!"}]
)
print(message.content[0].text)

# Streaming
with client.messages.stream(
    model="claude-sonnet-4-20250514",
    max_tokens=1024,
    messages=[{"role": "user", "content": "Hello!"}]
) as stream:
    for text in stream.text_stream:
        print(text, end="", flush=True)
```

### cURL

```bash
curl http://127.0.0.1:8990/v1/messages \
  -H "Content-Type: application/json" \
  -H "x-api-key: sk-kiro-rs-qazWSXedcRFV123456" \
  -H "anthropic-version: 2023-06-01" \
  -d '{
    "model": "claude-sonnet-4-20250514",
    "max_tokens": 1024,
    "stream": true,
    "messages": [{"role": "user", "content": "Hello!"}]
  }'
```

## API Endpoints

### Standard Endpoints (/v1)

| Endpoint                    | Method | Description              |
|-----------------------------|--------|--------------------------|
| `/v1/models`                | GET    | Get available model list |
| `/v1/messages`              | POST   | Create message (chat)    |
| `/v1/messages/count_tokens` | POST   | Estimate token count     |

### Claude Code Compatible Endpoints (/cc/v1)

| Endpoint                       | Method | Description                                           |
|--------------------------------|--------|-------------------------------------------------------|
| `/cc/v1/messages`              | POST   | Create message (buffered mode, accurate input_tokens) |
| `/cc/v1/messages/count_tokens` | POST   | Estimate token count (same as `/v1`)                  |

> **Difference between `/cc/v1/messages` and `/v1/messages`**:
> - `/v1/messages`: Real-time streaming, `input_tokens` in `message_start` is an estimate
> - `/cc/v1/messages`: Buffered mode, waits for upstream stream to complete, corrects `message_start` with accurate `input_tokens` calculated from `contextUsageEvent`, then returns all events at once
> - Sends `ping` events every 25 seconds during the wait to keep the connection alive

### Thinking Mode

Supports Claude's extended thinking feature:

```json
{
  "model": "claude-sonnet-4-20250514",
  "max_tokens": 16000,
  "thinking": {
    "type": "enabled",
    "budget_tokens": 10000
  },
  "messages": [...]
}
```

#### Thinking Suffix Trigger

This feature allows you to enable thinking mode simply by adding a suffix to the model name, without modifying the request body. This is especially useful for tools that don't support the `thinking` parameter directly.

**How it works:**

1. Client sends a request with model name like `claude-sonnet-4-thinking`
2. The proxy detects the `-thinking` suffix and:
   - Strips the suffix from the model name (`claude-sonnet-4-thinking` -> `claude-sonnet-4`)
   - Automatically enables thinking mode with default budget
   - Injects a system prompt to guide the model's reasoning process
3. The response includes the model's thinking process based on the configured format

**Configuration Options:**

| Option           | Default     | Description                                                     |
|------------------|-------------|-----------------------------------------------------------------|
| `thinkingSuffix` | `-thinking` | The suffix to trigger thinking mode (e.g., `-think`, `-reason`) |
| `thinkingFormat` | `thinking`  | Output format for the thinking content                          |

**thinkingFormat Values:**

| Value               | Description                                            | Use Case                          |
|---------------------|--------------------------------------------------------|-----------------------------------|
| `thinking`          | Wraps thinking in `<thinking>...</thinking>` tags      | Standard Anthropic format         |
| `think`             | Wraps thinking in `<think>...</think>` tags            | Alternative tag format            |
| `reasoning_content` | Returns thinking as separate `reasoning_content` field | OpenAI/DeepSeek compatible format |

**Example Usage:**

```bash
# Instead of adding thinking parameter to request body:
curl http://127.0.0.1:8990/v1/messages \
  -H "Content-Type: application/json" \
  -H "x-api-key: your-api-key" \
  -d '{
    "model": "claude-sonnet-4-thinking",
    "max_tokens": 16000,
    "messages": [{"role": "user", "content": "Solve: What is 15 * 23?"}]
  }'
```

**Custom Suffix Example:**

If you set `"thinkingSuffix": "-reason"` in config.json, you would use:
```json
{
  "model": "claude-sonnet-4-reason",
  "messages": [...]
}
```

### Tool Calling

Full support for Anthropic's tool use feature:

```json
{
  "model": "claude-sonnet-4-20250514",
  "max_tokens": 1024,
  "tools": [
    {
      "name": "get_weather",
      "description": "Get weather for a specified city",
      "input_schema": {
        "type": "object",
        "properties": {
          "city": {"type": "string"}
        },
        "required": ["city"]
      }
    }
  ],
  "messages": [...]
}
```

## Model Mapping

The proxy maps Anthropic model names to Kiro internal model IDs. For Sonnet models, version-specific mapping is used:

| Anthropic Model                  | Kiro Internal Model ID            |
|----------------------------------|-----------------------------------|
| `*sonnet*4.5*` or `*sonnet*4-5*` | `CLAUDE_SONNET_4_5_20250929_V1_0` |
| `*sonnet*4*` (not 4.5)           | `CLAUDE_SONNET_4_20250514_V1_0`   |
| `*sonnet*3.7*` or `*sonnet*3-7*` | `CLAUDE_3_7_SONNET_20250219_V1_0` |
| `*sonnet*` (other)               | `CLAUDE_SONNET_4_5_20250929_V1_0` |
| `*opus*` (with 4.5/4-5)          | `claude-opus-4.5`                 |
| `*opus*` (others)                | `claude-opus-4.6`                 |
| `*haiku*`                        | `claude-haiku-4.5`                |

### Available Models

The `/v1/models` endpoint returns the following models:

| Model ID                              | Display Name                           | Context | Thinking |
|---------------------------------------|----------------------------------------|---------|----------|
| `claude-sonnet-4-5-20250929`          | Claude Sonnet 4.5                      | 200K    | No       |
| `claude-sonnet-4-5-20250929-thinking` | Claude Sonnet 4.5 (Thinking)           | 200K    | Yes      |
| `claude-sonnet-4-5-20250929-agentic`  | Claude Sonnet 4.5 (Agentic)            | 200K    | No       |
| `claude-opus-4-5-20251101`            | Claude Opus 4.5                        | 200K    | No       |
| `claude-opus-4-5-20251101-thinking`   | Claude Opus 4.5 (Thinking)             | 200K    | Yes      |
| `claude-opus-4-5-20251101-agentic`    | Claude Opus 4.5 (Agentic)              | 200K    | No       |
| `claude-opus-4-6`                     | Claude Opus 4.6                        | 200K    | No       |
| `claude-opus-4-6-thinking`            | Claude Opus 4.6 (Thinking)             | 200K    | Yes      |
| `claude-opus-4-6-agentic`             | Claude Opus 4.6 (Agentic)              | 200K    | No       |
| `claude-opus-4-6-1m`                  | Claude Opus 4.6 (1M Context)           | 1M      | No       |
| `claude-opus-4-6-1m-thinking`         | Claude Opus 4.6 (1M Context, Thinking) | 1M      | Yes      |
| `claude-opus-4-6-1m-agentic`          | Claude Opus 4.6 (1M, Agentic)          | 1M      | No       |
| `claude-haiku-4-5-20251001`           | Claude Haiku 4.5                       | 200K    | No       |
| `claude-haiku-4-5-20251001-thinking`  | Claude Haiku 4.5 (Thinking)            | 200K    | Yes      |
| `claude-haiku-4-5-20251001-agentic`   | Claude Haiku 4.5 (Agentic)             | 200K    | No       |

> Note: Models ending with `-thinking` automatically enable extended thinking mode with a 20,000 token budget (max 128,000).

> Note: Opus 4.6 `-1m` variants support 1 million token context window for large codebases and projects.

> Note: Models ending with `-agentic` inject a system prompt that guides Claude to write files in chunks, preventing truncation issues with large file operations.

## Error Enhancement

The proxy enhances cryptic Kiro API error messages with user-friendly explanations:

| Error Code                         | User-Friendly Message                                                                   |
|------------------------------------|-----------------------------------------------------------------------------------------|
| `CONTENT_LENGTH_EXCEEDS_THRESHOLD` | Content length exceeds the maximum allowed limit. Please reduce your input size.        |
| `MONTHLY_REQUEST_LIMIT_REACHED`    | Monthly request limit reached. Please wait until next month or upgrade your plan.       |
| `MONTHLY_REQUEST_COUNT`            | Monthly request count limit reached. Please wait until next month or upgrade your plan. |
| `RATE_LIMIT_EXCEEDED`              | Rate limit exceeded. Please slow down your requests and try again.                      |
| `SERVICE_UNAVAILABLE`              | Kiro service is temporarily unavailable. Please try again later.                        |
| `THROTTLING_EXCEPTION`             | Request throttled due to high traffic. Please wait a moment and retry.                  |
| `VALIDATION_EXCEPTION`             | Request validation failed. Please check your input parameters.                          |

## Admin (Optional)

When `adminApiKey` is configured in `config.json`, the following are enabled:

- **Admin API (authenticated with API Key)**
  - `GET /api/admin/credentials` - Get all credential statuses
  - `POST /api/admin/credentials` - Add new credential
  - `DELETE /api/admin/credentials/:id` - Delete credential
  - `POST /api/admin/credentials/:id/disabled` - Set credential disabled status
  - `POST /api/admin/credentials/:id/priority` - Set credential priority
  - `POST /api/admin/credentials/:id/reset` - Reset failure count
  - `GET /api/admin/credentials/:id/balance` - Get credential balance

- **Admin UI**
  - `GET /admin` - Access management page (requires building `admin-ui/dist` before compilation)

## Notes

1. **Credential Security**: Keep your `credentials.json` file secure and do not commit it to version control
2. **Token Refresh**: The service automatically refreshes expired tokens without manual intervention
3. **WebSearch Tool**: When the `tools` list contains only a single `web_search` tool, the built-in WebSearch conversion logic is used

## Project Structure

```
kiro-rs/
+-- src/
|   +-- main.rs                 # Entry point
|   +-- http_client.rs          # HTTP client builder
|   +-- token.rs                # Token calculation module
|   +-- debug.rs                # Debug tools
|   +-- test.rs                 # Tests
|   +-- model/                  # Configuration and parameter models
|   |   +-- config.rs           # Application configuration
|   |   +-- arg.rs              # Command line arguments
|   +-- anthropic/              # Anthropic API compatibility layer
|   |   +-- router.rs           # Route configuration
|   |   +-- handlers.rs         # Request handlers
|   |   +-- middleware.rs       # Authentication middleware
|   |   +-- types.rs            # Type definitions
|   |   +-- converter.rs        # Protocol converter
|   |   +-- stream.rs           # Streaming response handling
|   |   +-- websearch.rs        # WebSearch tool handling
|   |   +-- tool_compression.rs # Tool payload compression
|   |   +-- truncation.rs       # Tool call truncation detection
|   +-- kiro/                   # Kiro API client
|   |   +-- provider.rs         # API provider
|   |   +-- token_manager.rs    # Token management
|   |   +-- machine_id.rs       # Device fingerprint generation
|   |   +-- errors.rs           # Error enhancement module
|   |   +-- model/              # Data models
|   |   |   +-- credentials.rs  # OAuth credentials
|   |   |   +-- events/         # Response event types
|   |   |   +-- requests/       # Request types
|   |   |   +-- common/         # Shared types
|   |   |   +-- token_refresh.rs# Token refresh model
|   |   |   +-- usage_limits.rs # Usage quota model
|   |   +-- parser/             # AWS Event Stream parser
|   |       +-- decoder.rs      # Stream decoder
|   |       +-- frame.rs        # Frame parsing
|   |       +-- header.rs       # Header parsing
|   |       +-- error.rs        # Error types
|   |       +-- crc.rs          # CRC validation
|   +-- admin/                  # Admin API module
|   |   +-- router.rs           # Route configuration
|   |   +-- handlers.rs         # Request handlers
|   |   +-- service.rs          # Business logic service
|   |   +-- types.rs            # Type definitions
|   |   +-- middleware.rs       # Authentication middleware
|   |   +-- error.rs            # Error handling
|   +-- admin_ui/               # Admin UI static file embedding
|   |   +-- router.rs           # Static file routing
|   +-- common/                 # Common modules
|       +-- auth.rs             # Authentication utility functions
+-- admin-ui/                   # Admin UI frontend (build output embedded in binary)
+-- tools/                      # Utility tools
+-- Cargo.toml                  # Project configuration
+-- config.example.json         # Configuration example
+-- docker-compose.yml          # Docker Compose configuration
+-- Dockerfile                  # Docker build file
```

## Tech Stack

- **Web Framework**: [Axum](https://github.com/tokio-rs/axum) 0.8
- **Async Runtime**: [Tokio](https://tokio.rs/)
- **HTTP Client**: [Reqwest](https://github.com/seanmonstar/reqwest)
- **Serialization**: [Serde](https://serde.rs/)
- **Logging**: [tracing](https://github.com/tokio-rs/tracing)
- **CLI**: [Clap](https://github.com/clap-rs/clap)

## License

MIT

## Acknowledgments

This project's implementation would not have been possible without the efforts of predecessors:
 - [kiro2api](https://github.com/caidaoli/kiro2api)
 - [proxycast](https://github.com/aiclientproxy/proxycast)

Parts of this project's logic were referenced from the above projects. Sincere thanks!
