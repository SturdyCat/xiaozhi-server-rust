#!/usr/bin/env python3
"""tuya_probe.py — 涂鸦云凭证/区域/授权诊断（零第三方依赖，仅 Python 标准库）。

用法：定位 `[tuya]` 云同步的鉴权/路径/订阅类失败。用同一套凭证对区域 host 发
token 请求，成功后拉一次授权设备清单，按涂鸦平台机制给出结论建议。
签名与 `server/src/device/tuya/cloud.rs` 同源（官方 v1.0 口径，2026-11-03 真凭证
联调验证）——本工具结果可直接对照服务端 `POST /api/tuya/cloud/proxy` 代理端点。

## 用法

```sh
# 从服务端配置库读凭证（推荐：secret 不落 shell 历史）
python3 tools/tuya_probe.py --db .dev/data.db

# 手动传凭证（注意会进 shell 历史）
python3 tools/tuya_probe.py --region cn --access-id ID --access-secret SEC
```

## 安全

- 绝不打印 access_secret 与 access_token 原值；输出只有区域、成功布尔、错误码与消息。
- 只读 SQLite（`SELECT doc FROM cfg_documents`），不写库。

## 结论判据（输出末尾的"结论"段）

- 1004 sign invalid：签名串构造错误（官方 v1.0 口径：`METHOD\\n` + sha256(body)小写
  + `\\n\\n`（空 Header 段）+ path?排序query，HMAC(secret, client_id+token+t+以上)
  大写 hex——两段换行缺一不可）或 secret 与 id 不配对；
- 1108 uri path invalid：路径在该区域网关不存在（如旧路径
  `…/actions/authorized-devices`，正确为 `/v1.0/iot-01/associated-users/devices`）；
- 28841003：API 组未订阅（IoT Core 连接服务覆盖设备详情/指令）；
- token 成功但设备清单 0 台：需在涂鸦平台项目完成「链接涂鸦 App 账号」（二维码授权）。
"""

from __future__ import annotations

import argparse
import hashlib
import hmac
import json
import sqlite3
import sys
import time
import urllib.request

HOSTS = {
    "cn": "https://openapi.tuyacn.com",
    "us": "https://openapi.tuyaus.com",
    "eu": "https://openapi.tuyaeu.com",
    "in": "https://openapi.tuyain.com",
}
PATH_TOKEN = "/v1.0/token"
PATH_DEVICES = "/v1.0/iot-01/associated-users/devices"


def official_sign(secret: str, access_id: str, token: str, t: str, method: str, path_query: str, body: str) -> str:
    """涂鸦官方 v1.0 签名（与 cloud.rs `official_sign` 同源）。

    str_to_sign = METHOD \\n sha256(body)小写 \\n \\n path?query
    sign = HMAC-SHA256(secret, client_id + access_token + t + str_to_sign) 大写 hex
    """
    content_sha = hashlib.sha256(body.encode()).hexdigest()  # 小写
    message = f"{access_id}{token}{t}{method}\n{content_sha}\n\n{path_query}"
    return hmac.new(secret.encode(), message.encode(), hashlib.sha256).hexdigest().upper()


def build_request(method: str, host: str, path_query: str, access_id: str, secret: str, token: str, body: str = "") -> urllib.request.Request:
    t = str(int(time.time() * 1000))
    headers = {
        "client_id": access_id,
        "access_token": token,
        "t": t,
        "sign_method": "HMAC-SHA256",
        "sign": official_sign(secret, access_id, token, t, method, path_query, body),
        "lang": "zh",
    }
    if body:
        headers["Content-Type"] = "application/json"
    return urllib.request.Request(
        f"{host}{path_query}",
        data=(body.encode() if body else None),
        headers=headers,
        method=method,
    )


def call(req: urllib.request.Request) -> tuple[int, dict | None]:
    try:
        with urllib.request.urlopen(req, timeout=10) as resp:
            body = resp.read().decode("utf-8", "replace")
            return resp.status, json.loads(body) if body else None
    except urllib.error.HTTPError as e:  # 4xx/5xx 也要看 body（涂鸦常在 body 里回 code/msg）
        body = e.read().decode("utf-8", "replace")
        try:
            return e.code, json.loads(body) if body else None
        except json.JSONDecodeError:
            return e.code, None
    except Exception as e:  # 网络/超时
        print(f"  请求失败（网络）: {e}")
        return 0, None


def try_token(region: str, access_id: str, secret: str) -> tuple[str, str | None]:
    """发 token 请求。返回 (描述, access_token 或 None)。"""
    status, body = call(build_request("GET", HOSTS[region], f"{PATH_TOKEN}?grant_type=1", access_id, secret, ""))
    code = (body or {}).get("code")
    msg = (body or {}).get("msg") or (body or {}).get("message") or ""
    ok = bool((body or {}).get("success"))
    print(f"  [{region}] HTTP {status} · success={ok} · code={code} · msg={msg[:120]}")
    if ok:
        result = (body or {}).get("result") or {}
        return f"token 获取成功（有效期 {result.get('expire_time')}s）", result.get("access_token") or None
    return f"code {code}: {msg}", None


