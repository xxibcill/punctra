#!/usr/bin/env python3
"""Serve the private browser demo with strict Range, CORS, and validator headers."""

from __future__ import annotations

import argparse
import hashlib
import ipaddress
import json
import mimetypes
import os
import platform
import subprocess
import time
from functools import lru_cache
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, unquote, urlsplit


WEB_ROOT = (Path(__file__).resolve().parent.parent / "apps/browser-demo/web").resolve()
REPOSITORY_ROOT = Path(__file__).resolve().parent.parent
VISUAL_VERIFIER_REPOSITORY_PATH = "scripts/verify-browser-visual-baseline.mjs"
VISUAL_VERIFIER_PATH = REPOSITORY_ROOT / VISUAL_VERIFIER_REPOSITORY_PATH
VISUAL_BASELINE_PATH = REPOSITORY_ROOT / "docs/releases/v0.21-browser-visual-baseline.json"
FOOTPRINT_VERIFIER_REPOSITORY_PATH = "scripts/verify-browser-point-footprint.mjs"
FOOTPRINT_VERIFIER_PATH = REPOSITORY_ROOT / FOOTPRINT_VERIFIER_REPOSITORY_PATH
FOOTPRINT_BASELINE_PATH = (
    REPOSITORY_ROOT / "docs/releases/v0.22-browser-point-footprint-baseline.json"
)
FOOTPRINT_CORPUS_REPOSITORY_PATH = (
    "apps/browser-demo/web/fixtures/footprint-v1/corpus.json"
)
FOOTPRINT_RUNTIME_REPOSITORY_PATHS = (
    "apps/browser-demo/web/package.json",
    "apps/browser-demo/web/pkg/browser_demo.js",
    "apps/browser-demo/web/pkg/browser_demo_bg.wasm",
)
FOOTPRINT_IMPLEMENTATION_REPOSITORY_PATHS = (
    "Cargo.lock",
    "Cargo.toml",
    "apps/browser-demo/Cargo.toml",
    "apps/browser-demo/src/browser.rs",
    "apps/browser-demo/src/capture.rs",
    "apps/browser-demo/src/diagnostics.rs",
    "apps/browser-demo/src/display.rs",
    "apps/browser-demo/src/host.rs",
    "apps/browser-demo/src/lib.rs",
    "apps/browser-demo/src/scene.rs",
    "apps/browser-demo/src/streaming.rs",
    "apps/browser-demo/web/footprint-artifacts.js",
    "apps/browser-demo/web/footprint-artifacts.test.mjs",
    "apps/browser-demo/web/footprint-corpus.js",
    "apps/browser-demo/web/footprint-corpus.test.mjs",
    "apps/browser-demo/web/footprint-evidence.js",
    "apps/browser-demo/web/footprint-evidence.test.mjs",
    "apps/browser-demo/web/footprint-export.js",
    "apps/browser-demo/web/footprint-export.test.mjs",
    "apps/browser-demo/web/footprint-main.js",
    "apps/browser-demo/web/footprint-qualification.js",
    "apps/browser-demo/web/footprint-records.js",
    "apps/browser-demo/web/footprint-records.test.mjs",
    "apps/browser-demo/web/footprint-runner-core.js",
    "apps/browser-demo/web/footprint-runner-core.test.mjs",
    "apps/browser-demo/web/footprint.css",
    "apps/browser-demo/web/footprint.html",
    "apps/browser-demo/web/visual-archive.js",
    "apps/browser-demo/web/visual-capture.js",
    "apps/browser-demo/web/visual-comparison.js",
    "apps/browser-demo/web/visual-corpus.js",
    "apps/browser-demo/web/visual-corpus.test.mjs",
    "apps/browser-demo/web/visual-footprint-metrics.js",
    "apps/browser-demo/web/visual-footprint-metrics.test.mjs",
    "apps/browser-demo/web/visual-png.js",
    "apps/browser-demo/web/visual-provenance.js",
    "apps/browser-demo/web/visual-rubric.js",
    "apps/browser-demo/web/visual-validation.js",
    FOOTPRINT_CORPUS_REPOSITORY_PATH,
    "apps/renderer-demo/src/appearance.rs",
    "crates/render-wgpu/Cargo.toml",
    "crates/render-wgpu/src/footprint.rs",
    "crates/render-wgpu/src/frame.rs",
    "crates/render-wgpu/src/gpu.rs",
    "crates/render-wgpu/src/eye_dome.wgsl",
    "crates/render-wgpu/src/lib.rs",
    "crates/render-wgpu/src/pick.rs",
    "crates/render-wgpu/src/pipeline.rs",
    "crates/render-wgpu/src/point.wgsl",
    "crates/render-wgpu/src/renderer.rs",
    "crates/render-wgpu/src/targets.rs",
    "crates/render-wgpu/tests/contracts.rs",
    "crates/render-wgpu/tests/offscreen.rs",
    "crates/render-wgpu/test-support/gpu.rs",
    "scripts/build-browser-demo.sh",
    "scripts/serve-browser-demo.py",
    FOOTPRINT_VERIFIER_REPOSITORY_PATH,
)
VIEWER_PACKAGE = json.loads((WEB_ROOT / "package.json").read_text(encoding="utf-8"))
EXPOSED_HEADERS = "Accept-Ranges, Content-Encoding, Content-Length, Content-Range, ETag"
FILE_CHUNK_BYTES = 64 * 1024
FAULTS = {"disconnect", "redirect", "retry", "truncated", "corrupt", "validator_drift"}
QUALIFICATION_HOST_SCHEMA = "punctra-qualification-host-v1"
VISUAL_EXPORT_PATH = "/qualification-visual-export"
VISUAL_EXPORT_FILENAME = "v0.21-browser-visual-evidence.tar"
VISUAL_EXPORT_RECEIPT_SCHEMA = "punctra-browser-visual-export-receipt-v1"
FOOTPRINT_EXPORT_PATH = "/qualification-footprint-export"
FOOTPRINT_EXPORT_FILENAME = "v0.22-browser-point-footprint-evidence.tar"
FOOTPRINT_EXPORT_RECEIPT_SCHEMA = (
    "punctra-browser-point-footprint-export-receipt-v1"
)
VISUAL_VERIFY_PINS_SCHEMA = "punctra-browser-visual-verify-pins-v1"
FOOTPRINT_VERIFY_PINS_SCHEMA = "punctra-browser-point-footprint-verify-pins-v1"
MAX_VISUAL_EXPORT_BYTES = 1_243_611_136
MAX_FOOTPRINT_EXPORT_BYTES = 134_217_728


