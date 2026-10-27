"""Copy an explicitly supplied candidate and declared assets into a NEW v2 portable directory.

No build, installer/global setting change, cloud secret, or implicit C57 selection.
"""
import argparse
from datetime import datetime, timezone
import json
from pathlib import Path, PureWindowsPath
import shutil

FORBIDDEN_PARTS = {".git", ".work", ".env", "node_modules", "target", "__pycache__",
                   ".venv", "venv", "webview2", "ebwebview", "credentials", "secrets"}
FORBIDDEN_SUFFIXES = {".sqlite", ".sqlite3", ".db", ".journal", ".pyc", ".pyo", ".pem", ".key", ".pfx"}
RUNTIME_SCRIPTS = {"virtual_device.py", "journal.py", "protocol.py", "devices.demo.json"}


class PackageError(ValueError):
    pass


def safe_relative(value):
    windows = PureWindowsPath(value)
    path = Path(value)
    if not path.parts or windows.is_absolute() or windows.drive or path.is_absolute() or ".." in windows.parts or ".." in path.parts:
        raise PackageError("resource destination must be a relative path")
    return path


def admissible(relative):
    parts = [part.lower() for part in relative.parts]
    if any(p in FORBIDDEN_PARTS or p.startswith(".env.") for p in parts):
        raise PackageError("private/cache path is not a deliverable")
    name = relative.name.lower()
    if relative.suffix.lower() in FORBIDDEN_SUFFIXES or name.endswith(("-journal", "-wal", "-shm", ".writer.lock")):
        raise PackageError("journal, secret or cache file is not a deliverable")
    if name in {"token.json", "tokens.json", "credentials.json", "config.toml"}:
        raise PackageError("private configuration is not a deliverable")


def collect_tree(source, source_boundary, destination, role):
    source = Path(source)
    if source.is_symlink() or source.is_junction() or not source.resolve().is_relative_to(source_boundary):
        raise PackageError("resource escapes its declared source root")
    if source.is_file():
        members = [source]
    elif source.is_dir():
        # Do not silently drop an unexpected private file from a declared closure.
        members = []
        for member in source.rglob("*"):
            if member.is_symlink() or member.is_junction() or not member.resolve().is_relative_to(source_boundary):
                raise PackageError("linked resource is outside the trusted closure")
            if member.is_file():
                members.append(member)
    else:
        raise PackageError("declared resource is missing")
    if not members:
        raise PackageError("declared resource is empty")
    result = []
    for member in sorted(members):
        relative_source = member.relative_to(source_boundary)
        admissible(relative_source)
        admissible(member.resolve().relative_to(source_boundary))
        target = destination / member.relative_to(source) if source.is_dir() else destination
        admissible(target)
        result.append((member, target, role))
    return result


def assemble(source_root, candidate_exe, device_resources, output, candidate_id, fixture=False):
    root = Path(source_root).resolve(strict=True)
    candidate = Path(candidate_exe).resolve(strict=True)
    runtime = Path(device_resources).resolve(strict=True)
    output = Path(output).resolve()
    if output.exists():
        raise PackageError("portable output already exists; refusing to overwrite")
    if not candidate.is_file() or candidate.suffix.lower() != ".exe":
        raise PackageError("explicit candidate executable is required")
    with candidate.open("rb") as stream:
        magic = stream.read(2)
    if magic != b"MZ":
        raise PackageError("candidate is not a Windows executable")
    if not candidate_id or any(c in candidate_id for c in "\r\n\0"):
        raise PackageError("candidate id is required")
    if output.is_relative_to(runtime) or runtime.is_relative_to(output):
        raise PackageError("portable output overlaps runtime source")
    config = root / "apps/planner/src-tauri/tauri.conf.json"
    mappings = json.loads(config.read_text(encoding="utf-8-sig"))["bundle"]["resources"]
    if not isinstance(mappings, dict):
        raise PackageError("expected explicit Tauri resource mapping")
    planned = [(candidate, Path("superfactory.exe"), "explicit_candidate_exe")]
    for name in ["superfactory-mcp-bridge.exe", "WebView2Loader.dll"]:
        sibling = candidate.parent / name
        if sibling.is_file():
            planned.append((sibling, Path(name), "candidate_sidecar"))
    for source, destination in mappings.items():
        # Device runtime is consumed exactly once from the separately fixed installation tree.
        dest = safe_relative(destination)
        if dest.parts and dest.parts[0].lower() == "v2-devices":
            continue
        declared = config.parent / source
        if not declared.resolve().is_relative_to(root):
            raise PackageError("declared resource source escapes project root")
        planned.extend(collect_tree(declared, root, dest, "declared_native_resource"))
    for name in ["runtime.json", "python/python.exe", "python/python314.dll", "python/python314.zip",
                 "python/python314._pth", "licenses/PYTHON-LICENSE.txt"]:
        if not (runtime / name).is_file():
            raise PackageError("embedded device runtime is incomplete")
    pth = (runtime / "python/python314._pth").read_text(encoding="ascii").splitlines()
    if pth != ["python314.zip", ".", "../scripts"]:
        raise PackageError("embedded path configuration is not isolated")
    if {p.name for p in (runtime / "scripts").iterdir()} != RUNTIME_SCRIPTS:
        raise PackageError("unexpected device script or cache in trusted runtime")
    planned.extend(collect_tree(runtime, runtime, Path("v2-devices"), "trusted_embedded_device_runtime"))
    for name in ["NOTICE", "NOTICE.md", "LICENSE", "LICENSE.md"]:
        p = root / name
        if p.is_file():
            planned.append((p, Path(name), "project_notice"))
    destinations = set()
    for _, destination, _ in planned:
        safe_relative(str(destination))
        normalized = str(destination).replace("\\", "/").lower()
        if normalized in destinations:
            raise PackageError("duplicate portable destination")
        destinations.add(normalized)
    # All checks occur before creating the new delivery directory.
    output.mkdir(parents=True, exist_ok=False)
    copied = []
    for source, destination, role in planned:
        target = output / destination
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
        copied.append({"path": destination.as_posix(), "bytes": target.stat().st_size, "role": role})
    result = {"candidate_id": candidate_id, "fixture_only": fixture,
              "assembled_at_utc": datetime.now(timezone.utc).isoformat(),
              "candidate_exe_explicitly_supplied": True,
              "candidate_version_asserted_by_caller_not_verified_by_copy": True,
              "actual_v2_gui_https_engine_acceptance_verified": False,
              "tauri_bundle_setting_changed": False, "files": copied,
              "runtime_resource_directory": "v2-devices", "journal_or_credentials_copied": False,
              "note": "Copying is not Native/Engine/cloud acceptance. Keep runtime journal in private appdata."}
    (output / "V2_PORTABLE.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-root", type=Path, required=True)
    parser.add_argument("--candidate-exe", type=Path, required=True)
    parser.add_argument("--device-resources", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--candidate-id", required=True)
    parser.add_argument("--fixture", action="store_true", help="Mark an independent copy-boundary fixture, never a real v2 product")
    args = parser.parse_args()
    try:
        result = assemble(args.source_root, args.candidate_exe, args.device_resources,
                          args.output, args.candidate_id, args.fixture)
    except (PackageError, OSError, ValueError, KeyError) as error:
        parser.exit(2, "portable preparation failed: " + type(error).__name__ + "\n")
    print(json.dumps({"candidate_id": result["candidate_id"], "fixture_only": result["fixture_only"],
                      "files": len(result["files"]), "bytes": sum(f["bytes"] for f in result["files"]),
                      "output": str(args.output)}, ensure_ascii=False))


if __name__ == "__main__":
    main()