def try_devices(region: str, access_id: str, secret: str, token: str) -> None:
    status, body = call(build_request("GET", HOSTS[region], f"{PATH_DEVICES}?size=50", access_id, secret, token))
    code = (body or {}).get("code")
    msg = (body or {}).get("msg") or (body or {}).get("message") or ""
    ok = bool((body or {}).get("success"))
    result = (body or {}).get("result")
    if isinstance(result, dict):
        devices = result.get("devices") or result.get("list") or []
    elif isinstance(result, list):
        devices = result
    else:
        devices = []
    print(f"  [{region}] 设备清单 HTTP {status} · success={ok} · code={code} · 台数={len(devices)} · msg={msg[:120]}")
    for d in devices[:3]:
        print(f"    · {d.get('id')} {d.get('name')!r} model={d.get('model')!r} online={d.get('online')}")
    if len(devices) > 3:
        print(f"    · … 共 {len(devices)} 台")


def load_from_db(db_path: str) -> tuple[str, str, str]:
    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    try:
        row = conn.execute("SELECT doc FROM cfg_documents WHERE id = 1").fetchone()
    finally:
        conn.close()
    if not row:
        sys.exit(f"配置库 {db_path} 里没有 cfg_documents（不是本项目的 data.db？）")
    tuya = (json.loads(row[0]) or {}).get("tuya") or {}
    return tuya.get("region", "cn"), tuya.get("access_id", ""), tuya.get("access_secret", "")


def main() -> None:
    ap = argparse.ArgumentParser(description="涂鸦云凭证/区域/授权诊断（不回显密钥）")
    ap.add_argument("--db", help="服务端配置库 data.db 路径（读 [tuya] 段凭证）")
    ap.add_argument("--region", help="只测该区域（默认四区域全测）", choices=sorted(HOSTS))
    ap.add_argument("--access-id")
    ap.add_argument("--access-secret")
    args = ap.parse_args()

    if args.db:
        region, access_id, secret = load_from_db(args.db)
        print(f"凭证来源：{args.db}（region={region}, access_id={access_id[:2]}****{access_id[-2:] if len(access_id) > 4 else '****'}）")
    elif args.access_id and args.access_secret:
        region, access_id, secret = args.region or "cn", args.access_id, args.access_secret
        print(f"凭证来源：命令行（access_id={access_id[:2]}****{access_id[-2:] if len(access_id) > 4 else '****'}）")
    else:
        sys.exit("请给 --db <data.db> 或 --access-id/--access-secret")

    if not access_id or not secret:
        sys.exit("凭证未配置（配置库 [tuya] 段为空）：先在管理台录入凭证再诊断")

    regions = [args.region] if args.region else sorted(HOSTS)
    print(f"\n== 对 {len(regions)} 个区域 host 发 token 请求（官方 v1.0 签名）==")
    winners: dict[str, str] = {}
    codes: dict[str, list[str]] = {}
    for r in regions:
        desc, token = try_token(r, access_id, secret)
        code = desc.split(":")[0]
        codes.setdefault(code, []).append(r)
        if token:
            winners[r] = token
            try_devices(r, access_id, secret, token)

    print("\n== 结论 ==")
    if winners:
        good = "、".join(winners)
        print(f"✔ 凭证在 {good} 区有效（官方 v1.0 签名通过）。")
        if good != region:
            print(f"→ 服务端当前 region={region}，与有效区域不一致：到管理台「涂鸦设备」把区域改成 {good} 后保存、立即同步。")
        else:
            print("→ 区域一致。若云同步仍失败，点「立即同步」看状态条的失败原因。")
        print("→ 若设备清单为 0 台：需在涂鸦 IoT 平台项目里完成「链接涂鸦 App 账号」（二维码授权），")
        print("   App 账号下的设备才会授权给云项目。")
        return
    for code, rs in codes.items():
        print(f"· {code} → {('、'.join(rs))}")
    if all("1004" in c for c in codes):
        print("✘ 全部 code 1004（sign invalid）：签名串构造或 secret 不配对。")
        print("→ ① 核对本工具签名为官方 v1.0 口径（METHOD\\n+sha256(body)小写+\\n\\n+path?query，缺空 Header 段必 1004）；")
        print("→ ② 在涂鸦云开发项目「API 概览」重置 Access Secret，复制新值重新录入；")
        print("→ ③ 确认 id/secret 来自同一项目（secret 只在创建/重置时完整显示一次）。")
    elif all("1108" in c for c in codes):
        print("✘ 全部 code 1108（uri path invalid）：路径在该区域网关不存在。")
        print("→ 核对路径拼写；设备清单正确路径为 /v1.0/iot-01/associated-users/devices。")
    else:
        print("→ 1106/28841003 = 该 API 组未订阅（项目 → 服务 API 开通）；其余码见涂鸦全局错误码文档。")


if __name__ == "__main__":
    main()