def fixture_validators() -> dict[Path, str]:
    fixture_root = WEB_ROOT / "fixtures" / "v1"
    manifest = json.loads((fixture_root / "deployment.json").read_text(encoding="utf-8"))
    return {
        (fixture_root / "representative.las").resolve(): manifest["source"]["strong_etag"],
        (fixture_root / "representative.pidx").resolve(): manifest["index"]["strong_etag"],
    }


FIXTURE_VALIDATORS = fixture_validators()


@lru_cache(maxsize=1)
def qualification_host_facts() -> dict[str, object]:
    hardware = first_system_profiler_record("SPHardwareDataType")
    display_controller = first_system_profiler_record("SPDisplaysDataType")
    primary_display = next(
        (
            display
            for display in display_controller.get("spdisplays_ndrvs", [])
            if display.get("spdisplays_main") == "spdisplays_yes"
        ),
        {},
    )
    chip = hardware.get("chip_type")
    gpu = display_controller.get("sppci_model") or chip
    return {
        "schema": QUALIFICATION_HOST_SCHEMA,
        "operating_system": {
            "name": "macOS" if platform.system() == "Darwin" else platform.system(),
            "version": command_text("sw_vers", "-productVersion") or platform.release(),
            "build": command_text("sw_vers", "-buildVersion"),
            "architecture": platform.machine(),
        },
        "device": {
            "class": "Apple silicon laptop"
            if chip and "MacBook" in hardware.get("machine_name", "")
            else hardware.get("machine_name"),
            "gpu": gpu,
            "gpu_cores": integer_text(display_controller.get("sppci_cores")),
            "gpu_class": "integrated" if display_controller.get("sppci_bus") == "spdisplays_builtin" else None,
            "metal_support": metal_support(display_controller.get("spdisplays_mtlgpufamilysupport")),
        },
        "display_path": display_path(primary_display),
        "package": {
            "name": VIEWER_PACKAGE["name"],
            "version": VIEWER_PACKAGE["version"],
        },
    }


