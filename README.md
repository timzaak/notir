# Notir

[![Release](https://img.shields.io/github/v/release/timzaak/notir)](https://github.com/timzaak/notir/pkgs/container/notir)

`Notir` is a lightweight WebSocket server built with Rust using the Salvo web framework and Tokio. It allows users to connect via WebSockets, subscribe to a real-time message feed, and publish messages to other connected clients.

Feel free to open an issue anytime, for any reason.

## Features

- WebSocket communication for real-time messaging.
- Simple publish/subscribe model.
- Browser page for custom WebSocket message handling.
- Small shared clipboard web app built on top of broadcast mode, including
  on-demand file transfer between online pages.
- Containerized with Docker for easy deployment.

## Getting Started

### Quick Try

It has been deployed on the public server, you can try it out right away:
```
https://notir.fornetcode.com/handler?id=${uuid}
```

Please change `uuid` to whatever you want, and now you can publish messages to
the server like this:

```bash
# Single mode - Point-to-point messaging
curl -X POST https://notir.fornetcode.com/single/pub?id=${uuid} \
 -H 'Content-Type: application/json' \
 -d '{"msg": "hello world"}'
 
# Single mode with PingPong - Two-way communication
curl -X POST https://notir.fornetcode.com/single/pub?id=${uuid}&mode=ping_pong \
 -H 'Content-Type: application/json' \
 -d '{"msg": "hello world"}'

# Broadcast mode - Message to all subscribers of a channel
curl -X POST https://notir.fornetcode.com/broad/pub?id=${uuid} \
 -H 'Content-Type: application/json' \
 -d '{"msg": "broadcast message"}'
```

<img src="/doc/img.png" alt="Usage screenshot" style="width: 100%" />

### Self Hosted

The easiest way to run `Notir` is by using the pre-built Docker image available
on GitHub Container Registry.

```bash
docker run -d -p 5800:5800 --name notir ghcr.io/timzaak/notir:latest

#The server will start on port 5800 by default. You can specify a different port using the `--port` or `-p` flag. 

docker run -d -p 8698:8698 --name notir ghcr.io/timzaak/notir:latest -- --port 8698
```

## Web UI

- `/handler?id=<user_id>`: Custom WebSocket message handler page for subscribing
  to raw messages and processing them with browser-side JavaScript.
- `/?id=<clipboard_id>`: Shared clipboard mini app. Multiple online pages with
  the same ID receive each other's latest text edits in real time. Pages can
  also send files to the channel: the file stays in the sender's browser and is
  only streamed through the server when another page downloads it.

## API Endpoints

### Single Mode (Point-to-Point Communication)

- `WS /single/sub?id=<user_id>`:
  - Establishes a WebSocket connection for a user to subscribe to messages.
  - Query Parameters:
    - `id` (required): A unique string identifier for the client. Cannot be
      empty.
  - Upgrades the connection to WebSocket. Messages from other users will be
    pushed to this WebSocket connection.
  - Supports bidirectional communication and heartbeat mechanism.

- `POST /single/pub?id=<user_id>&mode=<Mode>`:
  - Publishes a message to a specific connected client.
  - Query Parameters:
    - `id` (required): The unique string identifier of the target client. Cannot
      be empty.
    - `mode` (optional): The mode of communication. Can be `shot` or
      `ping_pong`, defaults to `shot`.
      - `shot`: One-way message delivery, no response expected.
      - `ping_pong`: Two-way communication, waits for client response within 5
        seconds.
  - Request Body: The message content.
    - If the `Content-Type` header is `application/json` or starts with `text/`
      (e.g., `text/plain`), the message is treated as a `UTF-8` text message.
    - Otherwise, the message is treated as binary.
  - Responses:
    - `200 OK`: If the message was successfully sent to the target user's
      channel.
    - `400 Bad Request`: If the `id` query parameter is missing or empty, or if
      a `text/*` body contains invalid UTF-8.
    - `404 Not Found`: If the specified `user_id` is not currently connected.
    - `408 Request Timeout`: If using `ping_pong` mode and no response received
      within 5 seconds.

### Broadcast Mode (One-to-Many Communication)

- `WS /broad/sub?id=<broadcast_id>`:
  - Establishes a WebSocket connection to subscribe to broadcast messages for a
    specific channel.
  - Query Parameters:
    - `id` (required): The broadcast channel identifier. Cannot be empty.
  - Multiple clients can subscribe to the same broadcast channel.
  - Receives messages from `broad/pub` as text frames; binary frames pushed by
    the server carry file transfer control messages (see below).
  - Client-sent messages are ignored except pong responses and the file
    transfer operations described below.
  - Supports heartbeat mechanism for connection health monitoring.

- `POST /broad/pub?id=<broadcast_id>`:
  - Broadcasts a message to all clients subscribed to the specified channel.
  - Query Parameters:
    - `id` (required): The broadcast channel identifier. Cannot be empty.
  - Request Body: The message content.
    - If the `Content-Type` header is `application/json` or starts with `text/`
      (e.g., `text/plain`), the message is treated as a `UTF-8` text message.
    - Otherwise, the message is treated as binary.
  - Responses:
    - `200 OK`: Always returns success, regardless of whether there are active
      subscribers.
    - `400 Bad Request`: If the `id` query parameter is missing or empty, or if
      a `text/*` body contains invalid UTF-8.

### File Transfer (Lazy Upload on Demand)

File transfer reuses the broadcast WebSocket. The server never stores file
bytes, only small in-memory metadata; the sender's browser holds the file and
streams it in chunks only when someone actually downloads it.

Frame conventions on `WS /broad/sub`:

- Client → server text frames are JSON ops:
  - `{"op":"offer","offerId":"...","name":"a.bin","size":123,"mime":"..."}`:
    register file metadata, server replies with an `offer_ok` control message.
  - `{"op":"done"}` / `{"op":"abort"}`: end the current transfer.
- Client → server binary frames are file chunks (256 KiB each), routed to the
  active transfer.
- Server → client text frames remain clipboard text (from `broad/pub`);
  server → client binary frames are JSON control messages: `offer_ok`,
  `pull`, `cancel`.

The sender announces the file to the channel itself by POSTing a JSON envelope
(`{"type":"notir-file","fileId":...,"name":...,"size":...,"mime":...}`) to
`/broad/pub` with a non-text `Content-Type`, so subscribers receive it as a
binary control frame.

HTTP endpoints:

- `GET /files/download/{file_id}`:
  - Asks the holder's WebSocket connection to stream the file and relays the
    chunks directly into the HTTP response (streaming, no disk writes).
  - Response headers: `Content-Type` (from the offer), `Content-Length`,
    `Content-Disposition: attachment`.
  - `404 Not Found`: unknown `file_id` (also returned after the holder
    disconnected, because offers die with their connection).
  - `410 Gone`: the offer exists but the holder connection is gone.
  - `409 Conflict`: another transfer is already in progress on the same
    holder connection; retry shortly.
- `GET /files/status/{file_id}`:
  - Returns `{"available": true|false}` depending on whether the offer exists
    and the holder is still connected.

Semantics and limits:

- The sender must stay online until the download completes; there is no
  offline store-and-forward and no resume of interrupted transfers.
- Transfers on the same holder connection are serialized; concurrent requests
  receive `409`.
- A transfer stalls if no chunk arrives within 60 seconds; the download is
    then aborted. If the receiver cancels, the holder is told to stop via a
    `cancel` control message.
- At most 32 pending offers per connection.

### General Endpoints

- `GET /health`: Health check endpoint, returns `200 OK` if the service is
  running.
- `GET /version`: Returns the current version of the service.
- `GET /connections?id=<user_id>`: Returns the number of active WebSocket connections for a given user ID.

## CLI Client

`notir-cli` connects to a Notir server, optionally transforms messages via a JS script, and outputs to console or file. Download from [Releases](https://github.com/timzaak/notir/releases) or use Docker:

```bash
docker run --rm ghcr.io/timzaak/notir-cli:latest --id myuser --server ws://your-server:5800
```

### Usage

```bash
# Subscribe and print raw messages
notir-cli --id myuser --server ws://localhost:5800

# Broadcast mode
notir-cli --id channel1 --mode broad

# Transform with JS script, output to file, auto-reconnect
notir-cli --id myuser --script transform.js --output file --output-dir ./data --reconnect

# Pipe to other tools
notir-cli --id myuser | jq .
```

### JS Transform

Write a `transform(event)` function. Return a string to output, `null` to discard.

```javascript
// transform.js — reformat and filter
function transform(event) {
  var data = JSON.parse(event.text);
  if (data.level !== "alert") return null; // discard non-alerts
  return JSON.stringify({ time: event.timestamp, msg: data });
}
```

`event` fields: `text` (string|null), `binary` (hex|null), `timestamp` (ISO 8601), `type` ("text"|"binary"), `source` ("single"|"broad").

## License

This project is dual-licensed under either:

- **Apache License 2.0** ([LICENSE-APACHE](LICENSE-APACHE) or
  http://www.apache.org/licenses/LICENSE-2.0)
- **MIT License** ([LICENSE-MIT](LICENSE-MIT) or
  http://opensource.org/licenses/MIT)

You may choose either license at your option.
