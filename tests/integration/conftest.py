"""Pytest fixtures for OpenRA browser integration tests."""
import pytest
import subprocess
import threading
import socket
import os
from http.server import HTTPServer, SimpleHTTPRequestHandler


def _find_free_port():
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("", 0))
        return s.getsockname()[1]


class WasmHandler(SimpleHTTPRequestHandler):
    """HTTP handler with correct MIME type for .wasm files."""

    extensions_map = {
        **SimpleHTTPRequestHandler.extensions_map,
        ".wasm": "application/wasm",
        ".js": "application/javascript",
    }

    def log_message(self, format, *args):
        pass  # Suppress request logging during tests


@pytest.fixture(scope="session")
def server():
    """Start HTTP server for the game on a free port."""
    port = _find_free_port()
    www_dir = os.path.join(os.getcwd(), "openra-wasm", "www")
    os.chdir(www_dir)

    httpd = HTTPServer(("", port), WasmHandler)
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    yield f"http://localhost:{port}"
    httpd.shutdown()


@pytest.fixture
def game_page(page, server):
    """Navigate to the game and return the Playwright page."""
    page.set_viewport_size({"width": 1280, "height": 720})
    page.goto(server, wait_until="networkidle")
    # Wait for WASM to initialize (map selector no longer shows "Initializing")
    page.wait_for_function(
        "!document.querySelector('#map-select option').textContent.includes('Initializing')",
        timeout=60000,
    )
    yield page
