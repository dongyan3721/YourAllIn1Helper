#!/usr/bin/env python3
"""准备字节一致的 GUI 测试 App 副本，用独立目录隔离日常数据；不启动应用。"""
from __future__ import annotations

import datetime
import hashlib
import json
import pathlib
import platform
import plistlib
import shutil
import subprocess
from zoneinfo import ZoneInfo


def prepare() -> None:
    root = pathlib.Path(__file__).resolve().parent.parent
    now = datetime.datetime.now(ZoneInfo("Asia/Shanghai"))
    date = now.strftime("%Y-%m-%d")
    source = root / "dist/All-in-One Toolbox.app"
    destination = root / ".runtime_data/gui-qa/All-in-One Toolbox QA.app"
    data_dir = root / f".runtime_data/gui-qa-{now:%Y%m%d}"
    if not source.is_dir():
        raise SystemExit("请先执行 scripts/package_macos_app.sh 生成 App")
    data_dir.mkdir(parents=True, exist_ok=True)
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(source, destination, dirs_exist_ok=True)
    info_path = destination / "Contents/Info.plist"
    info = plistlib.loads(info_path.read_bytes())
    # 只改启动身份与测试环境变量；可执行文件和资源保持原始 release 内容。
    info.update(
        CFBundleIdentifier="com.allinone.desktop.guiqa",
        CFBundleDisplayName="All-in-One Toolbox QA",
        CFBundleName="All-in-One Toolbox QA",
        LSEnvironment={"ALL_IN_ONE_GUI_TEST_DATA_DIR": str(data_dir)},
    )
    info_path.write_bytes(plistlib.dumps(info))
    executable = pathlib.Path("Contents/MacOS/all-in-one-desktop")
    digest = hashlib.sha256((source / executable).read_bytes()).hexdigest()
    if hashlib.sha256((destination / executable).read_bytes()).hexdigest() != digest:
        raise SystemExit("测试副本与交付包可执行文件不一致")
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
    dirty = bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=root, text=True).strip())
    manifest_path = root / f"docs/testing/build-manifest-{date}.json"
    manifest_path.parent.mkdir(parents=True, exist_ok=True)
    manifest = json.loads(manifest_path.read_text()) if manifest_path.exists() else {}
    if manifest.get("executable_sha256") != digest and "executable_sha256" in manifest:
        manifest.setdefault("previous_build_sha256", []).append(manifest["executable_sha256"])
    manifest.update(
        prepared_at=now.isoformat(), app=str(source), qa_app=str(destination),
        data_dir=str(data_dir), executable_sha256=digest, source_head=head,
        source_dirty=dirty, os=f"macOS {platform.mac_ver()[0]}, {platform.machine()}",
        signing="local unsigned build (--no-sign)", gui_cases=20,
    )
    manifest_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
    print(f"GUI 测试 App：{destination}\n隔离数据：{data_dir}\nSHA-256：{digest}")


if __name__ == "__main__":
    prepare()
