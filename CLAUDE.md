# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

s2d2 is an email forwarding and spam detection system built as a Cloudflare Worker. It receives [SendGrid Inbound Parse Webhook](https://docs.sendgrid.com/for-developers/parsing-email/setting-up-the-inbound-parse-webhook) requests, filters spam, and forwards emails to Discord webhooks for notification.

## Tech Stack

- **Language**: Rust (Edition 2021)
- **Runtime**: Cloudflare Workers (WASM)
- **Build**: Cargo with `worker-build`
- **Deployment**: Wrangler CLI

## Build Commands

```bash
# Build for deployment (used by wrangler)
cargo install -q worker-build && worker-build --release

# Local development
cargo build --release
cargo fmt              # Format code
cargo clippy           # Lint

# Local testing with Wrangler
wrangler dev           # Uses .dev.vars for local webhook URL

# Deploy
wrangler deploy
```

## Architecture

### Source Files
- `src/lib.rs` - Main worker fetch handler, Discord webhook integration, attachment processing
- `src/email.rs` - `Email` struct for parsing and validating form data

### Data Flow
1. SendGrid Inbound Parse Webhook sends HTTP POST with multipart/form-data
2. `Email::from_form_data()` parses and validates (filters `[SPAM]` subjects)
3. Spam score checked against threshold (env: `spam_score_threshold`, default 5.0)
4. Discord embed built with color coding (red=high spam, blue=low, black=unknown)
5. Webhook URL resolved from KV store (`WEBHOOK_URLS`) by recipient or "default"
6. POST to Discord webhook with embed and attachments

### SendGrid Form Fields
`from`, `to`, `subject`, `text`, `attachment-info` (JSON), `spam_score`

### Key Constants
- Email body truncation: 1000 characters
- Attachment size limit: 10MB cumulative
- Discord embeds use Japanese labels

## Patterns

- Use `anyhow::Result<T>` for error handling with `?` propagation
- Async/await for all I/O operations
- `console_log!` / `console_error!` macros for debugging in WASM
- Regex for email address extraction from headers
