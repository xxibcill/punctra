#!/usr/bin/env python3
"""Serve the private browser demo with strict Range, CORS, and validator headers."""

from __future__ import annotations

import argparse
import hashlib
import json
import mimetypes
import time
from functools import lru_cache
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, unquote, urlsplit


WEB_ROOT = (Path(__file__).resolve().parent.parent / "apps/browser-demo/web").resolve()
EXPOSED_HEADERS = "Accept-Ranges, Content-Encoding, Content-Length, Content-Range, ETag"
FILE_CHUNK_BYTES = 64 * 1024
FAULTS = {"disconnect", "redirect", "retry", "truncated", "corrupt", "validator_drift"}


def fixture_validators() -> dict[Path, str]:
    fixture_root = WEB_ROOT / "fixtures" / "v1"
    manifest = json.loads((fixture_root / "deployment.json").read_text(encoding="utf-8"))
    return {
        (fixture_root / "representative.las").resolve(): manifest["source"]["strong_etag"],
        (fixture_root / "representative.pidx").resolve(): manifest["index"]["strong_etag"],
    }


FIXTURE_VALIDATORS = fixture_validators()


class BrowserDemoHandler(BaseHTTPRequestHandler):
    """Serve one immutable repository tree without redirects or transformation."""

    server_version = "PunctraBrowserRange/1"

    def do_OPTIONS(self) -> None:  # noqa: N802
        self.send_response(HTTPStatus.NO_CONTENT)
        self._send_cors_headers()
        self.send_header("Access-Control-Allow-Headers", "Range")
        self.send_header("Access-Control-Allow-Methods", "GET, HEAD, OPTIONS")
        self.send_header("Content-Length", "0")
        self.end_headers()

    def do_HEAD(self) -> None:  # noqa: N802
        self._serve(send_body=False)

    def do_GET(self) -> None:  # noqa: N802
        self._serve(send_body=True)

    def _serve(self, *, send_body: bool) -> None:
        try:
            delay_milliseconds = self._delay_milliseconds()
            fault = self._fault()
            path = self._resolve_path()
            file_stat = path.stat()
        except FileNotFoundError:
            self.send_error(HTTPStatus.NOT_FOUND)
            return
        except ValueError as error:
            self.send_error(HTTPStatus.BAD_REQUEST, str(error))
            return

        if fault == "redirect":
            self.send_response(HTTPStatus.FOUND)
            self._send_cors_headers()
            self.send_header("Location", "/fixtures/v1/representative.las")
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        if fault == "disconnect":
            self.close_connection = True
            return
        if fault == "retry":
            self.send_response(HTTPStatus.SERVICE_UNAVAILABLE)
            self._send_cors_headers()
            self.send_header("Content-Length", "0")
            self.end_headers()
            return

        try:
            start, end, partial = self._requested_range(file_stat.st_size)
        except ValueError as error:
            self.send_error(HTTPStatus.REQUESTED_RANGE_NOT_SATISFIABLE, str(error))
            return

        if delay_milliseconds:
            time.sleep(delay_milliseconds / 1_000)

        body_length = end - start + 1
        if fault == "truncated":
            body_length -= 1
        self.send_response(HTTPStatus.PARTIAL_CONTENT if partial else HTTPStatus.OK)
        self._send_cors_headers()
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Cache-Control", cache_control(path))
        self.send_header("Content-Encoding", "identity")
        self.send_header("Content-Length", str(body_length))
        self.send_header("Content-Type", content_type(path))
        self.send_header(
            "ETag",
            '"changed"'
            if fault == "validator_drift"
            else representation_etag(path, file_stat.st_size, file_stat.st_mtime_ns),
        )
        if partial:
            self.send_header("Content-Range", f"bytes {start}-{end}/{file_stat.st_size}")
        self.end_headers()
        if send_body:
            self._write_body(path, start, body_length, corrupt=fault == "corrupt")

    def _write_body(self, path: Path, start: int, length: int, *, corrupt: bool = False) -> None:
        remaining = length
        first_chunk = True
        with path.open("rb") as source:
            source.seek(start)
            while remaining:
                chunk = bytearray(source.read(min(remaining, FILE_CHUNK_BYTES)))
                if not chunk:
                    return
                if corrupt and first_chunk:
                    chunk[0] ^= 0xFF
                    first_chunk = False
                self.wfile.write(chunk)
                remaining -= len(chunk)

    def _resolve_path(self) -> Path:
        raw_path = unquote(urlsplit(self.path).path)
        relative = raw_path.removeprefix("/") or "index.html"
        candidate = (WEB_ROOT / relative).resolve()
        if candidate != WEB_ROOT and WEB_ROOT not in candidate.parents:
            raise FileNotFoundError(relative)
        if not candidate.is_file():
            raise FileNotFoundError(relative)
        return candidate

    def _requested_range(self, length: int) -> tuple[int, int, bool]:
        value = self.headers.get("Range")
        if value is None:
            return 0, length - 1, False
        if not value.startswith("bytes=") or "," in value:
            raise ValueError("only one explicit byte range is supported")
        fields = value.removeprefix("bytes=").split("-", maxsplit=1)
        if len(fields) != 2 or not all(field.isdecimal() for field in fields):
            raise ValueError("range must contain explicit decimal start and end offsets")
        start, end = map(int, fields)
        if start > end or end >= length:
            raise ValueError("range is outside the immutable representation")
        return start, end, True

    def _delay_milliseconds(self) -> int:
        values = parse_qs(urlsplit(self.path).query).get("delay_ms", [])
        if not values:
            return 0
        if len(values) != 1 or not values[0].isdecimal():
            raise ValueError("delay_ms must be one decimal integer")
        delay = int(values[0])
        if delay > 1_000:
            raise ValueError("delay_ms exceeds the 1,000 millisecond fault ceiling")
        return delay

    def _fault(self) -> str | None:
        values = parse_qs(urlsplit(self.path).query).get("fault", [])
        if not values:
            return None
        if len(values) != 1 or values[0] not in FAULTS:
            raise ValueError("fault must be one supported bounded fault route")
        return values[0]

    def _send_cors_headers(self) -> None:
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Expose-Headers", EXPOSED_HEADERS)
        self.send_header("Cross-Origin-Resource-Policy", "cross-origin")


def content_type(path: Path) -> str:
    if path.suffix == ".wasm":
        return "application/wasm"
    if path.suffix == ".las":
        return "application/vnd.las"
    if path.suffix == ".pidx":
        return "application/octet-stream"
    return mimetypes.guess_type(path.name)[0] or "application/octet-stream"


def cache_control(path: Path) -> str:
    fixture_root = WEB_ROOT / "fixtures" / "v1"
    if path == fixture_root or fixture_root in path.parents:
        return "public, max-age=31536000, immutable, no-transform"
    return "no-store, no-transform"


def representation_etag(path: Path, length: int, modified_nanoseconds: int) -> str:
    fixture_validator = FIXTURE_VALIDATORS.get(path)
    if fixture_validator is not None:
        return fixture_validator
    return streamed_etag(path, length, modified_nanoseconds)


@lru_cache(maxsize=128)
def streamed_etag(path: Path, _length: int, _modified_nanoseconds: int) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(FILE_CHUNK_BYTES):
            digest.update(chunk)
    return f'"sha256-{digest.hexdigest()}"'


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", default=8000, type=int)
    return parser.parse_args()


def main() -> None:
    options = arguments()
    server = ThreadingHTTPServer((options.host, options.port), BrowserDemoHandler)
    host, port = server.server_address[:2]
    print(f"Serving {WEB_ROOT} at http://{host}:{port}/", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
