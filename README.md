# Notir

![Release](https://img.shields.io/github/v/release/timzaak/notir)

`Notir` is a lightweight WebSocket server built with Rust using the Salvo web framework and Tokio. It allows users to connect via WebSockets, subscribe to a real-time message feed, and publish messages to other connected clients.

## Features

- WebSocket communication for real-time messaging.
- Simple publish/subscribe model.
- Containerized with Docker for easy deployment.

## Getting Started

### Quick Try
It has been deployed on the public server, you can try it out right away: http://notir.fornecode.com:5800?id=${uuid} .

Please change `uuid` to whatever you want, and now you can publish messages to the server like this:
```bash
curl -X POST http://notir.fornecode.com:5800/pub?id=${uuid} \
 -H 'Content-Type: application/json' \
 -d '{"msg": "hello world"}'
 
# You can also publish with mode PingPong, which you can send response via WebSocket to the client.
curl -X POST http://notir.fornecode.com:5800/pub?id=${uuid}&mode=ping_pong \
 -H 'Content-Type: application/json' \
 -d '{"msg": "hello world"}'
  
```

<img src="/doc/img.png" alt="Usage screenshot" style="width: 100%" />

### Self Hosted

The easiest way to run `Notir` is by using the pre-built Docker image available on GitHub Container Registry.

```bash
docker run -d -p 5800:5800 --name notir ghcr.io/timzaak/notir:latest
# open browser: http://127.0.0.1:5800?id=test

# Publish the message via:
curl -X POST http://127.0.0.1:5800/pub?id=test \
 -H 'Content-Type: application/json' \
 -d '{"msg": "hello world"}'
```

## API Endpoints

*   `GET /sub?id=<user_id>?mode=<Mode>`:
    *   Establishes a WebSocket connection for a user to subscribe to messages.
    *   Query Parameters:
        *   `id` (required): A unique string identifier for the client. Cannot be empty.
        *   `mode` (optional): The mode of subscription. Can be `shot` or `ping_pong`. Defaults to `shot`, when use `ping_pong`, Subscriber(Websocket) can send message back to Publisher(http request as response).
    *   Upgrades the connection to WebSocket. Messages from other users will be pushed to this WebSocket connection.
*   `POST /pub?id=<user_id>`:
    *   Publishes a message from a client to all *other* connected clients.
    *   Query Parameters:
        *   `id` (required): The unique string identifier of the sending client. Cannot be empty.
    *   Request Body: The message content.
        *   If the `Content-Type` header is `application/json` or starts with `text/` (e.g., `text/plain`), the message is treated as a UTF-8 text message.
        *   Otherwise, the message is treated as binary.
    *   Responses:
        *   `200 OK`: If the message was successfully sent to the target user's channel.
        *   `400 Bad Request`: If the `id` query parameter is missing or empty, or if a `text/*` body contains invalid UTF-8.
        *   `404 Not Found`: If the specified `user_id` is not currently connected.