@lru_cache(maxsize=1)
def visual_verify_pins() -> dict[str, object]:
    implementation_commit = command_text(
        "git",
        "-C",
        str(REPOSITORY_ROOT),
        "rev-parse",
        "HEAD",
    )
    if implementation_commit is None or len(implementation_commit) != 40:
        raise RuntimeError("visual implementation commit is unavailable")
    verifier_bytes = VISUAL_VERIFIER_PATH.read_bytes()
    baseline = json.loads(VISUAL_BASELINE_PATH.read_text(encoding="utf-8"))
    accepted = baseline["pins"]
    return {
        "schema": VISUAL_VERIFY_PINS_SCHEMA,
        "accepted": {
            "implementation_commit": accepted["implementation_commit"],
            "verifier": accepted["verifier"],
        },
        "running": {
            "implementation_commit": implementation_commit,
            "verifier": {
                "path": VISUAL_VERIFIER_REPOSITORY_PATH,
                "byte_length": len(verifier_bytes),
                "sha256": hashlib.sha256(verifier_bytes).hexdigest(),
            },
        },
    }


@lru_cache(maxsize=1)
def footprint_verify_pins() -> dict[str, object]:
    implementation_commit = command_text(
        "git",
        "-C",
        str(REPOSITORY_ROOT),
        "rev-parse",
        "HEAD",
    )
    if implementation_commit is None or len(implementation_commit) != 40:
        raise RuntimeError("point-footprint implementation commit is unavailable")
    verifier = repository_digest_record(FOOTPRINT_VERIFIER_REPOSITORY_PATH)
    corpus_path = REPOSITORY_ROOT / FOOTPRINT_CORPUS_REPOSITORY_PATH
    corpus = json.loads(corpus_path.read_text(encoding="utf-8"))
    accepted = None
    if FOOTPRINT_BASELINE_PATH.is_file():
        baseline = json.loads(FOOTPRINT_BASELINE_PATH.read_text(encoding="utf-8"))
        accepted = baseline["pins"]
    return {
        "schema": FOOTPRINT_VERIFY_PINS_SCHEMA,
        "accepted": accepted,
        "running": {
            "implementation": {
                "commit": implementation_commit,
                "files": [
                    repository_digest_record(path)
                    for path in FOOTPRINT_IMPLEMENTATION_REPOSITORY_PATHS
                ],
            },
            "verifier": verifier,
            "runtime": {
                "package_name": VIEWER_PACKAGE["name"],
                "package_version": VIEWER_PACKAGE["version"],
                "artifacts": [
                    repository_digest_record(path)
                    for path in FOOTPRINT_RUNTIME_REPOSITORY_PATHS
                ],
            },
            "corpus": repository_digest_record(FOOTPRINT_CORPUS_REPOSITORY_PATH),
            "predecessor": corpus["predecessor"],
        },
    }


def repository_digest_record(repository_path: str) -> dict[str, object]:
    bytes_ = (REPOSITORY_ROOT / repository_path).read_bytes()
    return {
        "path": repository_path,
        "byte_length": len(bytes_),
        "sha256": hashlib.sha256(bytes_).hexdigest(),
    }


def first_system_profiler_record(data_type: str) -> dict[str, object]:
    try:
        result = subprocess.run(
            ["system_profiler", data_type, "-json"],
            check=False,
            capture_output=True,
            text=True,
            timeout=5,
        )
        payload = json.loads(result.stdout)
        records = payload.get(data_type, [])
        return records[0] if records else {}
    except (OSError, subprocess.SubprocessError, json.JSONDecodeError, IndexError, TypeError):
        return {}


