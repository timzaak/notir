# notir

`notir` is a lightweight WebSocket server built with Rust using the Salvo web framework and Tokio. It allows users to connect via WebSockets, subscribe to a real-time message feed, and publish messages to other connected clients.

## Features

- WebSocket communication for real-time messaging.
- Simple publish/subscribe model.
- Serves static files (e.g., a frontend application).
- Containerized with Docker for easy deployment.

## Getting Started

### Using Docker

The easiest way to run `notir` is by using the pre-built Docker image available on GitHub Container Registry.

**Important:** The Docker images are hosted on GitHub Container Registry at `ghcr.io/timzaak/notir`.

1.  **Pull the Docker image:**

    ```bash
    docker pull ghcr.io/timzaak/notir:latest
    ```

    Alternatively, you can pull a specific version (e.g., `v1.0.0`):

    ```bash
    docker pull ghcr.io/timzaak/notir:vX.Y.Z
    ```

2.  **Run the Docker container:**

    ```bash
    docker run -d -p 5800:5800 --name notir-server ghcr.io/timzaak/notir:latest
    ```

    This will start the `notir` server and map port `5800` on your host to port `5800` in the container. The server will be running in detached mode (`-d`). You can view logs with `docker logs notir-server`.

### Building and Running Locally

If you prefer to build and run the project from the source:

1.  **Prerequisites:**
    *   Rust (latest stable version recommended - see `rust-toolchain.toml` if present, otherwise install from [rustup.rs](https://rustup.rs/))
    *   Cargo (Rust's package manager, installed with Rust)

2.  **Clone the repository:**

    ```bash
    git clone https://github.com/timzaak/notir.git
    cd notir
    ```

3.  **Build the project:**

    ```bash
    cargo build --release
    ```

4.  **Run the server:**

    The executable will be located at `target/release/notir`.

    ```bash
    ./target/release/notir
    ```

    The server will start and listen on `0.0.0.0:5800` by default.

## API Endpoints

*   `GET /hello`:
    *   A simple health check endpoint.
    *   Returns: `Hello World` (text/plain).
*   `GET /sub?id=<user_id>`:
    *   Establishes a WebSocket connection for a user to subscribe to messages.
    *   Query Parameters:
        *   `id` (required): A unique string identifier for the client. Cannot be empty.
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
*   **Static Files**:
    *   The server serves static files from the embedded `static/` directory (e.g., `index.html`, CSS, JavaScript).
    *   `index.html` is served as the fallback for any unmatched routes, allowing for single-page application (SPA) frontend hosting.

## How it Works

`notir` maintains an in-memory map of currently connected users (identified by their `user_id`) and their corresponding WebSocket sender channels.

1.  **Connection (`/sub`)**: When a client connects to `/sub?id=<user_id>`, a WebSocket connection is established. The server stores the `user_id` and a sender part of the WebSocket channel. If an `id` is not provided or is empty, the connection is rejected.
2.  **Publishing (`/pub`)**: When a client sends a `POST` request to `/pub?id=<sender_id>` with a message in the body:
    *   The server looks up the `sender_id` to ensure they are connected. If not, a `404 Not Found` is returned.
    *   If the sender is found, the server iterates through all *other* registered users.
    *   For each other user, the message (from the POST body) is sent through their respective WebSocket sender channel.
    *   The type of WebSocket message (text or binary) is determined by the `Content-Type` header of the POST request.
3.  **Disconnection**: If a WebSocket connection is closed (either by the client or due to an error), the server removes the corresponding `user_id` from its list of active users. If a message fails to send to a user because their channel is disconnected, they are also removed.

## Development

### Dependencies

*   `salvo`: Web framework.
*   `tokio`: Asynchronous runtime.
*   `rust-embed`: To embed static files into the binary.
*   `futures-util`, `tokio-stream`: For stream manipulation with WebSockets.
*   `serde`, `serde_json`: For serialization/deserialization (though JSON is handled as text here).
*   `tracing`, `tracing-subscriber`: For logging.

### Environment Variables for Logging

Logging behavior can be controlled via the `RUST_LOG` environment variable. For example:

```bash
RUST_LOG=info,notir=debug ./target/release/notir
```

This sets the default log level to `info` and the log level for the `notir` crate specifically to `debug`.

## CI/CD

A GitHub Actions workflow (`.github/workflows/cd.yml`) is set up to:

1.  Trigger on pushes to tags matching `v*.*.*` (e.g., `v1.0.0`, `v0.2.1`).
2.  Build the Rust application in a Docker container.
3.  Create a final, small Docker image using `gcr.io/distroless/cc-debian12`.
4.  Push the Docker image to GitHub Container Registry (`ghcr.io`) with two tags:
    *   `ghcr.io/timzaak/notir:latest`
    *   `ghcr.io/timzaak/notir:vX.Y.Z` (where `vX.Y.Z` is the git tag)

## License

This project is licensed under the [MIT License](LICENSE). See the `LICENSE` file for details.
