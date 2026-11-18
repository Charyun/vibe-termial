#!/usr/bin/env python3
"""Build and zip a Windows x64 portable release. Uses only Python's stdlib.

Usage: python scripts/package-windows-portable.py [--single-exe] [--skip-build]
Single-exe mode links the Visual C++ CRT statically and needs no launcher or DLLs.
The ordinary portable launcher stores state beside the program. Neither mode
bundles an installer or personal data.
"""
from __future__ import annotations
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parent.parent


def command(*args: str) -> str:
    return subprocess.check_output(args, cwd=ROOT, text=True, encoding="utf-8").strip()


def pe_imports(path: Path) -> list[str]:
    """Read PE32+ imports without depending on Visual Studio or third-party Python packages."""
    data = path.read_bytes()
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if data[:2] != b"MZ" or data[pe:pe + 4] != b"PE\0\0":
        raise RuntimeError(f"Not a PE executable: {path}")
    machine, sections = struct.unpack_from("<HH", data, pe + 4)
    optional_size = struct.unpack_from("<H", data, pe + 20)[0]
    optional = pe + 24
    if machine != 0x8664 or struct.unpack_from("<H", data, optional)[0] != 0x20B:
        raise RuntimeError(f"Expected a Windows x64 PE32+ binary: {path}")
    headers = optional + optional_size

    def raw(rva: int) -> int:
        for index in range(sections):
            section = headers + index * 40
            virtual_size, virtual_address, raw_size, raw_address = struct.unpack_from("<IIII", data, section + 8)
            if virtual_address <= rva < virtual_address + max(virtual_size, raw_size):
                return raw_address + rva - virtual_address
        raise RuntimeError(f"Unmapped RVA {rva:x} in {path}")

    import_rva, import_size = struct.unpack_from("<II", data, optional + 112 + 8)
    if not import_rva:
        return []
    result = []
    offset = raw(import_rva)
    for index in range(import_size // 20):
        descriptor = struct.unpack_from("<IIIII", data, offset + index * 20)
        if not any(descriptor):
            break
        start = raw(descriptor[3])
        result.append(data[start:data.index(b"\0", start)].decode("ascii"))
    return result


def find_crt(explicit: str | None) -> Path:
    if explicit:
        directory = Path(explicit).resolve()
        if not (directory / "vcruntime140.dll").is_file():
            raise RuntimeError(f"Not an x64 Visual C++ redistributable directory: {directory}")
        return directory
    base = Path(os.environ.get("ProgramFiles", "C:/Program Files")) / "Microsoft Visual Studio"
    candidates = list(base.glob("*/*/VC/Redist/MSVC/[0-9]*/x64/Microsoft.VC*.CRT"))
    candidates = [p for p in candidates if (p / "vcruntime140.dll").is_file()]
    if not candidates:
        raise RuntimeError("Cannot find the x64 VC redistributable files. Supply --crt-dir.")
    return max(candidates, key=lambda p: tuple(int(n) for n in re.findall(r"\d+", p.parts[-3])))


def copy_runtime(executable: Path, destination: Path, crt: Path) -> tuple[list[str], list[str]]:
    visited: set[str] = set()
    bundled: list[str] = []
    system: set[str] = set()
    pending = [executable]
    system32 = Path(os.environ.get("SystemRoot", "C:/Windows")) / "System32"
    while pending:
        binary = pending.pop()
        for name in pe_imports(binary):
            key = name.lower()
            if key in visited:
                continue
            visited.add(key)
            if key.startswith(("api-ms-", "ext-ms-")):
                system.add(name)
                continue
            source = next((p for p in (crt / name, executable.parent / name) if p.is_file()), None)
            if source:
                target = destination / name.lower()
                shutil.copy2(source, target)
                bundled.append(target.name)
                pending.append(target)
            elif (system32 / name).is_file():
                system.add(name)
            else:
                raise RuntimeError(f"Unresolved runtime dependency {name} imported by {binary}")
    return sorted(bundled), sorted(system, key=str.lower)


def notices(metadata: dict, destination: Path) -> None:
    # Include exact upstream license/notice files from the local locked dependency
    # graph. Build/dev packages may also appear; including their notices is harmless.
    output = ["Third-party component notices\n", "Generated from the locked Cargo metadata for x86_64-pc-windows-msvc.\n"]
    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        if not package.get("source"):
            continue
        directory = Path(package["manifest_path"]).parent
        output.extend(["\n" + "=" * 72 + "\n", f"{package['name']} {package['version']}\n", f"License: {package.get('license') or 'See upstream license file'}\n", f"Repository: {package.get('repository') or package.get('homepage') or ''}\n"])
        files = {p for p in directory.iterdir() if p.is_file() and p.name.lower().startswith(("license", "licence", "copying", "notice", "copyright"))}
        if package.get("license_file"):
            files.add(directory / package["license_file"])
        for file in sorted(files):
            if file.is_file() and file.stat().st_size < 1_048_576:
                output.extend([f"\n--- {file.name} ---\n", file.read_text(encoding="utf-8", errors="replace"), "\n"])
    (destination / "THIRD-PARTY-NOTICES.txt").write_text("".join(output), encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--single-exe", action="store_true", help="Statically link the CRT; distribute one runnable exe without runtime DLLs")
    parser.add_argument("--output-dir", default="dist")
    parser.add_argument("--crt-dir")
    args = parser.parse_args()
    if os.name != "nt":
        raise RuntimeError("Build this Windows package on Windows.")
    if not args.skip_build:
        build = ["cargo", "build", "--release", "--locked", "--bin", "vibe-terminal"]
        environment = os.environ.copy()
        if args.single_exe:
            # An explicit target keeps host build tools/proc macros unaffected.
            # Both variants can coexist in the same Cargo target directory.
            build.extend(["--target", "x86_64-pc-windows-msvc"])
            environment["RUSTFLAGS"] = (environment.get("RUSTFLAGS", "") + " -C target-feature=+crt-static").strip()
            if "CARGO_ENCODED_RUSTFLAGS" in environment:
                environment["CARGO_ENCODED_RUSTFLAGS"] += "\x1f-C\x1ftarget-feature=+crt-static"
        subprocess.run(build, cwd=ROOT, env=environment, check=True)
    metadata = json.loads(command("cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", "x86_64-pc-windows-msvc"))
    package = next(p for p in metadata["packages"] if Path(p["manifest_path"]).parent == ROOT)
    binary_root = Path(metadata["target_directory"])
    if args.single_exe:
        binary_root /= "x86_64-pc-windows-msvc"
    executable = binary_root / "release" / "vibe-terminal.exe"
    if not executable.is_file():
        raise RuntimeError(f"Release executable missing: {executable}")
    variant = "single-exe" if args.single_exe else "portable"
    name = f"VibeTerm-{package['version']}-windows-x64-{variant}"
    output = (ROOT / args.output_dir).resolve()
    folder = output / name
    archive = output / (name + ".zip")
    if folder.exists() or archive.exists():
        raise RuntimeError(f"Output already exists. Choose a fresh --output-dir: {output}")
    folder.mkdir(parents=True)
    shutil.copy2(executable, folder / "VibeTerm.exe")
    if args.single_exe:
        system = pe_imports(executable)
        dynamic_crt = [name for name in system if name.lower().startswith(("vcruntime", "msvcp", "concrt"))]
        if dynamic_crt:
            raise RuntimeError(f"Static-CRT build still imports external VC runtime DLLs: {dynamic_crt}")
        bundled = []
    else:
        bundled, system = copy_runtime(executable, folder, find_crt(args.crt_dir))
        launcher = '@echo off\r\nsetlocal\r\nset "VIBE_TERMINAL_DATA_DIR=%~dp0data"\r\nstart "" /D "%~dp0" "%~dp0VibeTerm.exe"\r\n'
        (folder / "StartPortable.cmd").write_bytes(launcher.encode("ascii"))
        (folder / "data").mkdir()
        (folder / "data" / "README.txt").write_text("Portable application state is stored here when using StartPortable.cmd.\nNo personal state is included in this package.\n", encoding="utf-8")
    readme = f"""VibeTerm {package['version']} - Windows x64 portable

快速开始
1. 将整个 ZIP 解压到可写目录，不要直接从压缩包内运行。
2. 双击 StartPortable.cmd 启动，无需安装程序或管理员权限。
3. 工作区配置、终端历史和会话数据写入本目录下的 data 文件夹。
4. 关闭软件后，可将整个文件夹移动或复制到其他位置。

直接运行 VibeTerm.exe 也可以，但会沿用应用默认的系统用户数据目录；
如需配置跟随便携包，请使用 StartPortable.cmd。

运行条件
- Windows 10/11 x64，支持 ConPTY 和 Direct3D 的系统环境。
- 必需的 Visual C++ 运行时 DLL 已随包放置，不需要为本应用运行 VC 安装程序。
- 内嵌网页预览使用系统 WebView2 Runtime；若系统未提供，网页预览可能不可用。
- Shell、Git、SSH 和各 Agent CLI 使用本机已安装的版本，本包不附带账号或模型。

注意
- 不要同时运行多个实例访问同一个 data 目录。
- 删除 data 会移除便携版配置和显示历史，请先备份。
- 本包没有安装器、自动启动项或用户个人数据。

许可与来源见 LICENSES、THIRD-PARTY-NOTICES.txt 和 BUILD-INFO.json。
"""
    if args.single_exe:
        readme = f"""VibeTerm {package['version']} - Windows x64 single-exe Release

直接双击 VibeTerm.exe 即可运行，无需安装器、启动脚本或旁带 DLL。
Visual C++ 运行库已静态链接；字体、图标和远程页面资源已嵌入 exe。
你可以只复制这个 exe 到其他可写目录或电脑上运行。

配置、终端历史和会话文件保存在 Windows 当前用户的数据目录中。
需要自行指定数据位置时，可在启动前设置 VIBE_TERMINAL_DATA_DIR。

运行条件
- Windows 10/11 x64，支持 ConPTY 和 Direct3D。
- 内嵌网页预览使用系统已有的 WebView2 Runtime；缺少时不影响本地终端。
- Shell、Git、SSH 和各 Agent CLI 使用本机已安装的版本。
- 本包不包含个人工作区、历史、账号、模型或安装程序。

许可与构建来源见 LICENSES、THIRD-PARTY-NOTICES.txt 和 BUILD-INFO.json。
"""
    (folder / "README.txt").write_text(readme, encoding="utf-8-sig")
    licenses = folder / "LICENSES"
    licenses.mkdir()
    for source, target in [
        (ROOT / "LICENSE", "VibeTerm-MIT.txt"),
        (ROOT / "assets/fonts/Geist-OFL.txt", "Geist-OFL.txt"),
        (ROOT / "assets/fonts/GeistMono-OFL.txt", "GeistMono-OFL.txt"),
        (ROOT / "assets/agents/LICENSE.orca", "Orca-MIT.txt"),
        (ROOT / "assets/agents/README.md", "Agent-icon-sources.md"),
        (ROOT / "assets/web/vendor/LICENSE.xterm", "xterm-MIT.txt"),
        (ROOT / "vendor/vt100/LICENSE", "vt100-MIT.txt"),
    ]:
        shutil.copy2(source, licenses / target)
    if not args.single_exe:
        (licenses / "Microsoft-runtime.txt").write_text("Microsoft Visual C++ Runtime DLLs are redistributed app-locally from the Visual Studio x64 redistributable directory.\nCopyright Microsoft Corporation. All rights reserved.\nhttps://learn.microsoft.com/cpp/windows/redistributing-visual-cpp-files\n", encoding="utf-8")
    notices(metadata, folder)
    files = [{"path":str(p.relative_to(folder)).replace("\\", "/"), "bytes":p.stat().st_size, "sha256":hashlib.sha256(p.read_bytes()).hexdigest()} for p in sorted(folder.rglob("*")) if p.is_file()]
    info = {"product":"VibeTerm", "version":package["version"], "profile":"release", "target":"x86_64-pc-windows-msvc", "packaged_utc":datetime.now(timezone.utc).isoformat(), "git_commit":command("git", "rev-parse", "HEAD"), "working_tree_dirty":bool(command("git", "status", "--porcelain")), "rustc":command("rustc", "--version"), "bundled_runtime_dlls":bundled, "system_imports":system, "static_crt":args.single_exe, "data_directory":"Windows user data directory" if args.single_exe else "data (via StartPortable.cmd)", "files":files}
    (folder / "BUILD-INFO.json").write_text(json.dumps(info, ensure_ascii=False, indent=2), encoding="utf-8")
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as zip_file:
        for file in sorted(folder.rglob("*")):
            if file.is_file():
                zip_file.write(file, file.relative_to(output))
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    checksum = archive.with_suffix(".zip.sha256")
    checksum.write_text(f"{digest}  {archive.name}\n", encoding="ascii")
    standalone = None
    if args.single_exe:
        standalone = output / f"VibeTerm-{package['version']}-windows-x64.exe"
        if standalone.exists():
            raise RuntimeError(f"Standalone output already exists: {standalone}")
        shutil.copy2(executable, standalone)
        standalone.with_suffix(".exe.sha256").write_text(f"{hashlib.sha256(standalone.read_bytes()).hexdigest()}  {standalone.name}\n", encoding="ascii")
    print(json.dumps({"folder":str(folder), "archive":str(archive), "standalone_exe":str(standalone) if standalone else None, "archive_bytes":archive.stat().st_size, "sha256":digest, "checksum":str(checksum), "bundled_runtime_dlls":bundled}, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
