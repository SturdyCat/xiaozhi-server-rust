# Echo Dot 2 手动安装 emOS（不依赖官方控制器/docker）

来源核实：EchoMuse `emos/README.md`（Build and install / Flashing safely / Rollback 各节）。
运行期接入（设备 → 我们 server）由 `server/src/app/echomuse/` 承担，见 [echomuse.md](echomuse.md)；本文只覆盖硬件侧一次性安装。

## 总览

| 步骤 | 命令入口 | 说明 |
|---|---|---|
| 0. 解锁 | `firebreak`（一次性） | Dot 进 TWRP，adb 可见；本文不覆盖 |
| 1. 环境检查 | `tools/emos_install.py` prereq | adb + TWRP + 序列号 |
| 2. 读 boot 备份 | 同上 pull | **恢复镜像，10 秒救砖**，先验证后刷 |
| 3. 构建 | 同上 build | clone EchoMuse → checkout 最新 `emos-v*` → `./build.sh` |
| 4. 刷写+校验 | 同上 flash | busybox dd `conv=fsync` + drop_caches 后回读 md5 |
| 5. 重启确认 | 同上 boot | server `/api/logs` 出现 echomuse register 即接入 |

```sh
python3 tools/emos_install.py            # 全程/断点续跑
python3 tools/emos_install.py --status   # 查看进度
python3 tools/emos_install.py --reset flash   # 作废某步及之后，重跑
```

断点状态存 `<workdir>/state.json`（默认 `~/emos-install/`），每步成功才落盘；任何一步失败，修好后原命令重跑即从该步继续，之前步骤不重做。

## 手动等价命令（不用脚本）

```sh
# 1. 读 boot（TWRP 下 adb 即 root，无需 su）
adb exec-out dd if=/dev/block/mmcblk0p10 bs=4096 > boot_a_x.img
adb exec-out dd if=/dev/block/mmcblk0p10 bs=4096 | md5   # 与上一次哈希一致才可信

# 2. 构建（复用你自己 boot 的内核+DTB；emOS 无可分发镜像，必须从自己设备构建）
git clone https://github.com/wilbowes/EchoMuse.git && cd EchoMuse
git fetch --tags && git describe --tags --match 'emos-v*' origin/main  # 取最新 emos 标签
git checkout <emos-vX.Y> && ./build.sh ../boot_a_x.img ../emos-boot.img

# 3. 刷写（关键：fsync + drop_caches 后回读，否则页缓存假成功）
adb push emos-boot.img /tmp/
adb shell 'busybox dd if=/tmp/emos-boot.img of=/dev/block/mmcblk0p10 bs=2048 conv=fsync'
adb shell 'sync; echo 3 > /proc/sys/vm/drop_caches'
adb shell md5sum /dev/block/mmcblk0p10    # 必须等于本地 md5 -q emos-boot.img
adb reboot
```

## 硬性安全约束

- **无 `boot_a_x.img` 备份绝不刷写**（脚本 flash 步强制拦截）。恢复 = TWRP 中把它 dd 回 `mmcblk0p10`，约十秒。第一次刷之前先在脑内确认恢复路径走得通。
- Toolbox `dd` 拒绝 `conv=fsync`，必须 busybox（TWRP 自带）。
- 回读前必须 `echo 3 > drop_caches`，否则读到的是缓存里的新镜像。
- 写入瞬间断电 ⇒ 设备无法自恢复，必须 TWRP + 线缆救回；写入约 1 秒，风险窗口就这么大。
- amonet v2 设备 bootloader 只从 slot A 启动：emOS 写 A（p10），原厂镜像留在 B（p11）可作最后手段的源（恢复时 dd 回 A，不是切槽）。

## 刷完后：WiFi 与确认

- 设备 `/data` 若带 FireOS 时期的 `wpa_supplicant.conf` 会自动连；纯新设备无 conf，init 停在灯环第 11 位等待。
- 配网走串口控制台 `/dev/ttyGS0`：**必须 raw 模式**（默认 termios 有 ECHO，设备会执行自己的回显，命令全返回 127）。`stty -f <tty> raw -echo` 或 pyserial `ser.setraw()`，然后运行 `em-wifi` 选网输密码；WPA3/WEP 会被拒（FireOS 6 + emOS 驱动无 SAE），路由器用 WPA2。
- 确认接入：server 日志流 `ws /api/logs` 出现 echomuse register；或 `dns-sd -B _emcontroller._tcp` 看到广播。

## 日常恢复/再进 TWRP

- emOS 运行中：控制台敲 `/init recovery`（无需拆机断电按按钮）。
- 救砖：断电 → 按住 mute（或 vol+，视 amonet 版本）→ 上电直到灯环变化，进 TWRP 后 dd 回 `boot_a_x.img`。
- `/proc/last_kmsg` 保存上一次启动的内核日志，任何莫名重启先读它。