def command_text(*arguments: str) -> str | None:
    try:
        result = subprocess.run(
            list(arguments),
            check=False,
            capture_output=True,
            text=True,
            timeout=5,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    value = result.stdout.strip()
    return value or None


def integer_text(value: object) -> int | None:
    try:
        return int(str(value))
    except (TypeError, ValueError):
        return None


def metal_support(value: object) -> str | None:
    if not isinstance(value, str) or not value.startswith("spdisplays_metal"):
        return None
    return value.removeprefix("spdisplays_").replace("metal", "Metal ").strip()


def display_path(display: dict[str, object]) -> str | None:
    connection = display.get("spdisplays_connection_type")
    display_type = display.get("spdisplays_display_type")
    if connection == "spdisplays_internal" and isinstance(display_type, str):
        if "built-in" in display_type or "liquid-retina" in display_type:
            return "built-in Retina display"
    return None


class BrowserDemoHandler(BaseHTTPRequestHandler):
    """Serve one immutable repository tree without redirects or transformation."""

    server_version = "PunctraBrowserRange/1"

    def handle_one_request(self) -> None:
        try:
            super().handle_one_request()
        except (BrokenPipeError, ConnectionResetError):
            self.close_connection = True

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

    def do_POST(self) -> None:  # noqa: N802
        export_contract = self._export_contract()
        if export_contract is None:
            self.send_error(HTTPStatus.NOT_FOUND)
            return
        _, export_filename, receipt_schema, maximum_bytes, export_directory = (
            export_contract
        )
        if export_directory is None:
            self.send_error(HTTPStatus.NOT_FOUND)
            return
        if not self._is_same_origin_request():
            self.send_error(HTTPStatus.FORBIDDEN)
            return
        if self.headers.get_all("Content-Type", []) != ["application/x-tar"]:
            self.send_error(HTTPStatus.UNSUPPORTED_MEDIA_TYPE)
            return

        content_lengths = self.headers.get_all("Content-Length", [])
        if not content_lengths:
            self.send_error(HTTPStatus.LENGTH_REQUIRED)
            return
        if len(content_lengths) != 1 or not content_lengths[0].isdecimal():
            self.send_error(
                HTTPStatus.BAD_REQUEST,
                "Content-Length must be one decimal integer",
            )
            return
        content_length = int(content_lengths[0])
        if content_length == 0:
            self.send_error(HTTPStatus.BAD_REQUEST, "Content-Length must be greater than zero")
            return
        if content_length > maximum_bytes:
            self.send_error(HTTPStatus.REQUEST_ENTITY_TOO_LARGE)
            return

        export_path = export_directory / export_filename
        staging_path = export_path.with_name(f"{export_path.name}.part")
        if os.path.lexists(export_path):
            self.send_error(HTTPStatus.CONFLICT)
            return

        try:
            digest = self._persist_export(
                staging_path=staging_path,
                export_path=export_path,
                content_length=content_length,
            )
        except FileExistsError:
            self.send_error(HTTPStatus.CONFLICT)
            return
        except EOFError:
            self.send_error(HTTPStatus.BAD_REQUEST, "request body ended before Content-Length")
            return
        except OSError:
            self.send_error(HTTPStatus.INTERNAL_SERVER_ERROR)
            return

        body = json.dumps(
            {
                "schema": receipt_schema,
                "filename": export_filename,
                "path": str(export_path),
                "byte_length": content_length,
                "sha256": digest,
            },
            separators=(",", ":"),
        ).encode("utf-8")
        self.send_response(HTTPStatus.CREATED)
        self.send_header("Cache-Control", "no-store")
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _export_contract(self) -> tuple[str, str, str, int, Path | None] | None:
        request_path = urlsplit(self.path).path
        contracts = {
            VISUAL_EXPORT_PATH: (
                VISUAL_EXPORT_PATH,
                VISUAL_EXPORT_FILENAME,
                VISUAL_EXPORT_RECEIPT_SCHEMA,
                MAX_VISUAL_EXPORT_BYTES,
                self.server.visual_export_dir,
            ),
            FOOTPRINT_EXPORT_PATH: (
                FOOTPRINT_EXPORT_PATH,
                FOOTPRINT_EXPORT_FILENAME,
                FOOTPRINT_EXPORT_RECEIPT_SCHEMA,
                MAX_FOOTPRINT_EXPORT_BYTES,
                self.server.footprint_export_dir,
            ),
        }
        return contracts.get(request_path)

    def _is_same_origin_request(self) -> bool:
        hosts = self.headers.get_all("Host", [])
        origins = self.headers.get_all("Origin", [])
        if len(hosts) != 1:
            return False
        allowed_hosts = {"127.0.0.1", "localhost", "::1"}
        bound_host, bound_port = self.server.server_address[:2]
        try:
            if ipaddress.ip_address(bound_host).is_loopback:
                allowed_hosts.add(bound_host)
        except ValueError:
            pass
        allowed_authorities = {
            f"[{host}]:{bound_port}" if ":" in host else f"{host}:{bound_port}"
            for host in allowed_hosts
        }
        return hosts[0] in allowed_authorities and origins == [f"http://{hosts[0]}"]

    def _persist_export(
        self,
        *,
        staging_path: Path,
        export_path: Path,
        content_length: int,
    ) -> str:
        digest = hashlib.sha256()
        owns_staging_path = False
        published = False
        try:
            with staging_path.open("xb") as destination:
                owns_staging_path = True
                remaining = content_length
                while remaining:
                    chunk = self.rfile.read(min(remaining, FILE_CHUNK_BYTES))
                    if not chunk:
                        raise EOFError
                    destination.write(chunk)
                    digest.update(chunk)
                    remaining -= len(chunk)
                destination.flush()
                os.fsync(destination.fileno())
                if destination.tell() != content_length:
                    raise OSError("visual export length drift")

            os.link(staging_path, export_path)
            published = True
            staging_path.unlink()
            owns_staging_path = False
            if export_path.stat().st_size != content_length:
                raise OSError("published visual export length drift")
            return digest.hexdigest()
        finally:
            if owns_staging_path:
                staging_path.unlink(missing_ok=True)
            if published and not export_path.exists():
                raise OSError("visual export publication disappeared")

    def _serve(self, *, send_body: bool) -> None:
        if urlsplit(self.path).path == "/qualification-host.json":
            self._serve_qualification_host(send_body=send_body)
            return
        if urlsplit(self.path).path == "/qualification-visual-pins.json":
            self._serve_json(visual_verify_pins(), send_body=send_body)
            return
        if urlsplit(self.path).path == "/qualification-footprint-pins.json":
            self._serve_json(footprint_verify_pins(), send_body=send_body)
            return
        if urlsplit(self.path).path == "/qualification-footprint-baseline.json":
            if not FOOTPRINT_BASELINE_PATH.is_file():
                self.send_error(HTTPStatus.NOT_FOUND)
                return
            self._serve_repository_json_file(FOOTPRINT_BASELINE_PATH, send_body=send_body)
            return
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
        self.send_header("Cache-Control", cache_control(path, self.server.web_root))
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

    def _serve_qualification_host(self, *, send_body: bool) -> None:
        self._serve_json(
            qualification_host_facts(),
            send_body=send_body,
            allow_cross_origin=True,
        )

    def _serve_json(
        self,
        payload: dict[str, object],
        *,
        send_body: bool,
        allow_cross_origin: bool = False,
    ) -> None:
        body = json.dumps(
            payload,
            separators=(",", ":"),
            sort_keys=True,
        ).encode("utf-8")
        self.send_response(HTTPStatus.OK)
        if allow_cross_origin:
            self._send_cors_headers()
        self.send_header("Cache-Control", "no-store")
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if send_body:
            self.wfile.write(body)

    def _serve_repository_json_file(self, path: Path, *, send_body: bool) -> None:
        body = path.read_bytes()
        json.loads(body)
        self.send_response(HTTPStatus.OK)
        self.send_header("Cache-Control", "no-store")
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if send_body:
            self.wfile.write(body)

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
        web_root = self.server.web_root
        candidate = (web_root / relative).resolve()
        if candidate != web_root and web_root not in candidate.parents:
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


def cache_control(path: Path, web_root: Path = WEB_ROOT) -> str:
    fixture_root = web_root / "fixtures" / "v1"
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
    parser.add_argument("--root", default=WEB_ROOT, type=Path)
    parser.add_argument("--visual-export-dir", type=Path)
    parser.add_argument("--footprint-export-dir", type=Path)
    return parser.parse_args()


class BrowserDemoServer(ThreadingHTTPServer):
    def __init__(
        self,
        address: tuple[str, int],
        web_root: Path,
        visual_export_dir: Path | None = None,
        footprint_export_dir: Path | None = None,
    ) -> None:
        resolved_root = web_root.resolve()
        if not resolved_root.is_dir():
            raise ValueError(f"browser root is not a directory: {resolved_root}")
        self.web_root = resolved_root
        self.visual_export_dir = validated_export_directory(
            visual_export_dir,
            "visual",
        )
        self.footprint_export_dir = validated_export_directory(
            footprint_export_dir,
            "point-footprint",
        )
        super().__init__(address, BrowserDemoHandler)


def validated_export_directory(directory: Path | None, label: str) -> Path | None:
    if directory is None:
        return None
    resolved = directory.resolve()
    if not resolved.is_dir():
        raise ValueError(f"{label} export directory is not a directory: {resolved}")
    return resolved


def main() -> None:
    options = arguments()
    server = BrowserDemoServer(
        (options.host, options.port),
        options.root,
        options.visual_export_dir,
        options.footprint_export_dir,
    )
    host, port = server.server_address[:2]
    print(f"Serving {server.web_root} at http://{host}:{port}/", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
