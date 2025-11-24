# Notir

[![Release](https://img.shields.io/github/v/release/timzaak/notir)](https://github.com/timzaak/notir/pkgs/container/notir)

`Notir` is a lightweight WebSocket server built with Rust using the Salvo web framework and Tokio. It allows users to connect via WebSockets, subscribe to a real-time message feed, and publish messages to other connected clients.

Feel free to open an issue anytime, for any reason.

## Features

- WebSocket communication for real-time messaging.
- Simple publish/subscribe model.
- Containerized with Docker for easy deployment.

## Getting Started

### Quick Try

It has been deployed on the public server, you can try it out right away:
```
http://notir.fornetcode.com:5800?id=${uuid}
```
Please change `uuid` to whatever you want, and now you can publish
messages to the server like this:

```bash
# Single mode - Point-to-point messaging
curl -X POST http://notir.fornetcode.com:5800/single/pub?id=${uuid} \
 -H 'Content-Type: application/json' \
 -d '{"msg": "hello world"}'
 
# Single mode with PingPong - Two-way communication
curl -X POST http://notir.fornetcode.com:5800/single/pub?id=${uuid}&mode=ping_pong \
 -H 'Content-Type: application/json' \
 -d '{"msg": "hello world"}'

# Broadcast mode - Message to all subscribers of a channel
curl -X POST http://notir.fornetcode.com:5800/broad/pub?id=${uuid} \
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
  - Only receives messages from `broad/pub`, ignores client-sent messages
    (except pong responses).
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

### General Endpoints

- `GET /health`: Health check endpoint, returns `200 OK` if the service is
  running.
- `GET /version`: Returns the current version of the service.
- `GET /connections?id=<user_id>`: Returns the number of active WebSocket connections for a given user ID.

## License

This project is dual-licensed under either:

- **Apache License 2.0** ([LICENSE-APACHE](LICENSE-APACHE) or
  http://www.apache.org/licenses/LICENSE-2.0)
- **MIT License** ([LICENSE-MIT](LICENSE-MIT) or
  http://opensource.org/licenses/MIT)

You may choose either license at your option.
