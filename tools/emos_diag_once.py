"""一次性诊断：串口抓 emOS 设备侧 controller 连接日志。"""
import sys, os
sys.path.insert(0, os.path.join(os.path.dirname(__file__)))
from emos_install import Console, find_console_port

CMDS = [
    "cat /tmp/server.log",
    "tail -n 40 /run/net.log",
    "tail -n 20 /run/messages",
    "ls /data/emos/",
    "cat /data/emos/controller* 2>/dev/null; cat /data/emos/*.conf 2>/dev/null | head -40",
]

con = Console(find_console_port())
con.drain(1.0)
for c in CMDS:
    con.buf = ""
    con.send(c + "\n")
    out = con.drain(2.5)
    print(f"\n===== $ {c} =====")
    print(out)
con.close()
