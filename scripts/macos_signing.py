#!/usr/bin/env python3
"""固定 macOS 开发签名：证书私钥留在钥匙串，仓库仅保存工具和公开配置。"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import secrets
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
CONFIG = ROOT / ".runtime_data/macos-signing/identity.json"


def run(*args: str, **kwargs) -> subprocess.CompletedProcess:
    """命令参数使用数组；不打印口令、私钥或临时导入密码。"""
    return subprocess.run(args, check=True, text=True, capture_output=True, **kwargs)


def identity() -> str:
    """显式环境配置优先；无证书时拒绝悄悄退回每版变化的临时签名。"""
    selected = os.environ.get("MACOS_SIGNING_IDENTITY", "").strip()
    if not selected and CONFIG.exists():
        selected = json.loads(CONFIG.read_text())["sha1"]
    if not selected or selected == "-":
        raise RuntimeError(
            "缺少固定签名证书。请设置 MACOS_SIGNING_IDENTITY，或经本人同意后运行 "
            "python3 scripts/macos_signing.py setup-local。临时签名不能保留跨构建的 TCC 授权。"
        )
    return selected


def setup_local() -> None:
    """一次性显式操作：仅用户域的 codeSign 信任，不改变 SSL/系统根信任。"""
    if CONFIG.exists():
        print("本项目已配置固定签名身份，保留现有证书；未重新生成。")
        return
    keychain = run("security", "default-keychain", "-d", "user").stdout.strip().strip('"')
    password = secrets.token_urlsafe(40)
    env = dict(os.environ, ALLINONE_P12_PASSWORD=password)
    name = f"YourAllInOne Local Development ({os.getuid()})"
    with tempfile.TemporaryDirectory(prefix="allinone-signing-") as directory:
        work = Path(directory)
        key, cert, bundle = (work / n for n in ("key.pem", "cert.pem", "identity.p12"))
        run(
            "openssl", "req", "-x509", "-newkey", "rsa:3072", "-sha256", "-days", "3650",
            "-subj", f"/CN={name}/", "-keyout", str(key), "-out", str(cert),
            "-passout", "env:ALLINONE_P12_PASSWORD",
            "-addext", "basicConstraints=critical,CA:FALSE",
            "-addext", "keyUsage=critical,digitalSignature",
            "-addext", "extendedKeyUsage=critical,codeSigning", env=env,
        )
        # 使用钥匙串兼容的 P12 算法；文件只在私有临时目录存在，口令随机且不持久化。
        run(
            "openssl", "pkcs12", "-export", "-inkey", str(key), "-in", str(cert),
            "-out", str(bundle), "-name", name, "-passin", "env:ALLINONE_P12_PASSWORD",
            "-passout", "env:ALLINONE_P12_PASSWORD", "-keypbe", "PBE-SHA1-3DES",
            "-certpbe", "PBE-SHA1-3DES", "-macalg", "sha1", env=env,
        )
        # 私钥导入为不可导出，仅授权系统 codesign 使用，禁止 -A 放开所有应用访问。
        run("security", "import", str(bundle), "-k", keychain, "-P", password,
            "-x", "-T", "/usr/bin/codesign")
        run("security", "add-trusted-cert", "-r", "trustRoot", "-p", "codeSign",
            "-k", keychain, str(cert))
        der = subprocess.run(["openssl", "x509", "-in", str(cert), "-outform", "DER"],
                             check=True, capture_output=True).stdout
        fingerprint = hashlib.sha1(der).hexdigest().upper()
        CONFIG.parent.mkdir(parents=True, exist_ok=True)
        # 此处仅写公开证书和标识，不保存私钥/P12/密码。
        (CONFIG.parent / "certificate.pem").write_bytes(cert.read_bytes())
        CONFIG.write_text(json.dumps({"name": name, "sha1": fingerprint,
                                      "scope": "local-user-code-signing-only"}, indent=2) + "\n")
    print(f"已建立本机固定签名身份：{name}；私钥保存在登录钥匙串。")


def sign(app: Path) -> None:
    """对最终完整 App 签名一次；不再先签裸可执行文件再改写其身份。"""
    selected = identity()
    run("codesign", "--force", "--sign", selected, str(app))
    run("codesign", "--verify", "--strict", "--verbose=2", str(app))
    details = run("codesign", "-d", "-r-", "--verbose=4", str(app))
    signature = details.stdout + details.stderr
    requirement = next((line for line in signature.splitlines() if "designated =>" in line), "")
    # identifier-only 或 cdhash-only 都不是本方案要求的证书身份。
    if "Signature=adhoc" in signature or not re.search(r"anchor|certificate", requirement):
        raise RuntimeError("App 未得到证书绑定的固定身份，拒绝作为权限修复构建交付。")
    info = plistlib.loads((app / "Contents/Info.plist").read_bytes())
    executable = app / "Contents/MacOS" / info["CFBundleExecutable"]
    manifest = {
        "bundle_id": info["CFBundleIdentifier"], "signing_identity": selected,
        "designated_requirement": requirement, "executable_sha256": hashlib.sha256(executable.read_bytes()).hexdigest(),
        "source_head": run("git", "-C", str(ROOT), "rev-parse", "HEAD").stdout.strip(),
        "source_dirty": bool(run("git", "-C", str(ROOT), "status", "--porcelain").stdout),
    }
    (app.parent / "macos-build.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"固定证书签名已验证：{info['CFBundleIdentifier']}\n{requirement}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["check", "setup-local", "sign"])
    parser.add_argument("app", nargs="?", type=Path)
    args = parser.parse_args()
    try:
        if args.action == "setup-local":
            setup_local()
        elif args.action == "check":
            selected = identity()
            valid = run("security", "find-identity", "-v", "-p", "codesigning").stdout
            if selected not in valid:
                raise RuntimeError("配置的签名身份不在可用证书中，请检查钥匙串证书、私钥与信任。")
            print("固定签名身份检查通过。")
        else:
            if args.app is None:
                parser.error("sign 需要指定 App 路径")
            sign(args.app.resolve())
    except (RuntimeError, subprocess.CalledProcessError) as error:
        # 不输出 security import 的参数，避免临时 P12 口令出现在日志中。
        detail = error.stderr.strip() if isinstance(error, subprocess.CalledProcessError) else str(error)
        raise SystemExit(detail or "签名操作失败，请检查钥匙串访问提示。") from None


if __name__ == "__main__":
    main()
